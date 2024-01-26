// state.rs

use crate::*;

use esp_idf_hal::gpio::AnyIOPin;
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
    pub cnt: RwLock<u64>,
    pub sensors: RwLock<Vec<MyOnewire>>,
    pub data: RwLock<TempValues>,
}

// EOF
