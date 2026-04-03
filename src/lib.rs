// lib.rs
#![warn(clippy::large_futures)]

pub use std::{
    any::Any,
    net,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

pub use anyhow::bail;
pub use askama::Template;
pub use chrono::*;
#[allow(ambiguous_glob_reexports)]
pub use esp_idf_hal::{
    delay::{Ets, FreeRtos},
    gpio::{self, *},
    peripherals::Peripherals,
};
pub use esp_idf_svc::{nvs, sntp, wifi::WifiDriver};
pub use serde::{Deserialize, Serialize};
pub use tokio::{
    sync::RwLock,
    time::{Duration, sleep, timeout},
};
pub use tracing::*;

mod config;
pub use config::*;

mod state;
pub use state::*;

mod measure;
pub use measure::*;

mod rmt_ow;
pub use rmt_ow::*;

mod mqtt;
pub use mqtt::*;

mod apiserver;
pub use apiserver::*;

mod esphome_api;
pub use esphome_api::*;

mod wifi;
pub use wifi::*;

pub const FW_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AP_MODE_SSID: &str = "esp32temp";
pub const AP_MODE_IP_ADDR: net::Ipv4Addr = net::Ipv4Addr::new(10, 42, 42, 1);
pub const AP_MODE_IP_MASK: u8 = 24;

#[cfg(feature = "esp32-c3")]
pub const LED_ACTIVE_LOW: bool = true;
#[cfg(feature = "esp-wroom-32")]
pub const LED_ACTIVE_LOW: bool = false;

pub const NO_TEMP: f32 = -1000.0;

#[derive(Clone, Debug, Serialize)]
pub struct TempData {
    pub iopin: String,
    pub sensor: String,
    pub value: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sensor {
    pub iopin: String,
    pub sensor: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TempValues {
    pub timestamp: i64,
    pub last_update: String,
    pub uptime: u32,
    pub uptime_s: String,
    pub temperatures: Vec<TempData>,
}

impl TempValues {
    pub fn new() -> Self {
        TempValues {
            timestamp: 0,
            last_update: "-".to_string(),
            uptime: 0,
            uptime_s: "-".to_string(),
            temperatures: Vec::new(),
        }
    }
    pub fn with_capacity(c: usize) -> Self {
        TempValues {
            timestamp: 0,
            last_update: "-".to_string(),
            uptime: 0,
            uptime_s: "-".to_string(),
            temperatures: Vec::with_capacity(c),
        }
    }
}

impl Default for TempValues {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SensorValues {
    pub sensors: Vec<Sensor>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Uptime {
    pub uptime: u32,
    pub uptime_s: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFirmware {
    url: String,
}

// EOF
