// state.rs

use esp_idf_svc::nvs;
use one_wire_bus::Address;

use crate::*;

pub struct MyOnewire {
    pub pin: AnyIOPin,
    pub name: String,
    pub ids: Vec<Address>,
}
unsafe impl Send for MyOnewire {}
unsafe impl Sync for MyOnewire {}

pub struct MyState {
    pub config: MyConfig,
    pub ota_slot: String,

    pub api_cnt: AtomicU32,
    pub wifi_up: RwLock<bool>,
    pub ntp_ok: RwLock<bool>,
    pub if_index: RwLock<u32>,
    pub ip_addr: RwLock<net::Ipv4Addr>,
    pub ping_ip: RwLock<Option<net::Ipv4Addr>>,
    pub myid: RwLock<String>,
    pub sensors: RwLock<Vec<MyOnewire>>,
    pub data: RwLock<TempValues>,
    pub fresh_data: RwLock<bool>,
    pub nvs: RwLock<nvs::EspNvs<nvs::NvsDefault>>,
    pub reset: RwLock<bool>,
}

impl MyState {
    pub fn new(
        config: MyConfig,
        nvs: nvs::EspNvs<nvs::NvsDefault>,
        ota_slot: String,
        onewire_pins: Vec<MyOnewire>,
        temp_data: TempValues,
    ) -> Self {
        MyState {
            config,
            ota_slot,
            api_cnt: 0.into(),
            wifi_up: RwLock::new(false),
            ntp_ok: RwLock::new(false),
            if_index: RwLock::new(0),
            ip_addr: RwLock::new(net::Ipv4Addr::new(0, 0, 0, 0)),
            ping_ip: RwLock::new(None),
            myid: RwLock::new("esp32temp".into()),
            sensors: RwLock::new(onewire_pins),
            data: RwLock::new(temp_data),
            fresh_data: RwLock::new(false),
            nvs: RwLock::new(nvs),
            reset: RwLock::new(false),
        }
    }
}
// EOF
