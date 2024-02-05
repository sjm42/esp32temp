// state.rs

use crate::*;

use esp_idf_hal::gpio::AnyIOPin;
use esp_idf_svc::nvs;
use one_wire_bus::Address;
use std::net::Ipv4Addr;
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
    pub cnt: RwLock<u64>,
    pub wifi_up: RwLock<bool>,
    pub ip_addr: RwLock<Ipv4Addr>,
    pub myid: RwLock<String>,
    pub sensors: RwLock<Vec<MyOnewire>>,
    pub data: RwLock<TempValues>,
    pub nvs: RwLock<nvs::EspNvs<nvs::NvsDefault>>,
    pub reset: RwLock<bool>,
}

// EOF
