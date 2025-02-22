use std::{net::SocketAddr, sync::Arc};

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	net::TcpStream,
	sync::Mutex,
};

use crate::{read_string, Frequency, GlobalObject};

const CLIENT_VERSION: i64 = 7;

pub(crate) struct ClientSession {
	pub(crate) reader: tokio::net::tcp::OwnedReadHalf,
	pub(crate) writer: tokio::net::tcp::OwnedWriteHalf,
	pack_start: chrono::DateTime<chrono::Utc>,
	freq: Option<Frequency>,
	pub(crate) meta: Arc<Mutex<ClientMeta>>,
	pub(crate) go: Arc<GlobalObject>,
}
pub struct ClientMeta {
	pub(crate) id: uuid::Uuid,
	pub(crate) addr: std::net::SocketAddr,
	pub hostname: String,
	pub last_sync_time: i64,
}

#[derive(FromPrimitive, ToPrimitive, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(i8)]
enum Command {
	NOP = -1,
	SetFrequency = 1,
	ItemFromClient = 2,
	ItemToClient = 3,
	FluidFromClient = 4,
	FluidToClient = 5,
	EnergyFromClient = 6,
	EnergyToClient = 7,
	SetHostName = 8,
	PackStart = 9,
	PackEnd = 10,
}

impl ClientSession {
	pub fn new(soc: TcpStream, addr: SocketAddr, go: Arc<GlobalObject>) -> Self {
		let (reader, writer) = soc.into_split();
		let meta = Arc::new(Mutex::new(ClientMeta {
			id: uuid::Uuid::new_v4(),
			addr,
			hostname: "DefaultHostName".into(),
			last_sync_time: 0,
		}));
		ClientSession {
			reader,
			writer,
			pack_start: chrono::Utc::now(),
			freq: None,
			meta,
			go,
		}
	}
	pub async fn session(mut self) -> Result<(), tokio::io::Error> {
		self.writer.write_i64(CLIENT_VERSION).await?;
		{
			let mut clients = self.go.clients.write().await;
			let id = self.meta.lock().await.id;
			clients.insert(id, self.meta.clone());
			println!(
				"start session remote address {}",
				self.meta.lock().await.addr
			);
		}
		loop {
			let command = self.reader.read_i8().await?;
			match Command::from_i8(command) {
				Some(Command::NOP) => {
					//NOP
				}
				Some(Command::SetHostName) => {
					let mut meta = self.meta.lock().await;
					meta.hostname = read_string(&mut self.reader).await?;
				}
				Some(Command::PackStart) => {
					self.pack_start = chrono::Utc::now();
				}
				Some(Command::PackEnd) => {
					let mut meta = self.meta.lock().await;
					meta.last_sync_time = (chrono::Utc::now() - self.pack_start).num_milliseconds();
				}
				Some(Command::SetFrequency) => {
					self.freq = Some(Frequency(read_string(&mut self.reader).await?));
				}
				Some(Command::EnergyToClient) => {
					self.energy_send().await?;
				}
				Some(Command::EnergyFromClient) => {
					self.energy_recv().await?;
				}
				Some(Command::ItemToClient) => {
					self.item_send().await?;
				}
				Some(Command::ItemFromClient) => {
					self.item_recv().await?;
				}
				Some(Command::FluidToClient) => {
					self.fluid_send().await?;
				}
				Some(Command::FluidFromClient) => {
					self.fluid_recv().await?;
				}
				None => {
					//謎
					println!("unknown command {}", command);
					break;
				}
			}
		}
		Ok(())
	}
	pub(crate) fn freq(&self) -> &Frequency {
		self.freq.as_ref().unwrap()
	}
}
