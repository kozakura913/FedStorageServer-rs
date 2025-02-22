use std::sync::Arc;

use tokio::{
	io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
	sync::RwLock,
};

use crate::{client::ClientSession, read_string, to_hex_string, write_string};

const ITEM_BUFFER_LIMIT: usize = 100;

#[derive(Clone, Debug)]
pub struct Items {
	pub(crate) data: Arc<RwLock<Vec<ItemStack>>>,
}
impl Items {
	pub(crate) fn new() -> Self {
		Self {
			data: Arc::new(RwLock::new(Vec::new())),
		}
	}
	pub async fn take_items(&self, max_stacks: i32) -> Vec<ItemStack> {
		let mut data = self.data.write().await;
		let max_stacks = data.len().min(max_stacks.max(0) as usize);
		let stacks = data.drain(0..max_stacks);
		stacks.collect()
	}
	pub async fn insert_items(&self, stacks: &mut Vec<ItemStack>) {
		let mut data = self.data.write().await;
		let max_stacks = stacks
			.len()
			.min(ITEM_BUFFER_LIMIT.saturating_sub(data.len()));
		let stacks = stacks.drain(0..max_stacks);
		data.append(&mut stacks.collect());
	}
	pub async fn len(&self) -> usize {
		self.data.read().await.len()
	}
}
#[derive(Clone, Debug, Hash, Eq, PartialEq, PartialOrd)]
pub struct ItemStack {
	pub(crate) damage: i32,
	pub(crate) count: i32,
	pub(crate) id: String,
	pub(crate) nbt: Option<NBT>,
}
#[derive(Clone, Debug, Hash, Eq, PartialEq, PartialOrd)]
pub enum NBT {
	Raw(Vec<u8>),
	Extra(Option<GzipNBT>),
}
impl NBT {
	pub fn hint(&self) -> String {
		match self {
			NBT::Raw(raw) => to_hex_string(&raw),
			NBT::Extra(Some(gz)) => {
				use md5::Digest;
				let mut hasher = md5::Md5::new();
				hasher.update(gz.as_gzip());
				let result = hasher.finalize();
				to_hex_string(&result)
			}
			_ => "".into(),
		}
	}
}
impl ItemStack {
	pub async fn read<R: AsyncRead + std::marker::Unpin>(
		r: &mut R,
	) -> Result<Self, tokio::io::Error> {
		let id = read_string(r).await?; //アイテムID
		let damage = r.read_i32().await?; //ダメージ値
		let count = r.read_i32().await?; //スタックサイズ
		let nbt_size = r.read_i16().await?;
		let nbt = if nbt_size > 0 {
			let mut v = vec![0u8; nbt_size as usize];
			r.read_exact(&mut v).await?;
			Some(NBT::Raw(v))
		} else if nbt_size == -1 {
			Some(NBT::Extra(None))
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
	pub async fn read_extra<R: AsyncRead + std::marker::Unpin>(
		&mut self,
		r: &mut R,
	) -> Result<(), tokio::io::Error> {
		if let Some(NBT::Extra(nbt)) = &mut self.nbt {
			let len = r.read_i32().await?;
			let mut data = vec![0u8; len as usize];
			r.read_exact(&mut data).await?;
			*nbt = Some(GzipNBT::from_gzip(data));
		}
		Ok(())
	}
	pub async fn write<W: AsyncWrite + std::marker::Unpin>(
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
			Some(NBT::Raw(nbt)) => {
				let len = nbt
					.len()
					.try_into()
					.map_err(|e| tokio::io::Error::other(e))?;
				w.write_i16(len).await?;
				w.write_all(&nbt).await?;
			}
			Some(NBT::Extra(_)) => {
				w.write_i16(-1).await?; //遅延書き込みフラグ
			}
		}
		Ok(())
	}
	pub async fn write_extra<W: AsyncWrite + std::marker::Unpin>(
		&self,
		w: &mut W,
	) -> Result<(), tokio::io::Error> {
		match self.nbt.as_ref() {
			Some(NBT::Extra(Some(gz))) => {
				w.write_i32(gz.as_gzip().len() as i32).await?;
				w.write_all(gz.as_gzip()).await?;
			}
			_ => {}
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
		for is in insert_items.iter_mut() {
			is.read_extra(&mut self.reader).await?;
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
		for item in &items {
			item.write(&mut write_buffer).await?;
		}
		write_buffer.shutdown().await?;
		let compressed_bytes = write_buffer.into_inner();
		self.writer.write_i32(compressed_bytes.len() as i32).await?;
		self.writer.write_all(&compressed_bytes).await?;
		for item in &items {
			item.write_extra(&mut self.writer).await?;
		}
		Ok(())
	}
}
#[derive(Clone, Debug, Hash, Eq, PartialEq, PartialOrd)]
pub struct GzipNBT {
	data: Vec<u8>,
}
impl GzipNBT {
	pub fn from_gzip(data: Vec<u8>) -> Self {
		Self { data }
	}
	pub fn as_gzip(&self) -> &[u8] {
		&self.data
	}
}

#[cfg(test)]
mod tests {
	use tokio::io::{AsyncReadExt, AsyncWriteExt};

	use super::{GzipNBT, ItemStack, Items, ITEM_BUFFER_LIMIT, NBT};

	impl GzipNBT {
		pub async fn from_raw(raw: &[u8]) -> Result<Self, tokio::io::Error> {
			let mut write_buffer = async_compression::tokio::write::GzipEncoder::new(Vec::new());
			write_buffer.write(raw).await?;
			write_buffer.shutdown().await?;
			let compressed_bytes = write_buffer.into_inner();
			Ok(Self {
				data: compressed_bytes,
			})
		}
		#[allow(dead_code)]
		pub async fn to_raw(&self) -> Result<Vec<u8>, tokio::io::Error> {
			let mut gz_data = std::io::Cursor::new(&self.data);
			let mut reader = async_compression::tokio::bufread::GzipDecoder::new(&mut gz_data);
			let mut raw = Vec::new();
			reader.read_to_end(&mut raw).await?;
			Ok(raw)
		}
	}
	impl Items {
		pub async fn to_vec(&self) -> Vec<ItemStack> {
			let mut items = Vec::new();
			for is in self.data.read().await.iter() {
				items.push(is.clone());
			}
			items
		}
	}
	impl ItemStack {
		pub fn dummy() -> Self {
			Self {
				id: "minecraft:stone".into(),
				damage: 0,
				count: 64,
				nbt: Some(NBT::Raw(vec![0, 1, 2, 3])),
			}
		}
		pub async fn heavy_dummy() -> Self {
			let mut nbt = Vec::new();
			for _ in 0..u8::MAX {
				for b in 0..u8::MAX {
					nbt.push(b);
				}
			}
			Self {
				id: "minecraft:stone".into(),
				damage: 0,
				count: 64,
				nbt: Some(NBT::Extra(Some(GzipNBT::from_raw(&nbt).await.unwrap()))),
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
				src.write_extra(&mut v).await.unwrap();
				let r = &mut std::io::Cursor::new(&v);
				let mut dst = ItemStack::read(r).await.unwrap();
				dst.read_extra(r).await.unwrap();
				assert_eq!(src, dst);
			});
	}
	#[test]
	fn read_write_heavy_item() {
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let src = ItemStack::heavy_dummy().await;
				let mut v = Vec::new();
				src.write(&mut v).await.unwrap();
				src.write_extra(&mut v).await.unwrap();
				let r = &mut std::io::Cursor::new(&v);
				let mut dst = ItemStack::read(r).await.unwrap();
				dst.read_extra(r).await.unwrap();
				assert_eq!(src, dst);
			});
	}
}
