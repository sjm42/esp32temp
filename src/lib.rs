// lib.rs
#![warn(clippy::large_futures)]

pub use std::{pin::Pin, sync::Arc};

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

pub const NO_TEMP: f32 = -1000.0;

// EOF
