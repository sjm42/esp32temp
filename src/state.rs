// state.rs

use crate::*;

use std::net::{self, Ipv4Addr};

use esp_idf_hal::gpio::AnyIOPin;
use esp_idf_svc::nvs;
use one_wire_bus::Address;
use tokio::sync::RwLock;

pub struct MyOnewire {
    pub pin: AnyIOPin,
    pub name: String,
    pub ids: Vec<Address>,
}
unsafe impl Send for MyOnewire {}
unsafe impl Sync for MyOnewire {}

pub struct MyState {
    pub config: RwLock<MyConfig>,
    pub uptime: RwLock<usize>,
    pub api_cnt: RwLock<u64>,
    pub wifi_up: RwLock<bool>,
    pub if_index: RwLock<u32>,
    pub ip_addr: RwLock<Ipv4Addr>,
    pub ping_ip: RwLock<Option<Ipv4Addr>>,
    pub myid: RwLock<String>,
    pub sensors: RwLock<Vec<MyOnewire>>,
    pub data: RwLock<TempValues>,
    pub data_updated: RwLock<bool>,
    pub nvs: RwLock<nvs::EspNvs<nvs::NvsDefault>>,
    pub reset: RwLock<bool>,
}

impl MyState {
    pub fn new(config: MyConfig, onewire_pins: Vec<MyOnewire>, temp_data: TempValues, nvs: nvs::EspNvs<nvs::NvsDefault>) -> Self {
        MyState {
            config: RwLock::new(config),
            uptime: RwLock::new(0),
            api_cnt: RwLock::new(0),
            wifi_up: RwLock::new(false),
            if_index: RwLock::new(0),
            ip_addr: RwLock::new(net::Ipv4Addr::new(0, 0, 0, 0)),
            ping_ip: RwLock::new(None),
            myid: RwLock::new("esp32temp".into()),
            sensors: RwLock::new(onewire_pins),
            data: RwLock::new(temp_data),
            data_updated: RwLock::new(false),
            nvs: RwLock::new(nvs),
            reset: RwLock::new(false),
        }
    }
}
// EOF
