use std::{
	collections::HashMap,
	sync::Arc,
};

use num_derive::{FromPrimitive,ToPrimitive};
use num_traits::FromPrimitive;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener, sync::RwLock};

const CLIENT_VERSION:i64=6;

fn main() {
	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.expect("async runtime")
		.block_on(async {
			let bind = TcpListener::bind("0.0.0.0:3030").await;
			let listener = bind.expect("bind error");
			let go=Arc::new(GlobalObject{
				item_buffers: RwLock::new(HashMap::new()),
				energy_buffers: RwLock::new(HashMap::new()),
			});
			loop{
				tcp_loop(&listener,go.clone()).await;
			}
		});
}
async fn tcp_loop(listener: &TcpListener,go:Arc<GlobalObject>){
	match listener.accept().await{
		Ok((soc,addr))=>{
			println!("connect");
			let (reader,writer)=soc.into_split();
			let client=ClientSession{
				reader,writer,addr,
				hostname: "DefaultHostName".into(),
				pack_start: chrono::Utc::now(),
				last_sync_time: 0,
				freq:None,
				go,
			};
			tokio::runtime::Handle::current().spawn(async{
				if let Err(e)=client.session().await{
					eprintln!("{:?}",e);
				}
			});
		},
		Err(e)=>{
			println!("client tcp error {:?}",e);
		}
	}
}
struct ClientSession{
	reader: tokio::net::tcp::OwnedReadHalf,
	writer: tokio::net::tcp::OwnedWriteHalf,
	addr:std::net::SocketAddr,
	hostname: String,
	pack_start: chrono::DateTime<chrono::Utc>,
	last_sync_time: i64,
	freq:Option<Frequency>,
	go: Arc<GlobalObject>,
}
impl ClientSession{
	async fn session(mut self)->Result<(),tokio::io::Error>{
		self.writer.write_i64(CLIENT_VERSION).await?;
		println!("start session remote address {}",self.addr);
		loop{
			let command=self.reader.read_i8().await?;
			match Command::from_i8(command){
				Some(Command::DATA_NOP)=>{
					//NOP
				},
				Some(Command::DATA_HOST_NAME)=>{
					self.hostname=self.read_string().await?;
				},
				Some(Command::DATA_PACK_START)=>{
					self.pack_start=chrono::Utc::now();
				},
				Some(Command::DATA_PACK_END)=>{
					self.last_sync_time=(chrono::Utc::now()-self.pack_start).num_milliseconds();
				},
				Some(Command::DATA_FREQUENCY)=>{
					self.freq=Some(Frequency(self.read_string().await?));
				},
				Some(Command::DATA_ENERGY_SEND)=>{
					self.energy_send().await?;
				},
				Some(Command::DATA_ENERGY_RECEIVE)=>{
					self.energy_recv().await?;
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
	async fn energy_recv(&mut self) -> Result<(),tokio::io::Error>{
		let energy=self.reader.read_i64().await?;
		let reject={
			let mut lock=self.go.energy_buffers.write().await;
			let v=lock.remove(self.freq());
			let v=v.unwrap_or(0);
			let available=0.max(u32::MAX as i64-energy);
			let reject=0.max(v-available);
			let energy=energy+(v-reject);
			lock.insert(self.freq().clone(), energy);
			reject
		};
		self.writer.write_i64(reject).await?;
		Ok(())
	}
	async fn energy_send(&mut self) -> Result<(),tokio::io::Error>{
		let max_recv=self.reader.read_i64().await?;
		let max_recv=max_recv.max(0);
		let send={
			let mut lock=self.go.energy_buffers.write().await;
			let v=lock.remove(self.freq());
			let v=v.unwrap_or(0);
			let send=max_recv.min(v);
			let v=v-send;
			if v>0{
				lock.insert(self.freq().clone(),v);
			}
			send
		};
		self.writer.write_i64(send).await?;
		Ok(())
	}
	fn freq(&self)->&Frequency{
		self.freq.as_ref().unwrap()
	}
	async fn read_string(&mut self)->Result<String,tokio::io::Error>{
		let len=self.reader.read_u16().await?;
		let mut v=vec![0u8;len.into()];
		self.reader.read_exact(&mut v).await?;
		let s=String::from_utf8(v);
		s.map_err(|e|tokio::io::Error::other(e))
	}
}
struct GlobalObject {
	item_buffers: RwLock<HashMap<Frequency, Arc<Items>>>,
	energy_buffers: RwLock<HashMap<Frequency, i64>>,
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
#[derive(Clone, Debug, Hash,Eq, PartialEq, PartialOrd)]
pub struct Frequency(pub String);

#[derive(Clone, Debug)]
pub struct Items {
	data: Arc<RwLock<Vec<ItemStack>>>,
}
#[derive(Clone, Debug, Hash, PartialEq, PartialOrd)]
pub struct ItemStack {
	damage: i32,
	count: i32,
	id: String,
	nbt: Option<Vec<u8>>,
}
fn wip_fn() -> i32 {
	9
}
#[test]
fn wip() {
	assert_eq!(9, wip_fn());
}
