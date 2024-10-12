// bin/esp32temp.rs

#![warn(clippy::large_futures)]

use std::{sync::Arc, time::Duration};

use esp32temp::*;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{AnyInputPin, IOPin, Input, InputPin, PinDriver, Pull};
use esp_idf_hal::prelude::Peripherals;
use esp_idf_svc::{eventloop::EspSystemEventLoop, hal::gpio, nvs, ping, timer::EspTaskTimerService,
                  wifi::WifiDriver};
use esp_idf_sys::{esp, esp_app_desc};
use log::*;
use one_wire_bus::OneWire;
use tokio::time::sleep;

const CONFIG_RESET_COUNT: i32 = 9;


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
    let button = gpio::PinDriver::input(pins.gpio9.downgrade_input())?;

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
        let mut pin_drv = gpio::PinDriver::input_output_od(&mut pin).unwrap();
        pin_drv.set_pull(Pull::Up).unwrap();
        let mut w = OneWire::new(pin_drv).unwrap();
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
            value: NO_TEMP,
        });
    });

    let wifidriver = WifiDriver::new(
        peripherals.modem,
        sysloop.clone(),
        Some(nvs_default_partition),
    )?;

    let state = Box::pin(MyState::new(config, onewire_pins, temp_data, nvs));
    let shared_state = Arc::new(state);

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(Box::pin(async move {
            let wifi_loop = WifiLoop {
                state: shared_state.clone(),
                wifi: None,
            };

            info!("Entering main loop...");
            tokio::select! {
                _ = Box::pin(poll_reset(shared_state.clone(), button)) => { error!("poll_reset() ended."); }
                _ = Box::pin(poll_sensors(shared_state.clone())) => { error!("poll_sensors() ended."); }
                _ = Box::pin(run_mqtt(shared_state.clone())) => { error!("run_mqtt() ended."); }
                _ = Box::pin(run_api_server(shared_state.clone())) => { error!("run_api_server() ended."); }
                _ = Box::pin(wifi_loop.run(wifidriver, sysloop, timer)) => { error!("wifi_loop.run() ended."); }
                _ = Box::pin(pinger(shared_state.clone())) => { error!("pinger() ended."); }
            };
        }));

    // not actually returning from main() but we reboot instead
    info!("main() finished, reboot.");
    FreeRtos::delay_ms(3000);
    esp_idf_hal::reset::restart();
}

async fn poll_reset(mut state: Arc<Pin<Box<MyState>>>, button: PinDriver<'_, AnyInputPin, Input>) -> anyhow::Result<()> {
    let mut uptime: usize = 0;
    loop {
        sleep(Duration::from_secs(2)).await;

        uptime += 2;
        *(state.uptime.write().await) = uptime;

        if *state.reset.read().await {
            esp_idf_hal::reset::restart();
        }

        if button.is_low() {
            Box::pin(reset_button(&mut state, &button)).await?;
        }
    }
}

async fn reset_button<'a, 'b>(
    state: &mut Arc<std::pin::Pin<Box<MyState>>>,
    button: &PinDriver<'a, AnyInputPin, Input>,
) -> anyhow::Result<()> {
    let mut reset_cnt = CONFIG_RESET_COUNT;

    while button.is_low() {
        // button is pressed and kept down, countdown and factory reset if reach zero
        let msg = format!("Reset? {reset_cnt}");
        error!("{msg}");

        if reset_cnt == 0 {
            // okay do factory reset now
            error!("Factory resetting...");

            let new_config = MyConfig::default();
            new_config.to_nvs(&mut *state.nvs.write().await)?;
            sleep(Duration::from_millis(2000)).await;
            esp_idf_hal::reset::restart();
        }

        reset_cnt -= 1;
        sleep(Duration::from_millis(500)).await;
        continue;
    }
    Ok(())
}

async fn pinger(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    loop {
        sleep(Duration::from_secs(300)).await;

        if let Some(ping_ip) = *state.ping_ip.read().await {
            let if_idx = *state.if_index.read().await;
            if if_idx > 0 {
                info!("Starting ping {ping_ip} (if_idx {if_idx})");
                let conf = ping::Configuration {
                    count: 3,
                    interval: Duration::from_secs(1),
                    timeout: Duration::from_secs(1),
                    data_size: 64,
                    tos: 0,
                };
                let mut ping = ping::EspPing::new(if_idx);
                let res = ping.ping(ping_ip, &conf)?;
                info!("Pinger result: {res:?}");
                if res.received == 0 {
                    error!("Ping failed, rebooting.");
                    sleep(Duration::from_millis(2000)).await;
                    esp_idf_hal::reset::restart();
                }
            } else {
                error!("No if_index. wat?");
            }
        }
    }
}

// EOF
