// config.rs

use std::net;

use anyhow::bail;
use askama::Template;
use crc::{Crc, CRC_32_ISCSI};
use esp_idf_svc::nvs;
use log::*;
use serde::{Deserialize, Serialize};


pub const NVS_BUF_SIZE: usize = 256;

const DEFAULT_API_PORT: u16 = 80;
const DEFAULT_SENSOR_RETRIES: u32 = 4;
const DEFAULT_POLL_DELAY: u64 = 30;

const CONFIG_NAME: &str = "cfg";

#[derive(Clone, Debug, Serialize, Deserialize, Template)]
#[template(path = "index.html.ask", escape = "html")]
pub struct MyConfig {
    pub port: u16,
    pub retries: u32,
    pub delay: u64,

    pub wifi_ssid: String,
    pub wifi_pass: String,

    pub v4dhcp: bool,
    pub v4addr: net::Ipv4Addr,
    pub v4mask: u8,
    pub v4gw: net::Ipv4Addr,
    pub dns1: net::Ipv4Addr,
    pub dns2: net::Ipv4Addr,

    pub mqtt_enable: bool,
    pub mqtt_url: String,
    pub mqtt_topic: String,
}

impl Default for MyConfig {
    fn default() -> Self {
        Self {
            port: option_env!("API_PORT")
                .unwrap_or("-")
                .parse()
                .unwrap_or(DEFAULT_API_PORT),
            retries: DEFAULT_SENSOR_RETRIES,
            delay: DEFAULT_POLL_DELAY,

            wifi_ssid: option_env!("WIFI_SSID").unwrap_or("internet").into(),
            wifi_pass: option_env!("WIFI_PASS").unwrap_or("password").into(),

            v4dhcp: true,
            v4addr: net::Ipv4Addr::new(0, 0, 0, 0),
            v4mask: 0,
            v4gw: net::Ipv4Addr::new(0, 0, 0, 0),
            dns1: net::Ipv4Addr::new(0, 0, 0, 0),
            dns2: net::Ipv4Addr::new(0, 0, 0, 0),

            mqtt_enable: false,
            mqtt_url: "mqtt://mqtt.local:1883".into(),
            mqtt_topic: "esp32temp".into(),
        }
    }
}

impl MyConfig {
    pub fn from_nvs(nvs: &mut nvs::EspNvs<nvs::NvsDefault>) -> Option<Self> {
        let mut nvsbuf = [0u8; NVS_BUF_SIZE];
        info!("Reading up to {sz} bytes from nvs...", sz = NVS_BUF_SIZE);
        let b = match nvs.get_raw(CONFIG_NAME, &mut nvsbuf) {
            Err(e) => {
                error!("Nvs read error {e:?}");
                return None;
            }
            Ok(Some(b)) => b,
            _ => {
                error!("Nvs key not found");
                return None;
            }
        };
        info!("Got {sz} bytes from nvs. Parsing config...", sz = b.len());

        let crc = Crc::<u32>::new(&CRC_32_ISCSI);
        let digest = crc.digest();
        match postcard::from_bytes_crc32::<MyConfig>(b, digest) {
            Ok(c) => {
                info!("Successfully parsed config from nvs.");
                Some(c)
            }
            Err(e) => {
                error!("Cannot parse config from nvs: {e:?}");
                None
            }
        }
    }

    pub fn to_nvs(&self, nvs: &mut nvs::EspNvs<nvs::NvsDefault>) -> anyhow::Result<()> {
        let mut nvsbuf = [0u8; NVS_BUF_SIZE];
        let crc = Crc::<u32>::new(&CRC_32_ISCSI);
        let digest = crc.digest();
        let nvsdata = match postcard::to_slice_crc32(self, &mut nvsbuf, digest) {
            Ok(d) => d,
            Err(e) => {
                let estr = format!("Cannot encode config to buffer {e:?}");
                bail!("{estr}");
            }
        };
        info!(
            "Encoded config to {sz} bytes. Saving to nvs...",
            sz = nvsdata.len()
        );

        match nvs.set_raw(CONFIG_NAME, nvsdata) {
            Ok(_) => {
                info!("Config saved.");
                Ok(())
            }
            Err(e) => {
                let estr = format!("Cannot save to nvs: {e:?}");
                bail!("{estr}");
            }
        }
    }
}

// EOF
