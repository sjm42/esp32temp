// bin/esp32temp.rs

#![warn(clippy::large_futures)]

use esp_idf_svc::{
    eventloop::EspSystemEventLoop, hal::gpio, ota::EspOta, ping, timer::EspTaskTimerService,
};
use esp_idf_sys::esp;

use esp32temp::*;

const CONFIG_RESET_COUNT: i32 = 9;
const BUTTON_POLL_MS: u64 = 500;
const BUTTON_BLINK_MS: u64 = 500;
const BUTTON_COUNTDOWN_STEP_MS: u64 = 500;

// use esp_idf_sys::esp_app_desc;
// esp_app_desc!();

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
    info!("Starting up, firmare version {}", FW_VERSION);
    let ota_slot = {
        let mut ota = EspOta::new()?;
        let running_slot = ota.get_running_slot()?;
        ota.mark_running_slot_valid()?;
        let ota_slot = format!("{} ({:?})", &running_slot.label, running_slot.state);
        info!("OTA slot: {ota_slot}");
        ota_slot
    };

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
    let mut ap_mode = matches!(nvs.get_u8(AP_MODE_NVS_KEY)?, Some(1));
    if ap_mode {
        info!("One-shot AP mode requested for this boot.");
        let _ = nvs.remove(AP_MODE_NVS_KEY)?;
    }

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
    if !config.has_wifi_config() {
        ap_mode = true;
        info!("No WiFi configuration stored, starting in AP mode.");
    }
    info!("My config:\n{config:#?}");

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    #[cfg(feature = "esp32-c3")]
    let button = gpio::PinDriver::input(pins.gpio9.degrade_input(), Pull::Up)?;
    #[cfg(feature = "esp32-c3")]
    let led = gpio::PinDriver::output(pins.gpio8.degrade_output())?;

    #[cfg(feature = "esp-wroom-32")]
    let button = gpio::PinDriver::input(pins.gpio0.degrade_input(), Pull::Up)?;
    #[cfg(feature = "esp-wroom-32")]
    let led = gpio::PinDriver::output(pins.gpio2.degrade_output())?;

    #[cfg(feature = "esp32-c3")]
    let hw_onewire_pins = Box::new([
        (pins.gpio0.degrade_input_output(), "gpio0"),
        (pins.gpio1.degrade_input_output(), "gpio1"),
        (pins.gpio2.degrade_input_output(), "gpio2"),
        (pins.gpio3.degrade_input_output(), "gpio3"),
        (pins.gpio4.degrade_input_output(), "gpio4"),
        (pins.gpio5.degrade_input_output(), "gpio5"),
        (pins.gpio6.degrade_input_output(), "gpio6"),
        (pins.gpio7.degrade_input_output(), "gpio7"),
        (pins.gpio10.degrade_input_output(), "gpio10"),
    ]);

    #[cfg(feature = "esp-wroom-32")]
    let hw_onewire_pins = Box::new([
        (pins.gpio4.degrade_input_output(), "gpio4"),
        (pins.gpio18.degrade_input_output(), "gpio18"),
        (pins.gpio19.degrade_input_output(), "gpio19"),
        (pins.gpio21.degrade_input_output(), "gpio21"),
        (pins.gpio22.degrade_input_output(), "gpio22"),
        (pins.gpio23.degrade_input_output(), "gpio23"),
        (pins.gpio25.degrade_input_output(), "gpio25"),
        (pins.gpio26.degrade_input_output(), "gpio26"),
        (pins.gpio27.degrade_input_output(), "gpio27"),
        (pins.gpio32.degrade_input_output(), "gpio32"),
        (pins.gpio33.degrade_input_output(), "gpio33"),
    ]);

    info!("Scanning 1-wire devices...");
    let mut n_sensors = 0;
    let mut onewire_pins = Vec::with_capacity(hw_onewire_pins.len());
    for (i, (mut pin, name)) in hw_onewire_pins.into_iter().enumerate() {
        let mut w = OWDriver::new(unsafe { pin.reborrow() })?;
        match scan_1wire(&mut w) {
            Ok(scan) => {
                drop(w);

                if scan.all_devices.is_empty() {
                    info!("Onewire response[{i}]: {name} no devices");
                } else {
                    for device in scan.all_devices.iter() {
                        info!(
                            "Onewire response[{i}]: {name} device {} family=0x{:02X}",
                            format_device_id(device),
                            device.family_code(),
                        );
                    }
                }

                if !scan.ds18b20_devices.is_empty() {
                    n_sensors += scan.ds18b20_devices.len();
                    onewire_pins.push(MyOnewire {
                        pin,
                        name: name.to_string(),
                        ids: scan.ds18b20_devices,
                    });
                }
            }
            Err(e) => {
                drop(w);
                error!("Onewire scan error[{i}] {name}: {e:#}");
            }
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

    let state = Box::pin(MyState::new(
        ap_mode,
        config,
        nvs,
        ota_slot,
        onewire_pins,
        temp_data,
        led,
    ));
    let shared_state = Arc::new(state);

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(Box::pin(async move {
            shared_state.led_off().await.ok();
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
                _ = Box::pin(run_esphome_api(shared_state.clone())) => { error!("run_esphome_api() ended."); }
                _ = Box::pin(wifi_loop.run(wifidriver, sysloop, timer)) => { error!("wifi_loop.run() ended."); }
                _ = Box::pin(pinger(shared_state.clone())) => { error!("pinger() ended."); }
            };
        }));

    // not actually returning from main() but we reboot instead
    info!("main() finished, reboot.");
    FreeRtos::delay_ms(3000);
    esp_idf_hal::reset::restart();
}

async fn poll_reset(
    mut state: Arc<Pin<Box<MyState>>>,
    button: PinDriver<'static, Input>,
) -> anyhow::Result<()> {
    let mut uptime: u32 = 0;
    let mut uptime_ms: u64 = 0;
    loop {
        sleep(Duration::from_millis(BUTTON_POLL_MS)).await;
        uptime_ms += BUTTON_POLL_MS;
        if uptime_ms >= 1000 {
            let secs = (uptime_ms / 1000) as u32;
            uptime += secs;
            uptime_ms %= 1000;

            let mut data = state.data.write().await;
            data.uptime = uptime;
            data.uptime_s =
                humantime::format_duration(Duration::from_secs(uptime as u64)).to_string();
        }

        if *state.reset.read().await {
            esp_idf_hal::reset::restart();
        }

        if button.is_low() {
            Box::pin(reset_button(&mut state, &button)).await?;
        }
    }
}

async fn reset_button<'a>(
    state: &mut Arc<std::pin::Pin<Box<MyState>>>,
    button: &PinDriver<'a, Input>,
) -> anyhow::Result<()> {
    let mut reset_cnt = CONFIG_RESET_COUNT;
    let mut blink_on = true;
    let mut blink_elapsed_ms = 0;
    let mut countdown_elapsed_ms = 0;

    while button.is_low() {
        if countdown_elapsed_ms == 0 {
            let msg = format!("Reset? {reset_cnt}");
            error!("{msg}");

            if reset_cnt == 0 {
                error!("Factory resetting...");
                state.led_on().await?;

                let new_config = MyConfig::default();
                let mut nvs = state.nvs.write().await;
                new_config.to_nvs(&mut nvs)?;
                let _ = nvs.remove(AP_MODE_NVS_KEY)?;
                sleep(Duration::from_millis(2000)).await;
                esp_idf_hal::reset::restart();
            }

            reset_cnt -= 1;
        }

        if blink_elapsed_ms == 0 {
            state.set_led(blink_on).await?;
            blink_on = !blink_on;
        }

        sleep(Duration::from_millis(BUTTON_POLL_MS)).await;
        blink_elapsed_ms = (blink_elapsed_ms + BUTTON_POLL_MS) % BUTTON_BLINK_MS;
        countdown_elapsed_ms = (countdown_elapsed_ms + BUTTON_POLL_MS) % BUTTON_COUNTDOWN_STEP_MS;
    }
    state.led_off().await?;

    if !state.ap_mode {
        info!("Short button press, rebooting into AP mode for manual configuration.");
        state.request_ap_mode_on_next_boot().await?;
        sleep(Duration::from_millis(250)).await;
        esp_idf_hal::reset::restart();
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
