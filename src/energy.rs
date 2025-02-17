use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::client::ClientSession;

const ENERGY_BUFFER_LIMIT: i64 = u32::MAX as i64;

impl ClientSession {
	pub(crate) async fn energy_recv(&mut self) -> Result<(), tokio::io::Error> {
		let energy = self.reader.read_i64().await?;
		let reject = {
			let mut lock = self.go.energy_buffers.write().await;
			let v = lock.remove(self.freq());
			let v = v.unwrap_or(0);
			let available = 0.max(ENERGY_BUFFER_LIMIT - energy);
			let reject = 0.max(v - available);
			let energy = energy + (v - reject);
			lock.insert(self.freq().clone(), energy);
			reject
		};
		self.writer.write_i64(reject).await?;
		Ok(())
	}
	pub(crate) async fn energy_send(&mut self) -> Result<(), tokio::io::Error> {
		let max_recv = self.reader.read_i64().await?;
		let max_recv = max_recv.max(0);
		let send = {
			let mut lock = self.go.energy_buffers.write().await;
			let v = lock.remove(self.freq());
			let v = v.unwrap_or(0);
			let send = max_recv.min(v);
			let v = v - send;
			if v > 0 {
				lock.insert(self.freq().clone(), v);
			}
			send
		};
		self.writer.write_i64(send).await?;
		Ok(())
	}
}
