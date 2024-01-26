// bin/esp32ircbot.rs

use esp32temp::*;

use embedded_svc::wifi::{ClientConfiguration, Configuration};
use esp_idf_hal::gpio::IOPin;
use esp_idf_hal::prelude::Peripherals;
use esp_idf_svc::hal::gpio;
use esp_idf_svc::wifi::{AsyncWifi, EspWifi};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition, timer::EspTaskTimerService,
};
use esp_idf_sys as _;
use esp_idf_sys::{esp, esp_app_desc, EspError};
use log::*;
use one_wire_bus::OneWire;
use std::sync::Arc;
use tokio::sync::RwLock;

esp_app_desc!();

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASS: &str = env!("WIFI_PASS");

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // eventfd is needed by our mio poll implementation.  Note you should set max_fds
    // higher if you have other code that may need eventfd.
    info!("Setting up eventfd...");
    #[allow(clippy::needless_update)]
    let config = esp_idf_sys::esp_vfs_eventfd_config_t {
        max_fds: 1,
        ..Default::default()
    };
    esp! { unsafe { esp_idf_sys::esp_vfs_eventfd_register(&config) } }?;

    info!("Setting up board...");
    let sysloop = EspSystemEventLoop::take()?;
    let timer = EspTaskTimerService::new()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    #[cfg(feature = "esp32-c3")]
    let onew_pins = [
        (pins.gpio0.downgrade(), "gpio0"),
        (pins.gpio1.downgrade(), "gpio1"),
        (pins.gpio2.downgrade(), "gpio2"),
        (pins.gpio3.downgrade(), "gpio3"),
        (pins.gpio4.downgrade(), "gpio4"),
        (pins.gpio5.downgrade(), "gpio5"),
        (pins.gpio6.downgrade(), "gpio6"),
        (pins.gpio7.downgrade(), "gpio7"),
        (pins.gpio8.downgrade(), "gpio8"),
        (pins.gpio9.downgrade(), "gpio9"),
        (pins.gpio10.downgrade(), "gpio10"),
    ];

    #[cfg(feature = "esp32s")]
    let onew_pins = [
        (pins.gpio4.downgrade(), "gpio4"),
        (pins.gpio16.downgrade(), "gpio16"),
        (pins.gpio17.downgrade(), "gpio17"),
        (pins.gpio18.downgrade(), "gpio18"),
        (pins.gpio19.downgrade(), "gpio19"),
        (pins.gpio21.downgrade(), "gpio21"),
        (pins.gpio22.downgrade(), "gpio22"),
        (pins.gpio23.downgrade(), "gpio23"),
        (pins.gpio25.downgrade(), "gpio25"),
        (pins.gpio26.downgrade(), "gpio26"),
        (pins.gpio27.downgrade(), "gpio27"),
        (pins.gpio32.downgrade(), "gpio32"),
        (pins.gpio33.downgrade(), "gpio33"),
    ];

    info!("Scanning 1-wire devices...");
    let mut n_sensors = 0;
    let mut onewire_pins = Vec::with_capacity(onew_pins.len());
    for (i, (mut pin, name)) in onew_pins.into_iter().enumerate() {
        let mut w = OneWire::new(gpio::PinDriver::input_output_od(&mut pin).unwrap()).unwrap();
        if let Ok(devs) = scan_1wire(&mut w) {
            drop(w);
            n_sensors += devs.len();
            info!("Onewire response[{i}]:\n{name} {devs:#?}");
            onewire_pins.push(MyOnewire {
                pin,
                name: name.to_string(),
                ids: devs,
            });
        }
    }

    // populate the temp_data structure
    let mut temp_data = TempValues::with_capacity(n_sensors);
    (0..n_sensors).for_each(|_| {
        temp_data.temperatures.push(TempData {
            iopin: "N/A".into(),
            sensor: "N/A".into(),
            value: -100.0,
        });
    });
    let state = MyState {
        cnt: RwLock::new(0),
        sensors: RwLock::new(onewire_pins),
        data: RwLock::new(temp_data),
    };
    let shared_state = Arc::new(state);

    info!("Initializing Wi-Fi...");
    let wifi = AsyncWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs))?,
        sysloop,
        timer.clone(),
    )?;

    info!("Starting async run loop");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let mut wifi_loop = WifiLoop { wifi };
            wifi_loop.configure().await?;
            wifi_loop.initial_connect().await?;

            tokio::spawn(api_server(shared_state.clone()));
            tokio::spawn(poll_sensors(shared_state));

            info!("Entering main Wi-Fi run loop...");
            wifi_loop.stay_connected().await
        })?;

    Ok(())
}

pub struct WifiLoop<'a> {
    wifi: AsyncWifi<EspWifi<'a>>,
}

impl<'a> WifiLoop<'a> {
    pub async fn configure(&mut self) -> Result<(), EspError> {
        info!("Setting Wi-Fi credentials...");
        self.wifi
            .set_configuration(&Configuration::Client(ClientConfiguration {
                ssid: WIFI_SSID.try_into().unwrap(),
                password: WIFI_PASS.try_into().unwrap(),
                ..Default::default()
            }))?;

        info!("Starting Wi-Fi driver...");
        self.wifi.start().await
    }

    pub async fn initial_connect(&mut self) -> Result<(), EspError> {
        self.do_connect_loop(true).await
    }

    pub async fn stay_connected(mut self) -> Result<(), EspError> {
        self.do_connect_loop(false).await
    }

    async fn do_connect_loop(&mut self, exit_after_first_connect: bool) -> Result<(), EspError> {
        let wifi = &mut self.wifi;
        loop {
            // Wait for disconnect before trying to connect again.  This loop ensures
            // we stay connected and is commonly missing from trivial examples as it's
            // way too difficult to showcase the core logic of an example and have
            // a proper Wi-Fi event loop without a robust async runtime.  Fortunately, we can do it
            // now!
            wifi.wifi_wait(|w| w.is_up(), None).await?;

            info!("Connecting to Wi-Fi...");
            wifi.connect().await?;

            info!("Waiting for association...");
            wifi.ip_wait_while(|w| w.is_up().map(|s| !s), None).await?;

            if exit_after_first_connect {
                return Ok(());
            }
        }
    }
}
// EOF
