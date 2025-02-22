use std::sync::Arc;

use tokio::io::{
	AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter,
};

use crate::{
	fluid::{self, Fluids},
	item::{self, Items},
	read_string, write_string, Frequency, GlobalObject,
};

const SAVE_DATA_FORMAT: i64 = 3;

pub(crate) async fn cli(go: Arc<GlobalObject>) {
	let mut stdout = BufReader::new(tokio::io::stdin()).lines();
	while let Ok(Some(text)) = stdout.next_line().await {
		match text.as_str() {
			"load" => println!("{:?}", load_file("save.dat.gz", &go).await),
			"save" => println!("{:?}", save_file("save.dat.gz", &go).await),
			"stop" => {
				println!("{:?}", save_file("save.dat.gz", &go).await);
				std::process::exit(0);
			}
			_ => {
				println!("Command Not Found");
			}
		}
	}
}
pub async fn save_file(path: &str, go: &GlobalObject) -> Result<(), tokio::io::Error> {
	let mut w = tokio::fs::File::create(path).await?;
	save(&mut w, go).await
}
pub async fn save<W: AsyncWrite + std::marker::Unpin>(
	w: &mut W,
	go: &GlobalObject,
) -> Result<(), tokio::io::Error> {
	use async_compression::tokio::write::GzipEncoder;
	let mut w = GzipEncoder::new(BufWriter::new(w));
	w.write_i64(SAVE_DATA_FORMAT).await?;
	{
		let fluid_buffers = go.fluid_buffers.read().await;
		w.write_i32(fluid_buffers.len() as i32).await?;
		for (freq, fluids) in fluid_buffers.iter() {
			write_string(&mut w, &freq.0).await?;
			let fluid_data = fluids.data.read().await;
			w.write_i32(fluid_data.len() as i32).await?;
			for fs in fluid_data.values() {
				fs.write(&mut w).await?;
			}
		}
	}
	{
		let item_buffers = go.item_buffers.read().await;
		w.write_i32(item_buffers.len() as i32).await?;
		for (freq, items) in item_buffers.iter() {
			write_string(&mut w, &freq.0).await?;
			let items = items.data.read().await;
			w.write_i32(items.len() as i32).await?;
			for is in items.iter() {
				is.write(&mut w).await?;
			}
			for is in items.iter() {
				is.write_extra(&mut w).await?;
			}
		}
	}
	{
		let energy_buffers = go.energy_buffers.read().await;
		w.write_i32(energy_buffers.len() as i32).await?;
		for (freq, value) in energy_buffers.iter() {
			write_string(&mut w, &freq.0).await?;
			w.write_i64(*value).await?;
		}
	}
	w.shutdown().await?;
	w.into_inner();
	Ok(())
}
pub async fn load_file(path: &str, go: &GlobalObject) -> Result<(), tokio::io::Error> {
	let mut r = tokio::fs::File::open(path).await?;
	load(&mut r, go).await
}
pub async fn load<R: AsyncRead + std::marker::Unpin>(
	r: &mut R,
	go: &GlobalObject,
) -> Result<(), tokio::io::Error> {
	use async_compression::tokio::bufread::GzipDecoder;
	let mut r = GzipDecoder::new(BufReader::new(r));
	let version = r.read_i64().await?;
	if version == 2 {
		//V2はそのままV3デコーダで読み込める
	} else if version != SAVE_DATA_FORMAT {
		return Err(tokio::io::Error::other("Bad Data Format Version"));
	}
	{
		let mut fluid_buffers = go.fluid_buffers.write().await;
		let fluid_freq_count = r.read_i32().await?;
		for _ in 0..fluid_freq_count {
			let freq = read_string(&mut r).await?;
			let fluids = Fluids::new();
			let stack_count = r.read_i32().await?;
			for _ in 0..stack_count {
				let fs = fluid::FluidStack::read(&mut r).await?;
				fluids.insert_fluid(fs).await;
			}
			let freq = Frequency(freq);
			if let Some(old) = fluid_buffers.remove(&freq) {
				for fs in old.data.read().await.values() {
					fluids.insert_fluid(fs.clone()).await;
				}
			}
			fluid_buffers.insert(freq, Arc::new(fluids));
		}
	}
	{
		let mut item_buffers = go.item_buffers.write().await;
		let item_freq_count = r.read_i32().await?;
		for _ in 0..item_freq_count {
			let freq = read_string(&mut r).await?;
			let freq = Frequency(freq);
			let items = Items::new();
			{
				let mut item_data = items.data.write().await;
				let stack_count = r.read_i32().await?;
				let mut read_buffer = Vec::new();
				for _ in 0..stack_count {
					let is = item::ItemStack::read(&mut r).await?;
					read_buffer.push(is);
				}
				for is in read_buffer.iter_mut() {
					is.read_extra(&mut r).await?;
				}
				item_data.reserve(read_buffer.len());
				for is in read_buffer {
					item_data.push(is);
				}
				if let Some(old) = item_buffers.remove(&freq) {
					let old_items = old.data.read().await;
					for is in old_items.iter() {
						item_data.push(is.clone());
					}
				}
			}
			item_buffers.insert(freq, Arc::new(items));
		}
	}
	{
		let mut energy_buffers = go.energy_buffers.write().await;
		let energy_freq_count = r.read_i32().await?;
		for _ in 0..energy_freq_count {
			let freq = read_string(&mut r).await?;
			let freq = Frequency(freq);
			let mut value = r.read_i64().await?;
			if let Some(old) = energy_buffers.get(&freq) {
				value = value.saturating_add(*old);
			}
			energy_buffers.insert(freq, value);
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {

	use crate::{cli::load, GlobalObject};

	use super::save;

	#[test]
	fn save_load() {
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let src = GlobalObject::dummy().await;
				let mut v = Vec::new();
				save(&mut v, &src).await.unwrap();
				let mut dst = GlobalObject::new();
				load(&mut std::io::Cursor::new(&v), &mut dst).await.unwrap();
				assert_eq!(
					src.energy_buffers.read().await.iter().collect::<Vec<_>>(),
					dst.energy_buffers.read().await.iter().collect::<Vec<_>>()
				);

				let r = src.item_buffers.read().await;
				let mut src_items = futures::future::join_all({
					r.iter()
						.map(|(f, items)| async { (f.clone(), items.to_vec().await) })
				})
				.await
				.into_iter()
				.collect::<Vec<_>>();
				src_items.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0));
				let r = dst.item_buffers.read().await;
				let mut dst_items = futures::future::join_all({
					r.iter()
						.map(|(f, items)| async { (f.clone(), items.to_vec().await) })
				})
				.await
				.into_iter()
				.collect::<Vec<_>>();
				dst_items.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0));
				assert_eq!(src_items, dst_items);

				let r = src.fluid_buffers.read().await;
				let mut src_fluids = futures::future::join_all({
					r.iter()
						.map(|(f, fluids)| async { (f.clone(), fluids.to_vec().await) })
				})
				.await
				.into_iter()
				.collect::<Vec<_>>();
				src_fluids.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0));
				let r = dst.fluid_buffers.read().await;
				let mut dst_fluids = futures::future::join_all({
					r.iter()
						.map(|(f, fluids)| async { (f.clone(), fluids.to_vec().await) })
				})
				.await
				.into_iter()
				.collect::<Vec<_>>();
				dst_fluids.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0));
				assert_eq!(src_fluids, dst_fluids);
			});
	}
}
