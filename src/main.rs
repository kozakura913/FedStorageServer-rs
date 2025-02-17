use std::{collections::HashMap, sync::Arc};

use client::ClientSession;
use fluid::Fluids;
use item::Items;
use tokio::{
	io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
	net::TcpListener,
	sync::RwLock,
};

mod client;
mod energy;
mod fluid;
mod item;

fn main() {
	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.expect("async runtime")
		.block_on(async {
			let bind = TcpListener::bind("0.0.0.0:3030").await;
			let listener = bind.expect("bind error");
			let go = Arc::new(GlobalObject {
				item_buffers: RwLock::new(HashMap::new()),
				fluid_buffers: RwLock::new(HashMap::new()),
				energy_buffers: RwLock::new(HashMap::new()),
			});
			loop {
				tcp_loop(&listener, go.clone()).await;
			}
		});
}
async fn tcp_loop(listener: &TcpListener, go: Arc<GlobalObject>) {
	match listener.accept().await {
		Ok((soc, addr)) => {
			println!("connect");
			let client = ClientSession::new(soc, addr, go);
			tokio::runtime::Handle::current().spawn(async {
				if let Err(e) = client.session().await {
					eprintln!("{:?}", e);
				}
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
