// state.rs

use crate::*;

use esp_idf_hal::gpio::AnyIOPin;
use esp_storage::FlashStorage;
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
    pub cnt: RwLock<u64>,
    pub sensors: RwLock<Vec<MyOnewire>>,
    pub data: RwLock<TempValues>,
    pub flash: RwLock<FlashStorage>,
    pub reset: RwLock<bool>,
}

// EOF
