// apiserver.rs

use axum::{extract::State, http::StatusCode, response::Html, routing::*, Json, Router};
pub use axum_macros::debug_handler;
use core::f32;
use log::*;
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
// use tower_http::trace::TraceLayer;

use crate::*;

#[derive(Clone, Serialize)]
pub struct TempData {
    pub iopin: String,
    pub sensor: String,
    pub value: f32,
}

#[derive(Clone, Serialize)]
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

pub async fn api_server(state: Arc<MyState>) -> anyhow::Result<()> {
    let listen = format!("0.0.0.0:{}", state.config.read().await.api_port);
    let addr = listen.parse::<SocketAddr>()?;

    let app = Router::new()
        .route(
            "/",
            get({
                let index = "<HTML></HTML>".to_string();
                move || async { Html(index) }
            }),
        )
        .route("/read", get(get_temp))
        .route("/conf", get(get_conf).post(set_conf))
        .with_state(state);
    // .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("API server listening to {listen}");
    Ok(axum::serve(listener, app.into_make_service()).await?)
}

async fn get_temp(State(state): State<Arc<MyState>>) -> (StatusCode, Json<TempValues>) {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} get_temp()");
    }

    let mut ret;
    {
        let data = state.data.read().await;
        ret = TempValues::with_capacity(data.temperatures.len());
        data.temperatures
            .iter()
            // do not return invalid values
            .filter(|v| v.value > -1000.0)
            .for_each(|v| ret.temperatures.push(v.clone()));
    }
    (StatusCode::OK, Json(ret))
}

async fn get_conf(State(state): State<Arc<MyState>>) -> (StatusCode, Json<MyConfig>) {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} get_conf()");
    }

    (StatusCode::OK, Json(state.config.read().await.clone()))
}

async fn set_conf(
    State(state): State<Arc<MyState>>,
    Json(new_config): Json<MyConfig>,
) -> (StatusCode, String) {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} set_conf()");
    }

    let mut flash = state.flash.write().await;
    match new_config.to_flash(&mut flash) {
        Ok(_) => {
            info!("Config saved to flash. Resetting...");
            *state.reset.write().await = true;
            (StatusCode::OK, "OK".to_string())
        }
        Err(e) => {
            let msg = format!("Flash save error: {e:?}");
            error!("{}", msg);
            (StatusCode::INTERNAL_SERVER_ERROR, msg)
        }
    }
}

// EOF
