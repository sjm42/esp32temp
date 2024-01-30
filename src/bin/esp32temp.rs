// bin/esp32ircbot.rs
#![warn(clippy::large_futures)]

use anyhow::bail;
use embedded_svc::wifi::{ClientConfiguration, Configuration};
use esp_idf_hal::{gpio::IOPin, prelude::Peripherals};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::gpio,
    ipv4,
    netif::{self, EspNetif},
    nvs,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, EspWifi, WifiDriver},
};
use esp_idf_sys::{self as _};
use esp_idf_sys::{esp, esp_app_desc};
use log::*;
use one_wire_bus::OneWire;
use std::{pin::Pin, sync::Arc};
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

    let sysloop = EspSystemEventLoop::take()?;
    let timer = EspTaskTimerService::new()?;
    let nvs_default_partition = nvs::EspDefaultNvsPartition::take()?;

    let ns = env!("CARGO_BIN_NAME");
    let mut nvs = match nvs::EspNvs::new(nvs_default_partition.clone(), ns, true) {
        Ok(nvs) => {
            info!("Got namespace {ns:?} from default partition");
            nvs
        }
        Err(e) => panic!("Could not get namespace {ns}: {e:?}"),
    };

    #[cfg(feature = "reset_settings")]
    let config = {
        let c = MyConfig::default();
        c.to_nvs(&mut nvs)?;
        c
    };

    #[cfg(not(feature = "reset_settings"))]
    let config = match MyConfig::from_nvs(&mut nvs) {
        None => {
            error!("Could not read nvs config, using defaults");
            let c = MyConfig::default();
            c.to_nvs(&mut nvs)?;
            info!("Successfully saved default config to nvs.");
            c
        }

        // using settings saved on nvs if we could find them
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

    info!("Initializing Wi-Fi...");

    let ipv4_config = if config.v4dhcp {
        ipv4::ClientConfiguration::DHCP(ipv4::DHCPClientSettings::default())
    } else {
        ipv4::ClientConfiguration::Fixed(ipv4::ClientSettings {
            ip: config.v4addr,
            subnet: ipv4::Subnet {
                gateway: config.v4gw,
                mask: ipv4::Mask(config.v4mask),
            },
            dns: None,
            secondary_dns: None,
        })
    };
    // info!("IP config: {ipv4_config:?}");

    let net_if = EspNetif::new_with_conf(&netif::NetifConfiguration {
        ip_configuration: ipv4::Configuration::Client(ipv4_config),
        ..netif::NetifConfiguration::wifi_default_client()
    })?;

    let wifidriver = WifiDriver::new(
        peripherals.modem,
        sysloop.clone(),
        Some(nvs_default_partition),
    )?;
    let espwifi = EspWifi::wrap_all(wifidriver, net_if, EspNetif::new(netif::NetifStack::Ap)?)?;
    let wifi = AsyncWifi::wrap(espwifi, sysloop, timer.clone())?;

    let state = Box::pin(MyState {
        config: RwLock::new(config),
        cnt: RwLock::new(0),
        sensors: RwLock::new(onewire_pins),
        data: RwLock::new(temp_data),
        nvs: RwLock::new(nvs),
        reset: RwLock::new(false),
    });
    let shared_state = Arc::new(state);

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(Box::pin(async move {
            let mut wifi_loop = WifiLoop { wifi };
            Box::pin(wifi_loop.configure(shared_state.clone())).await?;

            if let Err(e) = Box::pin(wifi_loop.initial_connect()).await {
                error!("WiFi connection failed: {e:?}");
                {
                    // failed boot, increase boot fail counter or reset "factory" settings

                    let mut nvs = shared_state.nvs.write().await;
                    let mut config = shared_state.config.write().await;

                    let cnt = &mut config.bfc;
                    if *cnt > BOOT_FAIL_MAX {
                        error!("Maximum boot fails. Resetting settings to default.");
                        let c = MyConfig::default();
                        if c.to_nvs(&mut nvs).is_ok() {
                            info!("Successfully saved default config to nvs.");
                        }
                    } else {
                        *cnt += 1;
                        config.to_nvs(&mut nvs).ok();
                    }
                }
                error!("Resetting...");
                sleep(Duration::from_secs(5)).await;
                esp_idf_hal::reset::restart();
            }

            // Successful startup, wifi connected: reset fail counter.
            {
                let mut config = shared_state.config.write().await;
                let cnt = &mut config.bfc;
                if *cnt > 0 {
                    info!("Successful startup, resetting boot fail counter.");
                    *cnt = 0;
                    let mut nvs = shared_state.nvs.write().await;
                    if config.to_nvs(&mut nvs).is_ok() {
                        info!("Successfully saved config to nvs.");
                    }
                }
            }

            info!("Entering main loop...");
            let myname = env!("CARGO_BIN_NAME").into();
            tokio::select! {
                _ = Box::pin(mqtt_sender(shared_state.clone(), myname)) => {}
                _ = Box::pin(api_server(shared_state.clone())) => {}
                _ = Box::pin(poll_sensors(shared_state)) => {}
                _ = Box::pin(wifi_loop.stay_connected()) => {}
            };
            Ok(())
        }))
}

pub struct WifiLoop<'a> {
    wifi: AsyncWifi<EspWifi<'a>>,
}

impl<'a> WifiLoop<'a> {
    pub async fn configure(&mut self, state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
        info!("WiFi setting credentials...");
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

        info!("WiFi driver starting...");
        Ok(Box::pin(self.wifi.start()).await?)
    }

    pub async fn initial_connect(&mut self) -> anyhow::Result<()> {
        self.do_connect_loop(true).await
    }

    pub async fn stay_connected(mut self) -> anyhow::Result<()> {
        self.do_connect_loop(false).await
    }

    async fn do_connect_loop(&mut self, initial: bool) -> anyhow::Result<()> {
        let wifi = &mut self.wifi;
        loop {
            // Wait for disconnect before trying to connect again.  This loop ensures
            // we stay connected and is commonly missing from trivial examples as it's
            // way too difficult to showcase the core logic of an example and have
            // a proper Wi-Fi event loop without a robust async runtime.  Fortunately, we can do it
            // now!
            let timeout = if initial {
                Some(Duration::from_secs(30))
            } else {
                None
            };
            Box::pin(wifi.wifi_wait(|w| w.is_up(), timeout)).await.ok();

            info!("WiFi connecting...");
            Box::pin(wifi.connect()).await.ok();

            info!("WiFi waiting for association...");
            match Box::pin(wifi.ip_wait_while(|w| w.is_up().map(|s| !s), None)).await {
                Ok(_) => {}
                Err(e) => {
                    error!("WiFi error: {e:?}");

                    // only exit here if this is initial connection
                    // otherwise, keep trying
                    if initial {
                        bail!(e);
                    }
                }
            }

            info!("WiFi connected.");
            if initial {
                return Ok(());
            }
        }
    }
}

// EOF
