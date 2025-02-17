use std::sync::Arc;

use tokio::{
	io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
	sync::RwLock,
};

use crate::{client::ClientSession, read_string, write_string};

const ITEM_BUFFER_LIMIT: usize = 100;
#[derive(Clone, Debug)]
pub struct Items {
	data: Arc<RwLock<Vec<ItemStack>>>,
}
impl Items {
	pub(crate) fn new() -> Self {
		Self {
			data: Arc::new(RwLock::new(Vec::new())),
		}
	}
	async fn take_items(&self, max_stacks: i32) -> Vec<ItemStack> {
		let mut data = self.data.write().await;
		let max_stacks = data.len().min(max_stacks.max(0) as usize);
		let stacks = data.drain(0..max_stacks);
		stacks.collect()
	}
	async fn insert_items(&self, stacks: &mut Vec<ItemStack>) {
		let mut data = self.data.write().await;
		let max_stacks = stacks
			.len()
			.min(ITEM_BUFFER_LIMIT.saturating_sub(data.len()));
		let stacks = stacks.drain(0..max_stacks);
		data.append(&mut stacks.collect());
	}
}
#[derive(Clone, Debug, Hash, Eq, PartialEq, PartialOrd)]
pub struct ItemStack {
	damage: i32,
	count: i32,
	id: String,
	nbt: Option<Vec<u8>>,
}
impl ItemStack {
	async fn read<R: AsyncRead + std::marker::Unpin>(r: &mut R) -> Result<Self, tokio::io::Error> {
		let id = read_string(r).await?; //アイテムID
		let damage = r.read_i32().await?; //ダメージ値
		let count = r.read_i32().await?; //スタックサイズ
		let nbt_size = r.read_i16().await?;
		let nbt = if nbt_size > 0 {
			let mut v = vec![0u8; nbt_size as usize];
			r.read_exact(&mut v).await?;
			Some(v)
		} else {
			None
		};
		Ok(Self {
			damage,
			count,
			id,
			nbt,
		})
	}
	async fn write<W: AsyncWrite + std::marker::Unpin>(
		&self,
		w: &mut W,
	) -> Result<(), tokio::io::Error> {
		write_string(w, &self.id).await?; //アイテムID
		w.write_i32(self.damage).await?; //ダメージ値
		w.write_i32(self.count).await?; //スタックサイズ
		match self.nbt.as_ref() {
			None => {
				w.write_i16(0).await?;
			}
			Some(nbt) => {
				let len = nbt
					.len()
					.try_into()
					.map_err(|e| tokio::io::Error::other(e))?;
				w.write_i16(len).await?;
				w.write_all(&nbt).await?;
			}
		}
		Ok(())
	}
}

impl ClientSession {
	pub(crate) async fn item_recv(&mut self) -> Result<(), tokio::io::Error> {
		let data_size = self.reader.read_i32().await?;
		let data_size = data_size
			.try_into()
			.map_err(|e| tokio::io::Error::other(e))?;
		let mut raw_data = vec![0u8; data_size];
		self.reader.read_exact(&mut raw_data).await?;
		let mut raw_data = std::io::Cursor::new(&raw_data);
		let mut reader = async_compression::tokio::bufread::GzipDecoder::new(&mut raw_data);
		let item_count = reader.read_i32().await?;
		let mut insert_items = Vec::new();
		for _ in 0..item_count {
			let is = ItemStack::read(&mut reader).await?;
			insert_items.push(is);
		}
		let cache_hit = { self.go.item_buffers.read().await.get(self.freq()).cloned() };
		let freq_buffer = if let Some(cache_hit) = cache_hit {
			cache_hit
		} else {
			let mut item_buffers = self.go.item_buffers.write().await;
			let freq_buffer = item_buffers.get(self.freq()).cloned();
			match freq_buffer {
				Some(v) => v,
				None => {
					let v = Arc::new(Items::new());
					item_buffers.insert(self.freq().clone(), v.clone());
					v
				}
			}
		};
		freq_buffer.insert_items(&mut insert_items).await;
		let reject_start = item_count - insert_items.len() as i32;
		let mut write_buffer = async_compression::tokio::write::GzipEncoder::new(Vec::new());
		write_buffer.write_i32(item_count - reject_start).await?;
		for i in reject_start..item_count {
			write_buffer.write_i32(i).await?;
		}
		write_buffer.shutdown().await?;
		let compressed_bytes = write_buffer.into_inner();
		self.writer.write_i32(compressed_bytes.len() as i32).await?;
		self.writer.write_all(&compressed_bytes).await?;
		Ok(())
	}
	pub(crate) async fn item_send(&mut self) -> Result<(), tokio::io::Error> {
		let max_stacks = self.reader.read_i32().await?;
		let freq_buffer = self.go.item_buffers.read().await.get(self.freq()).cloned();
		let freq_buffer = if let Some(b) = freq_buffer {
			b
		} else {
			self.writer.write_i32(0).await?;
			return Ok(());
		};
		let items = freq_buffer.take_items(max_stacks).await;
		let mut write_buffer = async_compression::tokio::write::GzipEncoder::new(Vec::new());
		write_buffer.write_i32(items.len() as i32).await?;
		for item in items {
			item.write(&mut write_buffer).await?;
		}
		write_buffer.shutdown().await?;
		let compressed_bytes = write_buffer.into_inner();
		self.writer.write_i32(compressed_bytes.len() as i32).await?;
		self.writer.write_all(&compressed_bytes).await?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::{ItemStack, Items, ITEM_BUFFER_LIMIT};

	impl ItemStack {
		fn dummy() -> Self {
			Self {
				id: "minecraft:stone".into(),
				damage: 0,
				count: 64,
				nbt: Some(vec![0, 1, 2, 3]),
			}
		}
	}
	#[test]
	fn update_items() {
		let is = ItemStack::dummy();
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let items = Items::new();
				let mut add_stacks = Vec::new();
				for _ in 0..5 {
					add_stacks.push(is.clone());
				}
				items.insert_items(&mut add_stacks).await;
				assert_eq!(add_stacks.len(), 0);
				assert_eq!(items.data.read().await.len(), 5);
				for _ in 0..ITEM_BUFFER_LIMIT {
					add_stacks.push(is.clone());
				}
				items.insert_items(&mut add_stacks).await;
				assert_eq!(add_stacks.len(), 5);
				assert_eq!(items.data.read().await.len(), ITEM_BUFFER_LIMIT);
				let take_items = items.take_items(5).await;
				assert_eq!(take_items.len(), 5);
				assert_eq!(items.data.read().await.len(), ITEM_BUFFER_LIMIT - 5);
				let old = items.data.read().await.clone();
				let take_items = items.take_items(ITEM_BUFFER_LIMIT as i32).await;
				assert_eq!(take_items.len(), ITEM_BUFFER_LIMIT - 5);
				assert_eq!(items.data.read().await.len(), 0);
				assert_eq!(old, take_items);
			});
	}
	#[test]
	fn read_write_item() {
		let src = ItemStack::dummy();
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let mut v = Vec::new();
				src.write(&mut v).await.unwrap();
				let dst = ItemStack::read(&mut std::io::Cursor::new(&v))
					.await
					.unwrap();
				assert_eq!(src, dst);
			});
	}
}
