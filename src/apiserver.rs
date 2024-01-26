// apiserver.rs

use axum::{extract::State, http::StatusCode, response::Html, routing::get, Json, Router};
pub use axum_macros::debug_handler;
use core::f32;
use log::*;
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
// use tower_http::trace::TraceLayer;

use crate::*;

pub async fn api_server(state: Arc<MyState>) -> anyhow::Result<()> {
    let listen = format!("0.0.0.0:{}", env!("API_PORT"));
    let addr = listen.parse::<SocketAddr>()?;

    let app = Router::new()
        .route(
            "/",
            get({
                let index = "<HTML></HTML>".to_string();
                move || async { Html(index) }
            }),
        )
        .route("/read", get(read_temp))
        .with_state(state);

    // .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("API server listening to {listen}");
    Ok(axum::serve(listener, app.into_make_service()).await?)
}

#[derive(Clone, Debug, Serialize)]
pub struct TempData {
    pub iopin: String,
    pub sensor: String,
    pub value: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct TempValues {
    pub temperatures: Vec<TempData>,
}

impl TempValues {
    pub fn new() -> Self {
        TempValues {
            temperatures: Vec::new(),
        }
    }
    pub fn with_capacity(c: usize) -> Self {
        TempValues {
            temperatures: Vec::with_capacity(c),
        }
    }
}

impl Default for TempValues {
    fn default() -> Self {
        Self::new()
    }
}

async fn read_temp(State(state): State<Arc<MyState>>) -> (StatusCode, Json<TempValues>) {
    let status;
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        status = format!("#{c}");
    }
    info!("Read: {status}");

    let mut ret;
    {
        let data = state.data.read().await;
        ret = TempValues::with_capacity(data.temperatures.len());
        data.temperatures
            .iter()
            // do not return invalid values
            .filter(|v| v.value > -100.0)
            .for_each(|v| ret.temperatures.push(v.clone()));
    }
    (StatusCode::OK, Json(ret))
}

// EOF
