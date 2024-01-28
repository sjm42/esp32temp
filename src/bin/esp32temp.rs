// bin/esp32ircbot.rs

use embedded_storage::ReadStorage;
use embedded_svc::wifi::{ClientConfiguration, Configuration};
use esp_idf_hal::{gpio::IOPin, prelude::Peripherals};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::gpio,
    nvs::EspDefaultNvsPartition,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, EspWifi},
};
use esp_idf_sys as _;
use esp_idf_sys::{esp, esp_app_desc, EspError};
use esp_storage::FlashStorage;
use log::*;
use one_wire_bus::OneWire;
use std::sync::Arc;
use tokio::{
    sync::RwLock,
    time::{sleep, Duration},
};

use esp32temp::*;

esp_app_desc!();

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

    // comment or uncomment these, if you encounter this boot error:
    // E (439) esp_image: invalid segment length 0xXXXX
    // this means that the code size is not 32bit aligned
    // and any small change to the code will likely fix it.
    info!("Hello.");
    info!("Starting up.");

    info!("Setting up board...");
    let sysloop = EspSystemEventLoop::take()?;
    let timer = EspTaskTimerService::new()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut flash = FlashStorage::new();
    println!("Flash size = {} KiB", flash.capacity() >> 10);

    #[cfg(feature = "reset_settings")]
    let config = {
        let c = MyConfig::default();
        c.to_flash(&mut flash)?;
        c
    };

    #[cfg(not(feature = "reset_settings"))]
    let config = match MyConfig::from_flash(&mut flash) {
        None => {
            error!("Could not read flash config, using defaults");
            let c = MyConfig::default();
            c.to_flash(&mut flash)?;
            info!("Successfully saved default config to flash.");
            c
        }

        // we use settings saved on flash if we find them
        Some(c) => c,
    };

    info!("My config:\n{config:#?}");

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    #[cfg(feature = "esp32c3")]
    let onew_pins = Box::new([
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
    ]);

    #[cfg(feature = "esp32s")]
    let onew_pins = Box::new([
        (pins.gpio4.downgrade(), "gpio4"),
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
    ]);

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
            value: -1000.0,
        });
    });
    let state = MyState {
        config: RwLock::new(config),
        cnt: RwLock::new(0),
        sensors: RwLock::new(onewire_pins),
        data: RwLock::new(temp_data),
        flash: RwLock::new(flash),
        reset: RwLock::new(false),
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
            wifi_loop.configure(shared_state.clone()).await?;
            if let Err(e) = wifi_loop.initial_connect().await {
                error!("WiFi connection failed: {e:?}");
                {
                    let mut flash = shared_state.flash.write().await;
                    let mut config = shared_state.config.write().await;
                    let max = config.boot_fail_max;
                    let cnt = &mut config.boot_fail_cnt;
                    if *cnt > max {
                        error!("Maximum boot fails. Resetting settings to default.");
                        let c = MyConfig::default();
                        if c.to_flash(&mut flash).is_ok() {
                            info!("Successfully saved default config to flash.");
                        }
                    } else {
                        *cnt += 1;
                        config.to_flash(&mut flash).ok();
                    }
                }
                error!("Resetting...");
                sleep(Duration::from_secs(5)).await;
                esp_idf_hal::reset::restart();
            }

            tokio::spawn(housekeeping(shared_state.clone()));
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
    pub async fn configure(&mut self, state: Arc<MyState>) -> Result<(), EspError> {
        info!("Setting Wi-Fi credentials...");
        self.wifi
            .set_configuration(&Configuration::Client(ClientConfiguration {
                ssid: state
                    .config
                    .read()
                    .await
                    .wifi_ssid
                    .as_str()
                    .try_into()
                    .unwrap(),
                password: state
                    .config
                    .read()
                    .await
                    .wifi_pass
                    .as_str()
                    .try_into()
                    .unwrap(),
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

    async fn do_connect_loop(&mut self, initial: bool) -> Result<(), EspError> {
        let wifi = &mut self.wifi;
        loop {
            // Wait for disconnect before trying to connect again.  This loop ensures
            // we stay connected and is commonly missing from trivial examples as it's
            // way too difficult to showcase the core logic of an example and have
            // a proper Wi-Fi event loop without a robust async runtime.  Fortunately, we can do it
            // now!
            wifi.wifi_wait(|w| w.is_up(), Some(Duration::from_secs(30)))
                .await
                .ok();

            info!("Connecting to Wi-Fi...");
            wifi.connect().await.ok();

            info!("Waiting for association...");
            match wifi
                .ip_wait_while(|w| w.is_up().map(|s| !s), Some(Duration::from_secs(30)))
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    // only exit here if this is initial connection
                    // otherwise, keep trying
                    if initial {
                        return Err(e);
                    }
                }
            }
            if initial {
                return Ok(());
            }
        }
    }
}

pub async fn housekeeping(state: Arc<MyState>) -> ! {
    loop {
        let mut doit = false;
        {
            let mut reset = state.reset.write().await;
            if *reset {
                *reset = false;
                doit = true;
            }
        }
        if doit {
            esp_idf_hal::reset::restart();
        }

        sleep(Duration::from_secs(10)).await;
    }
}

// EOF
