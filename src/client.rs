use std::{net::SocketAddr, sync::Arc};

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream};

use crate::{read_string, Frequency, GlobalObject};


const CLIENT_VERSION:i64=6;

pub(crate) struct ClientSession{
	pub(crate) reader: tokio::net::tcp::OwnedReadHalf,
	pub(crate) writer: tokio::net::tcp::OwnedWriteHalf,
	addr:std::net::SocketAddr,
	hostname: String,
	pack_start: chrono::DateTime<chrono::Utc>,
	last_sync_time: i64,
	freq:Option<Frequency>,
	pub(crate) go: Arc<GlobalObject>,
}

#[derive(FromPrimitive, ToPrimitive,Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(i8)]
#[allow(non_camel_case_types)]
enum Command{
	DATA_NOP = -1,
	DATA_FREQUENCY = 1,
	DATA_ITEM_RECEIVE = 2,
	DATA_ITEM_SEND = 3,
	DATA_FLUID_RECEIVE = 4,
	DATA_FLUID_SEND = 5,
	DATA_ENERGY_RECEIVE = 6,
	DATA_ENERGY_SEND = 7,
	DATA_HOST_NAME = 8,
	DATA_PACK_START = 9,
	DATA_PACK_END = 10,
}

impl ClientSession{
	pub fn new(soc: TcpStream,addr:SocketAddr,go:Arc<GlobalObject>)->Self{
		let (reader,writer)=soc.into_split();
		ClientSession{
			reader,writer,addr,
			hostname: "DefaultHostName".into(),
			pack_start: chrono::Utc::now(),
			last_sync_time: 0,
			freq:None,
			go,
		}
	}
	pub async fn session(mut self)->Result<(),tokio::io::Error>{
		self.writer.write_i64(CLIENT_VERSION).await?;
		println!("start session remote address {}",self.addr);
		loop{
			let command=self.reader.read_i8().await?;
			match Command::from_i8(command){
				Some(Command::DATA_NOP)=>{
					//NOP
				},
				Some(Command::DATA_HOST_NAME)=>{
					self.hostname=read_string(&mut self.reader).await?;
				},
				Some(Command::DATA_PACK_START)=>{
					self.pack_start=chrono::Utc::now();
				},
				Some(Command::DATA_PACK_END)=>{
					self.last_sync_time=(chrono::Utc::now()-self.pack_start).num_milliseconds();
				},
				Some(Command::DATA_FREQUENCY)=>{
					self.freq=Some(Frequency(read_string(&mut self.reader).await?));
				},
				Some(Command::DATA_ENERGY_SEND)=>{
					self.energy_send().await?;
				},
				Some(Command::DATA_ENERGY_RECEIVE)=>{
					self.energy_recv().await?;
				},
				Some(Command::DATA_ITEM_SEND)=>{
					self.item_send().await?;
				},
				Some(Command::DATA_ITEM_RECEIVE)=>{
					self.item_recv().await?;
				},
				command=>{
					//謎
					println!("unknown command {:?}",command);
					break;
				}
			}
		}
		Ok(())
	}
	pub(crate) fn freq(&self)->&Frequency{
		self.freq.as_ref().unwrap()
	}
}
