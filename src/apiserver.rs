// apiserver.rs

use axum::{
    Json, Router,
    body::Body,
    extract::{Form, State},
    http::{Response, StatusCode, header},
    response::{Html, IntoResponse},
    routing::*,
};
pub use axum_macros::debug_handler;
// use tower_http::trace::TraceLayer;
use embedded_svc::http::client::Client as HttpClient;
use esp_idf_svc::{http::client::EspHttpConnection, io, ota::EspOta};
use std::any::Any;

use crate::*;

pub async fn run_api_server(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    loop {
        if *state.wifi_up.read().await {
            break;
        }
        sleep(Duration::from_secs(1)).await;
    }

    let listen = format!("0.0.0.0:{}", state.config.port);
    let addr = listen.parse::<net::SocketAddr>()?;

    let app = Router::new()
        .route("/", get(get_index))
        .route("/favicon.ico", get(get_favicon))
        .route("/form.js", get(get_formjs))
        .route("/index.css", get(get_indexcss))
        .route("/uptime", get(get_uptime))
        .route("/temp", get(get_temp))
        .route(
            "/config",
            get(get_config).post(post_config).options(options),
        )
        .route("/reset_config", get(reset_config))
        .route("/fw", post(update_fw).options(options))
        .with_state(state);
    // .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("API server listening to {listen}");
    Ok(axum::serve(listener, app.into_make_service()).await?)
}

pub async fn options(State(state): State<Arc<Pin<Box<MyState>>>>) -> Response<Body> {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} options()");

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
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} get_index()");

    let value_tuple: (&str, &dyn Any) = ("ota_slot", &state.ota_slot.clone());
    let index = match state.config.clone().render_with_values(&value_tuple) {
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
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} get_favicon()");

    let favicon = include_bytes!("favicon.ico");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/vnd.microsoft.icon")],
        favicon.to_vec(),
    )
        .into_response()
}

pub async fn get_formjs(State(state): State<Arc<Pin<Box<MyState>>>>) -> Response<Body> {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} get_formjs()");

    let formjs = include_bytes!("form.js");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/javascript")],
        formjs.to_vec(),
    )
        .into_response()
}

pub async fn get_indexcss(State(state): State<Arc<Pin<Box<MyState>>>>) -> Response<Body> {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} get_indexcss()");

    let indexcss = include_bytes!("index.css");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        indexcss.to_vec(),
    )
        .into_response()
}

pub async fn get_uptime(State(state): State<Arc<Pin<Box<MyState>>>>) -> (StatusCode, Json<Uptime>) {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} get_uptime()");

    let uptime = Uptime {
        uptime: state.data.read().await.uptime,
        uptime_s: state.data.read().await.uptime_s.clone(),
    };
    (StatusCode::OK, Json(uptime))
}

pub async fn get_temp(
    State(state): State<Arc<Pin<Box<MyState>>>>,
) -> (StatusCode, Json<TempValues>) {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} get_temp()");

    let ret = {
        let data = state.data.read().await;
        // info!("My current data:\n{data:#?}");
        TempValues {
            timestamp: data.timestamp,
            last_update: data.last_update.clone(),
            uptime: data.uptime,
            uptime_s: data.uptime_s.clone(),
            temperatures: data
                .temperatures
                .iter()
                // do not return invalid values
                .filter(|v| v.value > NO_TEMP)
                .cloned()
                .collect::<Vec<TempData>>(),
        }
    };
    (StatusCode::OK, Json(ret))
}

pub async fn get_config(
    State(state): State<Arc<Pin<Box<MyState>>>>,
) -> (StatusCode, Json<MyConfig>) {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} get_conf()");
    (StatusCode::OK, Json(state.config.clone()))
}

pub async fn post_config(
    State(state): State<Arc<Pin<Box<MyState>>>>,
    Json(mut config): Json<MyConfig>,
) -> (StatusCode, String) {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} set_conf()");

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

pub async fn reset_config(State(state): State<Arc<Pin<Box<MyState>>>>) -> (StatusCode, String) {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} reset_conf()");

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

async fn update_fw(
    State(state): State<Arc<Pin<Box<MyState>>>>,
    Form(fw_update): Form<UpdateFirmware>,
) -> Response<Body> {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} update_fw()");

    info!("Firmware update: \n{fw_update:#?}");
    if !fw_update.url.starts_with("http://") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let mut ota = EspOta::new().unwrap();
    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default()).unwrap());

    let req = client.get(fw_update.url.as_str()).unwrap();
    let resp = req.submit().unwrap();
    if resp.status() != StatusCode::OK {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let mut update = ota.initiate_update().unwrap();
    let mut buffer = [0_u8; 8192];
    io::utils::copy(resp, &mut update, &mut buffer).unwrap();
    info!("Update done. Restarting...");
    update.complete().unwrap();
    esp_idf_svc::hal::reset::restart();

    // not reached
    // StatusCode::OK.into_response()
}

// EOF
