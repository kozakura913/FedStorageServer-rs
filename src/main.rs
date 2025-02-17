use std::{collections::HashMap, sync::Arc};

use client::{ClientMeta, ClientSession};
use fluid::Fluids;
use item::Items;
use tokio::{
	io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
	net::TcpListener,
	sync::{Mutex, RwLock},
};

mod client;
mod energy;
mod fluid;
mod item;

fn main() {
	let rt = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.expect("async runtime");
	let go = Arc::new(GlobalObject {
		item_buffers: RwLock::new(HashMap::new()),
		fluid_buffers: RwLock::new(HashMap::new()),
		energy_buffers: RwLock::new(HashMap::new()),
		clients: RwLock::new(HashMap::new()),
	});
	rt.block_on(async move {
		let bind = TcpListener::bind("0.0.0.0:3030").await;
		let listener = bind.expect("bind error");
		loop {
			tcp_loop(&listener, go.clone()).await;
		}
	});
}
async fn tcp_loop(listener: &TcpListener, go: Arc<GlobalObject>) {
	match listener.accept().await {
		Ok((soc, addr)) => {
			println!("connect");
			let client = ClientSession::new(soc, addr, go.clone());
			tokio::runtime::Handle::current().spawn(async move {
				let sid = client.meta.lock().await.id;
				if let Err(e) = client.session().await {
					eprintln!("{:?}", e);
				}
				go.clients.write().await.remove(&sid);
			});
		}
		Err(e) => {
			println!("client tcp error {:?}", e);
		}
	}
}
#[derive(Clone, Debug, Hash, Eq, PartialEq, PartialOrd)]
pub struct Frequency(pub String);
struct GlobalObject {
	item_buffers: RwLock<HashMap<Frequency, Arc<Items>>>,
	fluid_buffers: RwLock<HashMap<Frequency, Arc<Fluids>>>,
	energy_buffers: RwLock<HashMap<Frequency, i64>>,
	clients: RwLock<HashMap<uuid::Uuid, Arc<Mutex<ClientMeta>>>>,
}
pub fn to_hex_string(v: &[u8]) -> String {
	v.iter().map(|n| format!("{:02X}", n)).collect::<String>()
}
pub async fn read_string<R: AsyncRead + std::marker::Unpin>(
	reader: &mut R,
) -> Result<String, tokio::io::Error> {
	let len = reader.read_u16().await?;
	let mut v = vec![0u8; len.into()];
	reader.read_exact(&mut v).await?;
	let s = String::from_utf8(v);
	s.map_err(|e| tokio::io::Error::other(e))
}
pub async fn write_string<W: AsyncWrite + std::marker::Unpin>(
	writer: &mut W,
	s: impl AsRef<str>,
) -> Result<(), tokio::io::Error> {
	let s = s.as_ref().as_bytes();
	let len = s.len().try_into().map_err(|e| tokio::io::Error::other(e))?;
	writer.write_u16(len).await?;
	writer.write_all(s).await?;
	Ok(())
}
#[test]
fn read_write_string() {
	tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap()
		.block_on(async {
			let src = "0123456789abcdefABCDEFあア亜".to_string();
			let mut v = Vec::new();
			write_string(&mut v, &src).await.unwrap();
			let dst = read_string(&mut std::io::Cursor::new(&v)).await.unwrap();
			assert_eq!(src, dst);
		});
}
