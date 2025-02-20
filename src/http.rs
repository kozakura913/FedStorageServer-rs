use std::{net::SocketAddr, sync::Arc};

use axum::{
	extract::{Query, State},
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::get,
	Router,
};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;

use crate::{to_hex_string, GlobalObject};

pub(crate) async fn server(go: Arc<GlobalObject>) {
	let http_addr: SocketAddr = "0.0.0.0:3031".parse().unwrap();
	let app = Router::new();
	let app = app.route("/api/list/item_frequency.json", get(item_frequency));
	let app = app.route("/api/list/items.json", get(items));
	let app = app.route("/api/list/fluid_frequency.json", get(fluid_frequency));
	let app = app.route("/api/list/fluids.json", get(fluids));
	let app = app.route("/api/list/energy_frequency.json", get(energy_frequency));
	let app = app.route("/api/list/clients.json", get(clients));
	let app = app.fallback_service(ServeDir::new("html"));
	let app = app.with_state(go);
	let listener = tokio::net::TcpListener::bind(http_addr).await.unwrap();
	axum::serve(
		listener,
		app.into_make_service_with_connect_info::<SocketAddr>(),
	)
	.with_graceful_shutdown(shutdown_signal())
	.await
	.unwrap();
}
#[derive(Debug, Deserialize)]
struct ParmFreqList {
	frequency: String,
}
async fn fluids(
	State(go): State<Arc<GlobalObject>>,
	Query(params): Query<ParmFreqList>,
) -> Response {
	let fluid_buffers = go
		.fluid_buffers
		.read()
		.await
		.get(&crate::Frequency(params.frequency))
		.cloned();
	let fluid_buffers = match fluid_buffers {
		Some(v) => v,
		None => {
			return (StatusCode::OK, "[]".to_owned()).into_response();
		}
	};
	#[derive(Serialize, Debug)]
	struct ItemStack {
		name: String,
		count: i64,
		nbt: Option<String>,
	}
	let fluids = {
		let fluids = fluid_buffers.data.read().await;
		let jobs = fluids.iter().map(|(_id, fluid)| async {
			let nbt = fluid.nbt.as_ref().map(|b| to_hex_string(&b));
			ItemStack {
				name: fluid.name.clone(),
				count: fluid.count,
				nbt,
			}
		});
		futures::future::join_all(jobs)
			.await
			.into_iter()
			.collect::<Vec<_>>()
	};
	match serde_json::to_string(&fluids) {
		Ok(json) => (StatusCode::OK, json).into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
	}
}
async fn items(
	State(go): State<Arc<GlobalObject>>,
	Query(params): Query<ParmFreqList>,
) -> Response {
	let item_buffers = go
		.item_buffers
		.read()
		.await
		.get(&crate::Frequency(params.frequency))
		.cloned();
	let item_buffers = match item_buffers {
		Some(v) => v,
		None => {
			return (StatusCode::OK, "[]".to_owned()).into_response();
		}
	};
	#[derive(Serialize, Debug)]
	struct ItemStack {
		name: String,
		count: i32,
		nbt: Option<String>,
	}
	let items = {
		let items = item_buffers.data.read().await;
		let jobs = items.iter().map(|item| async {
			let nbt = item.nbt.as_ref().map(|b| to_hex_string(&b));
			ItemStack {
				name: item.id.clone(),
				count: item.count,
				nbt,
			}
		});
		futures::future::join_all(jobs)
			.await
			.into_iter()
			.collect::<Vec<_>>()
	};
	match serde_json::to_string(&items) {
		Ok(json) => (StatusCode::OK, json).into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
	}
}
async fn item_frequency(State(go): State<Arc<GlobalObject>>) -> Response {
	let item_buffers = go.item_buffers.read().await;
	#[derive(Serialize, Debug)]
	struct ItemFrequency {
		id: String,
		size: i64,
	}
	let items = {
		let jobs = item_buffers.iter().map(|(freq, items)| async {
			ItemFrequency {
				id: freq.0.clone(),
				size: items.len().await as i64,
			}
		});
		futures::future::join_all(jobs)
			.await
			.into_iter()
			.collect::<Vec<_>>()
	};
	match serde_json::to_string(&items) {
		Ok(json) => (StatusCode::OK, json).into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
	}
}
async fn fluid_frequency(State(go): State<Arc<GlobalObject>>) -> Response {
	let fluid_buffers = go.fluid_buffers.read().await;
	#[derive(Serialize, Debug)]
	struct FluidFrequency {
		id: String,
		size: i64,
	}
	let fluids = {
		let jobs = fluid_buffers.iter().map(|(freq, fluids)| async {
			FluidFrequency {
				id: freq.0.clone(),
				size: fluids.len().await as i64,
			}
		});
		futures::future::join_all(jobs)
			.await
			.into_iter()
			.collect::<Vec<_>>()
	};
	match serde_json::to_string(&fluids) {
		Ok(json) => (StatusCode::OK, json).into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
	}
}
async fn energy_frequency(State(go): State<Arc<GlobalObject>>) -> Response {
	let energy_buffers = go.energy_buffers.read().await;
	#[derive(Serialize, Debug)]
	struct EnergyFrequency {
		id: String,
		value: i64,
	}
	let values = {
		let jobs = energy_buffers.iter().map(|(freq, value)| async {
			EnergyFrequency {
				id: freq.0.clone(),
				value: *value,
			}
		});
		futures::future::join_all(jobs)
			.await
			.into_iter()
			.collect::<Vec<_>>()
	};
	match serde_json::to_string(&values) {
		Ok(json) => (StatusCode::OK, json).into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
	}
}
async fn clients(State(go): State<Arc<GlobalObject>>) -> Response {
	let clients = go.clients.read().await;
	#[derive(Serialize, Debug)]
	struct ClientMeta {
		name: String,
		sync: i64,
	}
	let clients = {
		let jobs = clients.iter().map(|(_id, meta)| async {
			let meta = meta.lock().await;
			ClientMeta {
				name: meta.hostname.clone(),
				sync: meta.last_sync_time,
			}
		});
		futures::future::join_all(jobs)
			.await
			.into_iter()
			.collect::<Vec<_>>()
	};
	match serde_json::to_string(&clients) {
		Ok(json) => (StatusCode::OK, json).into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
	}
}

async fn shutdown_signal() {
	use futures::{future::FutureExt, pin_mut};
	use tokio::signal;
	let ctrl_c = async {
		signal::ctrl_c()
			.await
			.expect("failed to install Ctrl+C handler");
	}
	.fuse();

	#[cfg(unix)]
	let terminate = async {
		signal::unix::signal(signal::unix::SignalKind::terminate())
			.expect("failed to install signal handler")
			.recv()
			.await;
	}
	.fuse();
	#[cfg(not(unix))]
	let terminate = std::future::pending::<()>().fuse();
	pin_mut!(ctrl_c, terminate);
	futures::select! {
		_ = ctrl_c => {},
		_ = terminate => {},
	}
}
