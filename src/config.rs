// config.rs

use anyhow::{Context, ensure};
use crc::{CRC_32_ISCSI, Crc};

use crate::*;

// Serialized configuration limit, including the CRC; not an NVS partition limit.
pub const NVS_BUF_SIZE: usize = 512;
pub const HTTP_API_PORT: u16 = 80;
const DEFAULT_SENSOR_RETRIES: u32 = 5;
const DEFAULT_POLL_DELAY: u64 = 60;

const CONFIG_NAME: &str = "cfg";

#[derive(Clone, Debug, Serialize, Deserialize, Template)]
#[template(path = "index.html.ask", escape = "html")]
pub struct MyConfig {
    pub retries: u32,
    pub delay: u64,

    pub wifi_ssid: String,
    pub wifi_pass: String,
    pub wifi_wpa2ent: bool,
    pub wifi_username: String,

    pub v4dhcp: bool,
    pub v4addr: net::Ipv4Addr,
    pub v4mask: u8,
    pub v4gw: net::Ipv4Addr,
    pub dns1: net::Ipv4Addr,
    pub dns2: net::Ipv4Addr,

    pub esphome_enable: bool,
    pub mqtt_enable: bool,
    pub mqtt_url: String,
    pub mqtt_topic: String,
}

impl Default for MyConfig {
    fn default() -> Self {
        Self {
            wifi_ssid: option_env!("WIFI_SSID").unwrap_or("").into(),
            wifi_pass: option_env!("WIFI_PASS").unwrap_or("").into(),
            wifi_wpa2ent: false,
            wifi_username: String::new(),

            v4dhcp: true,
            v4addr: net::Ipv4Addr::new(0, 0, 0, 0),
            v4mask: 0,
            v4gw: net::Ipv4Addr::new(0, 0, 0, 0),
            dns1: net::Ipv4Addr::new(0, 0, 0, 0),
            dns2: net::Ipv4Addr::new(0, 0, 0, 0),

            esphome_enable: false,
            mqtt_enable: false,
            mqtt_url: "mqtt://mqtt.local:1883".into(),
            mqtt_topic: "esp32temp".into(),

            retries: DEFAULT_SENSOR_RETRIES,
            delay: DEFAULT_POLL_DELAY,
        }
    }
}

impl MyConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            (1..=86400).contains(&self.delay),
            "Sensor poll interval must be between 1 and 86400 seconds"
        );
        ensure!(self.retries <= 20, "Sensor read retries must be at most 20");
        ensure!(
            self.wifi_ssid.len() <= 32,
            "WiFi SSID must be at most 32 bytes"
        );
        ensure!(
            !self.wifi_ssid.contains('\0'),
            "WiFi SSID must not contain NUL"
        );
        ensure!(
            !self.wifi_pass.contains('\0'),
            "WiFi password must not contain NUL"
        );
        if self.has_wifi_config() {
            if self.wifi_wpa2ent {
                ensure!(
                    (1..=127).contains(&self.wifi_username.len()),
                    "Enterprise username must be between 1 and 127 bytes"
                );
                ensure!(
                    !self.wifi_username.contains('\0'),
                    "Enterprise username must not contain NUL"
                );
                ensure!(
                    !self.wifi_pass.is_empty(),
                    "Enterprise password must not be empty"
                );
            } else if !self.wifi_pass.is_empty() {
                ensure!(
                    (8..=63).contains(&self.wifi_pass.len())
                        || (self.wifi_pass.len() == 64
                            && self.wifi_pass.bytes().all(|byte| byte.is_ascii_hexdigit())),
                    "WiFi password must be 8–63 bytes or a 64-digit hexadecimal key"
                );
            }
        }
        if !self.v4dhcp {
            ensure!(
                (1..=30).contains(&self.v4mask),
                "Static IPv4 mask must be between 1 and 30"
            );
            ensure!(
                !self.v4addr.is_unspecified() && !self.v4addr.is_multicast(),
                "Static IPv4 address must be a unicast address"
            );
            ensure!(
                !self.v4gw.is_unspecified() && !self.v4gw.is_multicast(),
                "Static IPv4 gateway must be a unicast address"
            );
        }
        if self.mqtt_enable {
            ensure!(
                self.mqtt_url.starts_with("mqtt://") || self.mqtt_url.starts_with("mqtts://"),
                "MQTT URL must start with mqtt:// or mqtts://"
            );
            ensure!(
                !self.mqtt_url.contains('\0'),
                "MQTT URL must not contain NUL"
            );
            let uri = self
                .mqtt_url
                .parse::<axum::http::Uri>()
                .context("Invalid MQTT URL")?;
            ensure!(uri.authority().is_some(), "MQTT URL must include a host");
            ensure!(
                !self.mqtt_topic.is_empty() && !self.mqtt_topic.contains(['#', '+', '\0']),
                "MQTT topic prefix must be nonempty and contain no wildcards or NUL"
            );
        }
        let mut buffer = [0_u8; NVS_BUF_SIZE];
        self.encode(&mut buffer)?;
        Ok(())
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> anyhow::Result<&'a [u8]> {
        let crc = Crc::<u32>::new(&CRC_32_ISCSI);
        let digest = crc.digest();
        postcard::to_slice_crc32(self, buffer, digest)
            .map(|bytes| &*bytes)
            .context("Configuration exceeds the 512-byte storage limit")
    }

    pub fn has_wifi_config(&self) -> bool {
        !self.wifi_ssid.trim().is_empty()
    }

    pub fn from_nvs(nvs: &mut nvs::EspNvs<nvs::NvsDefault>) -> Option<Self> {
        let mut nvsbuf = [0u8; NVS_BUF_SIZE];
        info!("Reading up to {sz} bytes from nvs...", sz = NVS_BUF_SIZE);
        let b = match nvs.get_blob(CONFIG_NAME, &mut nvsbuf) {
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
                if let Err(error) = c.validate() {
                    error!("Invalid stored configuration: {error:#}");
                    return None;
                }
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
        self.validate()?;
        let mut nvsbuf = [0u8; NVS_BUF_SIZE];
        let nvsdata = self.encode(&mut nvsbuf)?;
        info!(
            "Encoded config to {sz} bytes. Saving to nvs...",
            sz = nvsdata.len()
        );

        match nvs.set_blob(CONFIG_NAME, nvsdata) {
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
