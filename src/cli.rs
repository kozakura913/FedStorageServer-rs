use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};

use crate::{
	fluid::{self, Fluids},
	item::{self, Items},
	read_string, write_string, Frequency, GlobalObject,
};

const SAVE_DATA_FORMAT: i64 = 2;

pub(crate) async fn cli(go: Arc<GlobalObject>) {
	let mut stdout = BufReader::new(tokio::io::stdin()).lines();
	while let Ok(Some(text)) = stdout.next_line().await {
		match text.as_str() {
			"load" => println!("{:?}", load(&go).await),
			"save" => println!("{:?}", save(&go).await),
			"stop" => {
				println!("{:?}", save(&go).await);
				std::process::exit(0);
			}
			_ => {
				println!("Command Not Found");
			}
		}
	}
}
pub async fn save(go: &GlobalObject) -> Result<(), tokio::io::Error> {
	use async_compression::tokio::write::GzipEncoder;
	let w = tokio::fs::File::create("save.dat.gz").await?;
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
pub async fn load(go: &GlobalObject) -> Result<(), tokio::io::Error> {
	use async_compression::tokio::bufread::GzipDecoder;
	let r = tokio::fs::File::open("save.dat.gz").await?;
	let mut r = GzipDecoder::new(BufReader::new(r));
	let version = r.read_i64().await?;
	if version != SAVE_DATA_FORMAT {
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
				for _ in 0..stack_count {
					let is = item::ItemStack::read(&mut r).await?;
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
