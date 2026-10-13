// bin/esp32temp.rs

#![warn(clippy::large_futures)]

use anyhow::Context;
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

fn main() {
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    if let Err(error) = run() {
        error!("Fatal application error: {error:#}");
    }
    info!("Application stopped, rebooting...");
    FreeRtos::delay_ms(3000);
    esp_idf_hal::reset::restart();
}

fn run() -> anyhow::Result<()> {
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
    info!("Starting up, firmware version {}", FW_VERSION);
    // The probe needs its one-shot failure request before application validation.
    #[cfg(feature = "ota-rollback-probe")]
    let nvs_default_partition = nvs::EspDefaultNvsPartition::take()?;
    #[cfg(feature = "ota-rollback-probe")]
    let mut nvs = nvs::EspNvs::new(nvs_default_partition.clone(), env!("CARGO_BIN_NAME"), true)?;
    let ota_details = {
        let mut ota = EspOta::new()?;
        let mut details = OtaDetails::capture();
        #[cfg(feature = "ota-rollback-probe")]
        ota_probe::on_boot(&mut details, &mut nvs)?;
        ota.mark_running_slot_valid()?;
        details.refresh_slots();
        info!("OTA details: {details:#?}");
        details
    };

    let sysloop = EspSystemEventLoop::take()?;
    let timer = EspTaskTimerService::new()?;
    #[cfg(not(feature = "ota-rollback-probe"))]
    let nvs_default_partition = nvs::EspDefaultNvsPartition::take()?;

    let ns = env!("CARGO_BIN_NAME");
    #[cfg(not(feature = "ota-rollback-probe"))]
    let mut nvs = nvs::EspNvs::new(nvs_default_partition.clone(), ns, true)
        .with_context(|| format!("Could not get NVS namespace {ns}"))?;
    info!("Got namespace {ns:?} from default partition");
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
    config.validate().context("Invalid stored configuration")?;
    info!("Configuration loaded (AP mode: {ap_mode})");

    let peripherals = Peripherals::take().context("Could not take peripherals")?;
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
    for onewire in &onewire_pins {
        for device in &onewire.ids {
            temp_data.temperatures.push(TempData {
                iopin: onewire.name.clone(),
                sensor: format_device_id(device),
                value: NO_TEMP,
            });
        }
    }

    let wifidriver = WifiDriver::new(
        peripherals.modem,
        sysloop.clone(),
        Some(nvs_default_partition),
    )?;

    let state = Box::pin(MyState::new(
        ap_mode,
        config,
        nvs,
        ota_details,
        onewire_pins,
        temp_data,
        led,
    ));
    let shared_state = Arc::new(state);

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        // OTA downloads and gateway pings use one bounded blocking worker.
        .max_blocking_threads(1)
        .thread_stack_size(20480)
        .build()?
        .block_on(Box::pin(async move {
            shared_state.led_off().await.ok();
            let wifi_loop = WifiLoop {
                state: shared_state.clone(),
            };

            info!("Entering main loop...");
            tokio::select! {
                result = Box::pin(poll_reset(shared_state.clone(), button)) => { error!("poll_reset() ended: {result:?}"); }
                result = Box::pin(poll_sensors(shared_state.clone())) => { error!("poll_sensors() ended: {result:?}"); }
                result = Box::pin(run_mqtt(shared_state.clone())) => { error!("run_mqtt() ended: {result:?}"); }
                result = Box::pin(run_api_server(shared_state.clone())) => { error!("run_api_server() ended: {result:?}"); }
                result = Box::pin(run_esphome_api(shared_state.clone())) => { error!("run_esphome_api() ended: {result:?}"); }
                result = Box::pin(wifi_loop.run(wifidriver, sysloop, timer)) => { error!("wifi_loop.run() ended: {result:?}"); }
                result = Box::pin(pinger(shared_state.clone())) => { error!("pinger() ended: {result:?}"); }
            };
        }));

    Ok(())
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

        if !*state.wifi_up.read().await {
            continue;
        }
        let generation = state.wifi_generation.load(Ordering::Relaxed);
        let ping_ip = *state.ping_ip.read().await;
        if let Some(ping_ip) = ping_ip {
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
                let res = tokio::task::spawn_blocking(move || {
                    ping::EspPing::new(if_idx).ping(ping_ip, &conf)
                })
                .await
                .context("Gateway ping worker failed")?;
                info!("Pinger result: {res:?}");
                if !matches!(res, Ok(ref summary) if summary.received > 0) {
                    sleep(Duration::from_millis(2000)).await;
                    if !*state.wifi_up.read().await
                        || state.wifi_generation.load(Ordering::Relaxed) != generation
                        || *state.ping_ip.read().await != Some(ping_ip)
                        || *state.if_index.read().await != if_idx
                    {
                        info!("Network changed during ping; leaving recovery to WiFi.");
                        continue;
                    }
                    error!("Ping failed on unchanged connection, rebooting.");
                    esp_idf_hal::reset::restart();
                }
            } else {
                warn!("No network interface available for gateway ping");
            }
        }
    }
}

// EOF
