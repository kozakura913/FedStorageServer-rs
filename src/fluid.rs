use std::{collections::HashMap, sync::Arc};

use tokio::{
	io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
	sync::RwLock,
};

use crate::{read_string, write_string, ClientSession};

//const FLUID_BUFFER_LIMIT:i64=i32::MAX as i64;//reject機能実装する時に使う
#[derive(Clone, Debug)]
pub struct Fluids {
	data: Arc<RwLock<HashMap<FluidId, FluidStack>>>,
}
impl Fluids {
	pub(crate) fn new() -> Self {
		Self {
			data: Arc::new(RwLock::new(HashMap::new())),
		}
	}
	async fn take_fluid(&self, mut max_stack: FluidStack) -> Option<FluidStack> {
		let mut data = self.data.write().await;
		let store = if max_stack.name.is_empty() {
			let fs = (data.values_mut().next()?).clone();
			let count = max_stack.count;
			max_stack = fs; //量以外の情報を搬出対象に
			max_stack.count = count;
			data.get_mut(&max_stack.id)?
		} else {
			data.get_mut(&max_stack.id)?
		};
		let target_count = max_stack.count.min(store.count);
		if target_count <= 0 {
			None
		} else {
			store.count -= target_count;
			max_stack.count = target_count;
			let _ = store;
			if store.count < 1 {
				data.remove(&max_stack.id);
			}
			Some(max_stack)
		}
	}
	async fn insert_fluid(&self, mut stack: FluidStack) {
		let mut data = self.data.write().await;
		if let Some(fluid) = data.remove(&stack.id) {
			stack.count = stack.count.saturating_add(fluid.count);
		}
		data.insert(stack.id.clone(), stack);
	}
}
#[derive(Clone, Debug, Hash, Eq, PartialEq, PartialOrd)]
struct FluidId(String);
impl FluidId {
	fn new(mut name: String, nbt: Option<impl AsRef<[u8]>>) -> Self {
		if let Some(nbt) = nbt {
			use md5::Digest;
			let mut hasher = md5::Md5::new();
			hasher.update(nbt);
			let result = hasher.finalize();
			let result = result
				.iter()
				.map(|n| format!("{:02X}", n))
				.collect::<String>();
			name += &result;
		}
		Self(name)
	}
}
#[derive(Clone, Debug, Hash, Eq, PartialEq, PartialOrd)]
struct FluidStack {
	id: FluidId, //name+nbt_hash
	name: String,
	count: i64,
	nbt: Option<Vec<u8>>,
}
impl FluidStack {
	async fn read<R: AsyncRead + std::marker::Unpin>(r: &mut R) -> Result<Self, tokio::io::Error> {
		let name = read_string(r).await?; //液体名
		let count = r.read_i64().await?; //スタックサイズ
		let nbt_size = r.read_i16().await?;
		let nbt = if nbt_size > 0 {
			let mut v = vec![0u8; nbt_size as usize];
			r.read_exact(&mut v).await?;
			Some(v)
		} else {
			None
		};
		Ok(Self {
			id: FluidId::new(name.clone(), nbt.as_ref()),
			name,
			count,
			nbt,
		})
	}
	async fn write<W: AsyncWrite + std::marker::Unpin>(
		&self,
		w: &mut W,
	) -> Result<(), tokio::io::Error> {
		write_string(w, &self.name).await?; //液体名
		w.write_i64(self.count).await?; //スタックサイズ
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
	pub(crate) async fn fluid_recv(&mut self) -> Result<(), tokio::io::Error> {
		let fs = FluidStack::read(&mut self.reader).await?;
		let cache_hit = { self.go.fluid_buffers.read().await.get(self.freq()).cloned() };
		let freq_buffer = if let Some(cache_hit) = cache_hit {
			cache_hit
		} else {
			let mut fluid_buffers = self.go.fluid_buffers.write().await;
			let freq_buffer = fluid_buffers.get(self.freq()).cloned();
			match freq_buffer {
				Some(v) => v,
				None => {
					let v = Arc::new(Fluids::new());
					fluid_buffers.insert(self.freq().clone(), v.clone());
					v
				}
			}
		};
		freq_buffer.insert_fluid(fs).await;
		Ok(())
	}
	pub(crate) async fn fluid_send(&mut self) -> Result<(), tokio::io::Error> {
		let fs = FluidStack::read(&mut self.reader).await?;
		let freq_buffer =
			if let Some(cache_hit) = self.go.fluid_buffers.read().await.get(self.freq()) {
				cache_hit.clone()
			} else {
				self.writer.write_i32(0).await?;
				return Ok(());
			};
		if let Some(fs) = freq_buffer.take_fluid(fs).await {
			let mut write_buffer = Vec::new();
			fs.write(&mut write_buffer).await?;
			self.writer.write_i32(write_buffer.len() as i32).await?;
			self.writer.write_all(&write_buffer).await?;
		} else {
			self.writer.write_i32(0).await?;
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::{FluidStack, Fluids};

	impl FluidStack {
		fn dummy() -> Self {
			let nbt = Some(vec![0, 1, 2, 3]);
			let name = "water".to_string();
			Self {
				id: super::FluidId::new(name.clone(), nbt.as_ref()),
				name,
				count: i32::MAX as i64 + 100,
				nbt,
			}
		}
		fn any() -> Self {
			let nbt = None;
			let name = "".to_string();
			Self {
				id: super::FluidId::new(name.clone(), nbt.as_ref()),
				name,
				count: i32::MAX as i64 + 100,
				nbt,
			}
		}
	}

	#[test]
	fn update_fluids() {
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let fluids = Fluids::new();
				fluids.insert_fluid(FluidStack::dummy()).await;
				assert_eq!(fluids.data.read().await.len(), 1);
				fluids.insert_fluid(FluidStack::dummy()).await;
				assert_eq!(fluids.data.read().await.len(), 1); //液体はスタックされる
				assert_eq!(
					fluids.take_fluid(FluidStack::dummy()).await,
					Some(FluidStack::dummy())
				);
				assert_eq!(
					fluids.data.read().await.values().next().cloned(),
					Some(FluidStack::dummy())
				);
				assert_eq!(
					fluids.take_fluid(FluidStack::any()).await,
					Some(FluidStack::dummy())
				);
				fluids.insert_fluid(FluidStack::dummy()).await;
				fluids.insert_fluid(FluidStack::dummy()).await;
				assert_ne!(
					fluids.data.read().await.values().next().cloned(),
					Some(FluidStack::dummy())
				);
			});
	}
	#[test]
	fn read_write_fluid() {
		let src = FluidStack::dummy();
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let mut v = Vec::new();
				src.write(&mut v).await.unwrap();
				let dst = FluidStack::read(&mut std::io::Cursor::new(&v))
					.await
					.unwrap();
				assert_eq!(src, dst);
			});
	}
}
