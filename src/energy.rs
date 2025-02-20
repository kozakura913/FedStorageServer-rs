use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::client::ClientSession;

const ENERGY_BUFFER_LIMIT: i64 = u32::MAX as i64;

impl ClientSession {
	pub(crate) async fn energy_recv(&mut self) -> Result<(), tokio::io::Error> {
		let raw_recv = self.reader.read_i64().await?;
		let reject = {
			let mut lock = self.go.energy_buffers.write().await;
			let old_energy = lock.remove(self.freq());
			let old_energy = old_energy.unwrap_or(0);
			let target_recv = 0.max(ENERGY_BUFFER_LIMIT - old_energy).min(raw_recv);
			let reject = 0.max(raw_recv - target_recv);
			let new_energy = old_energy + target_recv;
			lock.insert(self.freq().clone(), new_energy);
			reject
		};
		self.writer.write_i64(reject).await?;
		Ok(())
	}
	pub(crate) async fn energy_send(&mut self) -> Result<(), tokio::io::Error> {
		let max_send = self.reader.read_i64().await?;
		let max_send = max_send.max(0);
		let send = {
			let mut lock = self.go.energy_buffers.write().await;
			let old_energy = lock.remove(self.freq());
			let old_energy = old_energy.unwrap_or(0);
			let target_send = max_send.min(old_energy);
			let new_energy = old_energy - target_send;
			if new_energy > 0 {
				lock.insert(self.freq().clone(), new_energy);
			}
			target_send
		};
		self.writer.write_i64(send).await?;
		Ok(())
	}
}
