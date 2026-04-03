// state.rs

use crate::*;

pub const AP_MODE_NVS_KEY: &str = "boot_ap";

pub struct MyOnewire {
    pub pin: AnyIOPin<'static>,
    pub name: String,
    pub ids: Vec<OWAddress>,
}
unsafe impl Send for MyOnewire {}
unsafe impl Sync for MyOnewire {}

pub struct MyState {
    pub ap_mode: bool,
    pub config: MyConfig,
    pub ota_slot: String,

    pub api_cnt: AtomicU32,
    pub wifi_up: RwLock<bool>,
    pub ntp_ok: RwLock<bool>,
    pub if_index: RwLock<u32>,
    pub ip_addr: RwLock<net::Ipv4Addr>,
    pub ping_ip: RwLock<Option<net::Ipv4Addr>>,
    pub myid: RwLock<String>,
    pub my_mac_s: RwLock<String>,
    pub sensors: RwLock<Vec<MyOnewire>>,
    pub data: RwLock<TempValues>,
    pub fresh_data: RwLock<bool>,
    pub nvs: RwLock<nvs::EspNvs<nvs::NvsDefault>>,
    pub led: RwLock<PinDriver<'static, Output>>,
    pub reset: RwLock<bool>,
}

impl MyState {
    pub fn new(
        ap_mode: bool,
        config: MyConfig,
        nvs: nvs::EspNvs<nvs::NvsDefault>,
        ota_slot: String,
        onewire_pins: Vec<MyOnewire>,
        temp_data: TempValues,
        led: PinDriver<'static, Output>,
    ) -> Self {
        MyState {
            ap_mode,
            config,
            ota_slot,
            api_cnt: 0.into(),
            wifi_up: RwLock::new(false),
            ntp_ok: RwLock::new(false),
            if_index: RwLock::new(0),
            ip_addr: RwLock::new(net::Ipv4Addr::new(0, 0, 0, 0)),
            ping_ip: RwLock::new(None),
            myid: RwLock::new("esp32temp".into()),
            my_mac_s: RwLock::new("00:00:00:00:00:00".into()),
            sensors: RwLock::new(onewire_pins),
            data: RwLock::new(temp_data),
            fresh_data: RwLock::new(false),
            nvs: RwLock::new(nvs),
            led: RwLock::new(led),
            reset: RwLock::new(false),
        }
    }

    pub async fn set_led(&self, enabled: bool) -> anyhow::Result<()> {
        let mut led = self.led.write().await;
        if enabled != LED_ACTIVE_LOW {
            led.set_high()?;
        } else {
            led.set_low()?;
        }
        Ok(())
    }

    pub async fn led_on(&self) -> anyhow::Result<()> {
        self.set_led(true).await
    }

    pub async fn led_off(&self) -> anyhow::Result<()> {
        self.set_led(false).await
    }

    pub async fn request_ap_mode_on_next_boot(&self) -> anyhow::Result<()> {
        self.nvs.write().await.set_u8(AP_MODE_NVS_KEY, 1)?;
        Ok(())
    }
}
// EOF
