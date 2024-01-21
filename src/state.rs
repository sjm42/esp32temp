// state.rs

// use crate::*;

use esp_idf_hal::gpio::AnyIOPin;
use std::sync::RwLock;
// use tokio::sync::RwLock;

pub struct MyState {
    pub cnt: RwLock<u64>,
    pub onewire_pins: RwLock<Vec<(AnyIOPin, String)>>,
}

unsafe impl Send for MyState {}
unsafe impl Sync for MyState {}

// EOF
