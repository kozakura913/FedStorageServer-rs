use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use serde::{Deserialize, Serialize};
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
	//最新情報。API応答はmeta内
	freq_access: FrequencyAccess,
}
pub struct ClientMeta {
	pub(crate) id: uuid::Uuid,
	pub(crate) addr: std::net::SocketAddr,
	pub hostname: String,
	pub last_sync_time: i64,
	//API応答はこれ。リアルタイム更新は同期コストが高いからPackEnd毎に更新
	pub last_freq_access: FrequencyAccess,
}
#[derive(Deserialize, Serialize, Hash, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum FrequencyRW {
	ToClient,
	FromClient,
}
#[derive(Deserialize, Serialize, Debug)]
pub struct FrequencyAccess {
	item: HashMap<Frequency, FrequencyRW>,
	fluid: HashMap<Frequency, FrequencyRW>,
	energy: HashMap<Frequency, FrequencyRW>,
}
impl FrequencyAccess {
	fn clear(&mut self) {
		self.item.clear();
		self.fluid.clear();
		self.energy.clear();
	}
	fn move_to(&mut self, dst: &mut Self) {
		dst.item.extend(self.item.drain());
		dst.fluid.extend(self.fluid.drain());
		dst.energy.extend(self.energy.drain());
	}
	fn new() -> Self {
		Self {
			item: HashMap::new(),
			fluid: HashMap::new(),
			energy: HashMap::new(),
		}
	}
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
			last_freq_access: FrequencyAccess::new(),
		}));
		ClientSession {
			reader,
			writer,
			pack_start: chrono::Utc::now(),
			freq: None,
			meta,
			go,
			freq_access: FrequencyAccess::new(),
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
					self.freq_access.clear();
				}
				Some(Command::PackEnd) => {
					let mut meta = self.meta.lock().await;
					meta.last_sync_time = (chrono::Utc::now() - self.pack_start).num_milliseconds();
					//過去記録を消すと搬入が判断できなくなる(搬入が無い時要求が無い)
					//meta.last_freq_access.clear();
					self.freq_access.move_to(&mut meta.last_freq_access);
				}
				Some(Command::SetFrequency) => {
					self.freq = Some(Frequency(read_string(&mut self.reader).await?));
				}
				Some(Command::EnergyToClient) => {
					self.energy_send().await?;
					self.freq_access
						.energy
						.insert(self.freq().clone(), FrequencyRW::ToClient);
				}
				Some(Command::EnergyFromClient) => {
					self.energy_recv().await?;
					self.freq_access
						.energy
						.insert(self.freq().clone(), FrequencyRW::FromClient);
				}
				Some(Command::ItemToClient) => {
					self.item_send().await?;
					self.freq_access
						.item
						.insert(self.freq().clone(), FrequencyRW::ToClient);
				}
				Some(Command::ItemFromClient) => {
					self.item_recv().await?;
					self.freq_access
						.item
						.insert(self.freq().clone(), FrequencyRW::FromClient);
				}
				Some(Command::FluidToClient) => {
					self.fluid_send().await?;
					self.freq_access
						.fluid
						.insert(self.freq().clone(), FrequencyRW::ToClient);
				}
				Some(Command::FluidFromClient) => {
					self.fluid_recv().await?;
					self.freq_access
						.fluid
						.insert(self.freq().clone(), FrequencyRW::FromClient);
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
