// apiserver.rs

use core::f32;
use std::{net, net::SocketAddr, pin::Pin, sync::Arc};

use askama::Template;
use axum::{extract::State, http::StatusCode, response::Html, routing::*, Json, Router};
use axum::body::Body;
use axum::http::{header, Response};
use axum::response::IntoResponse;
pub use axum_macros::debug_handler;
use log::*;
use serde::Serialize;
use tokio::time::{sleep, Duration};
// use tower_http::trace::TraceLayer;

use crate::*;

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

pub async fn run_api_server(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    loop {
        if *state.wifi_up.read().await {
            break;
        }
        sleep(Duration::from_secs(1)).await;
    }

    let listen = format!("0.0.0.0:{}", state.config.read().await.port);
    let addr = listen.parse::<SocketAddr>()?;

    let app = Router::new()
        .route("/", get(get_index))
        .route("/favicon.ico", get(get_favicon))
        .route("/conf", get(get_conf).post(set_conf).options(options))
        .route("/read", get(get_temp))
        .route("/reset_conf", get(reset_conf))
        .with_state(state);
    // .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("API server listening to {listen}");
    Ok(axum::serve(listener, app.into_make_service()).await?)
}


pub async fn options(State(state): State<Arc<Pin<Box<MyState>>>>) -> Response<Body> {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} options()");
    }
    (
        StatusCode::OK,
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "get,post"),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, "content-type"),
        ],
    )
        .into_response()
}

pub async fn get_index(State(state): State<Arc<Pin<Box<MyState>>>>) -> Response<Body> {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} get_index()");
    }

    let index = match state.config.read().await.clone().render() {
        Err(e) => {
            let err_msg = format!("Index template error: {e:?}\n");
            error!("{err_msg}");
            return (StatusCode::INTERNAL_SERVER_ERROR, err_msg).into_response();
        }
        Ok(s) => s,
    };
    (StatusCode::OK, Html(index)).into_response()
}

pub async fn get_favicon(State(state): State<Arc<Pin<Box<MyState>>>>) -> Response<Body> {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} get_favicon()");
    }
    let favicon = include_bytes!("favicon.ico");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/vnd.microsoft.icon")],
        favicon.to_vec(),
    )
        .into_response()
}


pub async fn get_conf(State(state): State<Arc<Pin<Box<MyState>>>>) -> (StatusCode, Json<MyConfig>) {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} get_conf()");
    }
    (StatusCode::OK, Json(state.config.read().await.clone()))
}

pub async fn set_conf(
    State(state): State<Arc<Pin<Box<MyState>>>>,
    Json(mut config): Json<MyConfig>,
) -> (StatusCode, String) {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} set_conf()");
    }

    if config.v4mask > 30 {
        let msg = "IPv4 mask error: bits must be between 0..30";
        error!("{}", msg);
        return (StatusCode::INTERNAL_SERVER_ERROR, msg.to_string());
    }

    if config.v4dhcp {
        // clear out these if we are using DHCP
        config.v4addr = net::Ipv4Addr::new(0, 0, 0, 0);
        config.v4mask = 0;
        config.v4gw = net::Ipv4Addr::new(0, 0, 0, 0);
        config.dns1 = net::Ipv4Addr::new(0, 0, 0, 0);
        config.dns2 = net::Ipv4Addr::new(0, 0, 0, 0);
    }

    info!("Saving new config to nvs...");
    Box::pin(save_conf(state, config)).await
}

pub async fn get_temp(
    State(state): State<Arc<Pin<Box<MyState>>>>,
) -> (StatusCode, Json<TempValues>) {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} get_temp()");
    }

    let mut ret;
    {
        let data = state.data.read().await;
        // info!("My current data:\n{data:#?}");
        ret = TempValues::with_capacity(data.temperatures.len());
        data.temperatures
            .iter()
            // do not return invalid values
            .filter(|v| v.value > NO_TEMP)
            .for_each(|v| ret.temperatures.push(v.clone()));
    }
    (StatusCode::OK, Json(ret))
}

pub async fn reset_conf(State(state): State<Arc<Pin<Box<MyState>>>>) -> (StatusCode, String) {
    {
        let mut c = state.cnt.write().await;
        *c += 1;
        info!("#{c} reset_conf()");
    }
    info!("Saving  default config to nvs...");
    Box::pin(save_conf(state, MyConfig::default())).await
}

async fn save_conf(state: Arc<Pin<Box<MyState>>>, config: MyConfig) -> (StatusCode, String) {
    let mut nvs = state.nvs.write().await;
    match config.to_nvs(&mut nvs) {
        Ok(_) => {
            info!("Config saved to nvs. Resetting soon...");
            *state.reset.write().await = true;
            (StatusCode::OK, "OK".to_string())
        }
        Err(e) => {
            let msg = format!("Nvs write error: {e:?}");
            error!("{}", msg);
            (StatusCode::INTERNAL_SERVER_ERROR, msg)
        }
    }
}
// EOF
