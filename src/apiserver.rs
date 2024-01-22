// apiserver.rs

use axum::{extract::State, http::StatusCode, response::Html, routing::get, Json, Router};
use core::f32;
use esp_idf_hal::gpio;
use log::*;
use one_wire_bus::OneWire;
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
// use tower_http::trace::TraceLayer;

use crate::*;

pub async fn api_server(state: MyState) -> anyhow::Result<()> {
    let listen = format!("0.0.0.0:{}", env!("API_PORT"));
    let addr = listen.parse::<SocketAddr>()?;

    let shared_state = Arc::new(state);
    let app = Router::new()
        .route(
            "/",
            get({
                let index = "<HTML></HTML>".to_string();
                move || async { Html(index) }
            }),
        )
        .route("/read", get(read_temp))
        .with_state(shared_state);

    // .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("API server listening to {listen}");
    Ok(axum::serve(listener, app.into_make_service()).await?)
}

#[derive(Debug, Serialize)]
pub struct TempData {
    iopin: String,
    sensor: String,
    value: f32,
}

#[derive(Debug, Serialize)]
pub struct TempValues {
    temperatures: Vec<TempData>,
}

async fn read_temp(State(state): State<Arc<MyState>>) -> (StatusCode, Json<TempValues>) {
    let status;
    {
        let mut c = match state.cnt.write() {
            Ok(c) => c,
            Err(e) => {
                error!("lock error: {e:#?}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(TempValues {
                        temperatures: Vec::new(),
                    }),
                );
            }
        };
        *c += 1;
        status = format!("#{c}");
    }
    info!("Read: {status}");

    let mut vals = TempValues {
        temperatures: Vec::new(),
    };
    {
        let mut pins = match state.onewire_pins.write() {
            Ok(pins) => pins,
            Err(e) => {
                error!("lock error {e:#?}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(TempValues {
                        temperatures: Vec::new(),
                    }),
                );
            }
        };

        for (_i, (pin, name)) in pins.iter_mut().enumerate() {
            let mut w = OneWire::new(gpio::PinDriver::input_output_od(pin).unwrap()).unwrap();
            if let Ok(meas) = measure_temperature(&mut w) {
                info!("Onewire response {name}:\n{meas:#?}");
                for m in meas.into_iter() {
                    vals.temperatures.push(TempData {
                        iopin: name.clone(),
                        sensor: m.device_id,
                        value: m.temperature,
                    });
                }
            }
            drop(w);
        }
    }

    (StatusCode::OK, Json(vals))
}

// EOF
