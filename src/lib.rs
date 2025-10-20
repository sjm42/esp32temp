// lib.rs
#![warn(clippy::large_futures)]

pub use std::{net, pin::Pin, sync::Arc};

pub use anyhow::bail;
use askama::Template;
pub use chrono::*;
#[allow(ambiguous_glob_reexports)]
pub use esp_idf_hal::{
    delay::{Ets, FreeRtos},
    gpio::{self, *},
    prelude::*,
};
pub use serde::{Deserialize, Serialize};
pub use tokio::{
    sync::RwLock,
    time::{sleep, Duration},
};
pub use tracing::*;

mod config;
pub use config::*;

mod state;
pub use state::*;

mod measure;
pub use measure::*;

mod mqtt;
pub use mqtt::*;

mod apiserver;
pub use apiserver::*;

mod wifi;
pub use wifi::*;

pub const FW_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const NO_TEMP: f32 = -1000.0;

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

#[derive(Clone, Debug, Serialize)]
pub struct Uptime {
    pub uptime: usize,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFirmware {
    url: String,
}

// EOF
