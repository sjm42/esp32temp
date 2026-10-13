// apiserver.rs

use anyhow::Context;
use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Form, State,
        rejection::{FormRejection, JsonRejection},
    },
    http::{Response, StatusCode, header},
    response::{Html, IntoResponse},
    routing::*,
};
use embedded_svc::http::client::Client as HttpClient;
use esp_idf_svc::{
    http::client::{Configuration as HttpConfiguration, EspHttpConnection},
    ota::EspOta,
};

use crate::ota_transfer::{TransferError, copy_firmware};
use crate::*;

const HTTP_BODY_LIMIT: usize = 8 * 1024;
const REBOOT_DELAY: Duration = Duration::from_secs(2);
const OTA_TRANSFER_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    BadGateway(String),
    #[error("A configuration change or firmware update is already pending")]
    Conflict,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response<Body> {
        error!("HTTP handler failed: {self:#}");
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::BadGateway(message) => (StatusCode::BAD_GATEWAY, message),
            Self::Conflict => (StatusCode::CONFLICT, self.to_string()),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".into(),
            ),
        };
        json_error(status, message)
    }
}

// Own this guard inside the blocking OTA worker, so a disconnected browser cannot
// release it while flash writes are still in progress.
struct MaintenanceGuard {
    state: Arc<Pin<Box<MyState>>>,
    release_on_drop: bool,
}

impl MaintenanceGuard {
    fn acquire(state: Arc<Pin<Box<MyState>>>) -> Result<Self, ApiError> {
        state
            .maintenance_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ApiError::Conflict)?;
        Ok(Self {
            state,
            release_on_drop: true,
        })
    }

    fn reboot(mut self) {
        self.release_on_drop = false;
        let state = self.state.clone();
        tokio::spawn(async move {
            sleep(REBOOT_DELAY).await;
            *state.reset.write().await = true;
        });
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        if self.release_on_drop {
            self.state
                .maintenance_pending
                .store(false, Ordering::Release);
        }
    }
}

fn json_ok(message: &str) -> Response<Body> {
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": true, "message": message})),
    )
        .into_response()
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response<Body> {
    (
        status,
        Json(serde_json::json!({"ok": false, "message": message.into()})),
    )
        .into_response()
}

macro_rules! static_handler {
    ($fn_name:ident, $path:literal, $content_type:literal, $bytes:expr) => {
        async fn $fn_name(State(state): State<Arc<Pin<Box<MyState>>>>) -> Response<Body> {
            let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
            info!("#{cnt} {}", $path);

            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, $content_type),
                    (header::CONTENT_ENCODING, "gzip"),
                ],
                $bytes.to_vec(),
            )
                .into_response()
        }
    };
}
static_handler!(
    get_favicon,
    "/favicon.ico",
    "image/vnd.microsoft.icon",
    include_bytes!(concat!(env!("OUT_DIR"), "/favicon.ico.gz"))
);
static_handler!(
    get_formjs,
    "/form.js",
    "text/javascript",
    include_bytes!(concat!(env!("OUT_DIR"), "/form.js.gz"))
);
static_handler!(
    get_indexcss,
    "/index.css",
    "text/css; charset=utf-8",
    include_bytes!(concat!(env!("OUT_DIR"), "/index.css.gz"))
);

pub async fn run_api_server(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    loop {
        if *state.wifi_up.read().await {
            break;
        }
        sleep(Duration::from_secs(1)).await;
    }

    let listen = format!("0.0.0.0:{}", HTTP_API_PORT);
    let addr = listen.parse::<net::SocketAddr>()?;

    let app = Router::new()
        .route("/", get(get_index))
        .route("/favicon.ico", get(get_favicon))
        .route("/form.js", get(get_formjs))
        .route("/index.css", get(get_indexcss))
        .route("/uptime", get(get_uptime))
        .route("/sensors", get(get_sensors))
        .route("/temp", get(get_temp))
        .route(
            "/config",
            get(get_config).post(post_config).options(options),
        )
        .route("/reset_config", get(reset_config))
        .route("/fw", post(update_fw).options(options));
    #[cfg(feature = "ota-rollback-probe")]
    let app = app.route("/ota-test/rollback", post(test_ota_fallback));
    let app = app
        .layer(DefaultBodyLimit::max(HTTP_BODY_LIMIT))
        .with_state(state);
    // .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("API server listening to {listen}");
    Ok(axum::serve(listener, app.into_make_service()).await?)
}

#[cfg(feature = "ota-rollback-probe")]
async fn test_ota_fallback(
    State(state): State<Arc<Pin<Box<MyState>>>>,
) -> Result<Response<Body>, ApiError> {
    let operation = MaintenanceGuard::acquire(state)?;
    tokio::task::spawn_blocking(move || {
        let _ota = EspOta::new().context("Could not acquire OTA service")?;
        let mut nvs = operation.state.nvs.blocking_write();
        ota_probe::request_fallback(&operation.state.ota_details, &mut nvs)
            .map_err(|error| ApiError::BadRequest(format!("Fallback test refused: {error:#}")))?;
        drop(nvs);
        operation.reboot();
        Ok::<_, ApiError>(())
    })
    .await
    .context("Fallback test worker failed")??;
    Ok(json_ok(
        "Fallback test armed; restarting twice to return to previous firmware",
    ))
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

    let value_tuple: (&str, &dyn Any) = ("ota_details", &state.ota_details);
    let index = match state.config.render_with_values(&value_tuple) {
        Err(e) => {
            let err_msg = format!("Index template error: {e:?}\n");
            error!("{err_msg}");
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not render index page",
            );
        }
        Ok(s) => s,
    };
    (StatusCode::OK, Html(index)).into_response()
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

pub async fn get_sensors(
    State(state): State<Arc<Pin<Box<MyState>>>>,
) -> (StatusCode, Json<SensorValues>) {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} get_sensors()");

    let sensors = SensorValues {
        sensors: state.sensor_inventory.clone(),
    };

    (StatusCode::OK, Json(sensors))
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
    config: Result<Json<MyConfig>, JsonRejection>,
) -> Response<Body> {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} set_conf()");

    let Json(mut config) = match config {
        Ok(config) => config,
        Err(error) => return json_error(error.status(), format!("Invalid config JSON: {error}")),
    };

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

pub async fn reset_config(State(state): State<Arc<Pin<Box<MyState>>>>) -> Response<Body> {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} reset_conf()");

    info!("Saving  default config to nvs...");
    Box::pin(save_conf(state, MyConfig::default())).await
}

async fn save_conf(state: Arc<Pin<Box<MyState>>>, config: MyConfig) -> Response<Body> {
    if let Err(error) = config.validate() {
        return ApiError::BadRequest(error.to_string()).into_response();
    }
    let operation = match MaintenanceGuard::acquire(state.clone()) {
        Ok(operation) => operation,
        Err(error) => return error.into_response(),
    };
    let result = config.to_nvs(&mut *state.nvs.write().await);
    match result {
        Ok(()) => {
            info!("Configuration saved; restarting soon");
            operation.reboot();
            json_ok("Configuration saved, restarting")
        }
        Err(error) => ApiError::Internal(error).into_response(),
    }
}

async fn update_fw(
    State(state): State<Arc<Pin<Box<MyState>>>>,
    request: Result<Form<UpdateFirmware>, FormRejection>,
) -> Result<Response<Body>, ApiError> {
    let cnt = state.api_cnt.fetch_add(1, Ordering::Relaxed);
    info!("#{cnt} update_fw()");
    let Form(request) = match request {
        Ok(request) => request,
        Err(error) => {
            return Ok(json_error(
                error.status(),
                format!("Invalid firmware request: {error}"),
            ));
        }
    };
    let url = request.url.trim().to_owned();
    let uri = url
        .parse::<axum::http::Uri>()
        .map_err(|_| ApiError::BadRequest("Invalid firmware URL".into()))?;
    if uri.scheme_str() != Some("http") || uri.authority().is_none() {
        return Err(ApiError::BadRequest(
            "Firmware URL must start with http:// and include a host".into(),
        ));
    }
    let operation = MaintenanceGuard::acquire(state)?;
    tokio::task::spawn_blocking(move || {
        // Completion and reboot scheduling continue even if the request is cancelled.
        download_firmware(&url)?;
        operation.reboot();
        Ok::<_, ApiError>(())
    })
    .await
    .context("Firmware download worker failed")??;
    Ok(json_ok("Firmware updated, restarting"))
}

fn download_firmware(url: &str) -> Result<(), ApiError> {
    let mut ota = EspOta::new().context("Could not acquire OTA service")?;
    // SAFETY: IDF returns an immutable partition descriptor, or NULL if unavailable.
    let partition =
        unsafe { esp_idf_sys::esp_ota_get_next_update_partition(std::ptr::null()).as_ref() }
            .context("No OTA update partition available")?;
    let capacity = partition.size as usize;
    let config = HttpConfiguration {
        timeout: Some(Duration::from_secs(30)),
        ..Default::default()
    };
    let connection =
        EspHttpConnection::new(&config).context("Could not initialize firmware HTTP client")?;
    let mut client = HttpClient::wrap(connection);
    let request = client
        .get(url)
        .map_err(|error| ApiError::BadRequest(format!("Invalid firmware URL: {error}")))?;
    let mut response = request
        .submit()
        .map_err(|error| ApiError::BadGateway(format!("Firmware download failed: {error}")))?;
    if response.status() != 200 {
        return Err(ApiError::BadGateway(format!(
            "Firmware download failed: HTTP {}",
            response.status()
        )));
    }
    let expected_size = response
        .header("Content-Length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| ApiError::BadGateway("Invalid firmware Content-Length".into()))?;
    if expected_size.is_some_and(|size| size == 0 || size > capacity) {
        return Err(ApiError::BadGateway(format!(
            "Firmware must fit the {capacity}-byte OTA slot"
        )));
    }
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(8192)
        .context("Could not allocate OTA transfer buffer")?;
    buffer.resize(8192, 0);
    let mut update = ota
        .initiate_update()
        .context("Could not begin OTA update")?;
    let size = copy_firmware(
        &mut response,
        &mut update,
        &mut buffer,
        capacity,
        expected_size,
        std::time::Instant::now() + OTA_TRANSFER_TIMEOUT,
    )
    .map_err(|error| match error {
        TransferError::Read(_) | TransferError::Invalid(_) => {
            ApiError::BadGateway(error.to_string())
        }
        TransferError::Write(_) => ApiError::Internal(error.into()),
    })?;
    // Validate the complete image before changing the boot partition.
    update
        .finish()
        .map_err(|error| {
            ApiError::BadGateway(format!("Firmware image validation failed: {error}"))
        })?
        .activate()
        .context("Could not activate firmware image")?;
    info!("Firmware updated ({size} bytes); restarting soon");
    Ok(())
}

// EOF
