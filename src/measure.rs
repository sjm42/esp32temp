// measure.rs

use crate::*;

const DS18B20_FAMILY_CODE: u8 = 0x28;

#[repr(u8)]
enum Ds18b20Command {
    ConvertTemp = 0x44,
    WriteScratchpad = 0x4E,
    ReadScratchpad = 0xBE,
}

#[repr(u8)]
#[derive(Copy, Clone)]
enum MeasureResolution {
    TC = 0b0111_1111,
}

impl MeasureResolution {
    const fn time_ms(self) -> u16 {
        match self {
            MeasureResolution::TC => 750,
        }
    }
}

#[derive(Debug)]
pub struct Measurement {
    pub device_id: String,
    pub temperature: f32,
}

#[derive(Debug)]
pub struct ScanResult {
    pub all_devices: Vec<OWAddress>,
    pub ds18b20_devices: Vec<OWAddress>,
}

pub fn format_device_id(device: &OWAddress) -> String {
    format!(
        "{:016X}",
        u64::from_be_bytes(device.address().to_le_bytes())
    )
}

pub async fn measure_temperatures(
    one_wire_bus: &OWDriver<'_>,
    devices: &[OWAddress],
    max_retry: u32,
) -> anyhow::Result<Vec<Measurement>> {
    let mut meas = Vec::new();

    for device in devices.iter() {
        let device_id = format_device_id(device);
        let result = Box::pin(async {
            set_resolution(one_wire_bus, device, MeasureResolution::TC)?;
            sleep(Duration::from_millis(50)).await;
            let wait_ms =
                start_temperature_measurement(one_wire_bus, device, MeasureResolution::TC)?;
            sleep(Duration::from_millis(u64::from(wait_ms) + 10)).await;

            for retry in 0..=max_retry {
                match read_temperature(one_wire_bus, device) {
                    Ok(temperature) => return Ok(temperature),
                    Err(error) if retry == max_retry => return Err(error),
                    Err(error) => warn!("Sensor {device_id} read error, retry {retry}: {error:#}"),
                }
                sleep(Duration::from_millis(100)).await;
            }
            bail!("Sensor read attempts exhausted")
        })
        .await;
        match result {
            Ok(temperature) => {
                let measurement = Measurement {
                    device_id,
                    temperature,
                };
                info!("Got measurement: {measurement:?}");
                meas.push(measurement);
            }
            Err(error) => warn!("Sensor {device_id} measurement failed: {error:#}"),
        }

        sleep(Duration::from_millis(100)).await;
    }

    if meas.is_empty() {
        bail!("No DS18B20 measurements succeeded");
    } else {
        Ok(meas)
    }
}

pub fn scan_1wire(one_wire_bus: &mut OWDriver<'_>) -> anyhow::Result<ScanResult> {
    let mut all_devices = Vec::new();
    let mut ds18b20_devices = Vec::new();

    for device in one_wire_bus.search()? {
        let device = device?;
        if all_devices.contains(&device) {
            continue;
        }
        if device.family_code() == DS18B20_FAMILY_CODE {
            ds18b20_devices.push(device);
        }
        all_devices.push(device);
    }

    Ok(ScanResult {
        all_devices,
        ds18b20_devices,
    })
}

pub async fn poll_sensors(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    if state.ap_mode {
        info!("Sensor polling is disabled in AP mode.");
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    }

    let ntp = sntp::EspSntp::new_default()?;
    info!("Waiting for NTP before polling sensors");
    while Utc::now().year() <= 2020 || ntp.get_sync_status() != sntp::SyncStatus::Completed {
        sleep(Duration::from_secs(1)).await;
    }
    *state.ntp_ok.write().await = true;
    info!("NTP synchronized; starting sensor polling");

    let poll_delay = state.config.delay;
    let max_retry = state.config.retries;
    loop {
        info!("Polling 1-wire sensors");
        state.led_on().await?;

        let mut temperatures = state
            .sensor_inventory
            .iter()
            .map(|sensor| TempData {
                iopin: sensor.iopin.clone(),
                sensor: sensor.sensor.clone(),
                value: NO_TEMP,
            })
            .collect::<Vec<_>>();
        {
            let mut onewires = state.sensors.write().await;
            for onew in onewires.iter_mut() {
                let result = match OWDriver::new(unsafe { onew.pin.reborrow() }) {
                    Ok(driver) => {
                        Box::pin(measure_temperatures(&driver, &onew.ids, max_retry)).await
                    }
                    Err(error) => Err(error.into()),
                };
                match result {
                    Ok(measurements) => {
                        for measurement in measurements {
                            if let Some(entry) = temperatures.iter_mut().find(|entry| {
                                entry.iopin == onew.name && entry.sensor == measurement.device_id
                            }) {
                                entry.value = measurement.temperature;
                            }
                        }
                    }
                    Err(error) => warn!("Temperature read failed on {}: {error:#}", onew.name),
                }
                sleep(Duration::from_millis(100)).await;
            }
        }
        let now = Utc::now();
        let synchronized = now.year() > 2020;
        if synchronized && !*state.ntp_ok.read().await {
            info!("NTP time available");
            *state.ntp_ok.write().await = true;
        }
        {
            let mut data = state.data.write().await;
            data.temperatures = temperatures;
            data.timestamp = if synchronized { now.timestamp() } else { 0 };
            data.last_update = if synchronized {
                now.to_rfc2822()
            } else {
                "-".into()
            };
        }
        *state.fresh_data.write().await = true;
        state.led_off().await?;

        sleep(Duration::from_secs(poll_delay)).await;
    }
}

fn start_temperature_measurement(
    one_wire_bus: &OWDriver<'_>,
    device: &OWAddress,
    resolution: MeasureResolution,
) -> anyhow::Result<u16> {
    one_wire_bus.reset()?;
    send_command(one_wire_bus, device, Ds18b20Command::ConvertTemp as u8)?;
    Ok(resolution.time_ms())
}

fn read_temperature(one_wire_bus: &OWDriver<'_>, device: &OWAddress) -> anyhow::Result<f32> {
    let scratchpad = read_scratchpad(one_wire_bus, device)?;
    let raw = i16::from_le_bytes([scratchpad[0], scratchpad[1]]);
    let temperature = f32::from(raw) / 16.0;
    anyhow::ensure!(
        (-55.0..=125.0).contains(&temperature),
        "DS18B20 temperature out of range: {temperature}"
    );
    Ok(temperature)
}

fn set_resolution(
    one_wire_bus: &OWDriver<'_>,
    device: &OWAddress,
    resolution: MeasureResolution,
) -> anyhow::Result<()> {
    let scratchpad = read_scratchpad(one_wire_bus, device)?;

    one_wire_bus.reset()?;
    send_bytes(
        one_wire_bus,
        device,
        &[
            Ds18b20Command::WriteScratchpad as u8,
            scratchpad[2],
            scratchpad[3],
            resolution as u8,
        ],
    )?;

    Ok(())
}

fn read_scratchpad(one_wire_bus: &OWDriver<'_>, device: &OWAddress) -> anyhow::Result<[u8; 9]> {
    one_wire_bus.reset()?;
    send_command(one_wire_bus, device, Ds18b20Command::ReadScratchpad as u8)?;

    let mut scratchpad = [0u8; 9];
    one_wire_bus.read(&mut scratchpad)?;

    let computed = compute_crc8(&scratchpad[..8]);
    if computed != scratchpad[8] {
        bail!(
            "Scratchpad CRC mismatch for {}: computed=0x{computed:02X} expected=0x{:02X}",
            format_device_id(device),
            scratchpad[8]
        );
    }

    Ok(scratchpad)
}

fn send_command(one_wire_bus: &OWDriver<'_>, device: &OWAddress, cmd: u8) -> anyhow::Result<()> {
    send_bytes(one_wire_bus, device, &[cmd])
}

fn send_bytes(one_wire_bus: &OWDriver<'_>, device: &OWAddress, bytes: &[u8]) -> anyhow::Result<()> {
    let mut buf = [0u8; 16];
    let addr = device.address().to_le_bytes();

    anyhow::ensure!(
        bytes.len() <= buf.len() - 9,
        "1-Wire command payload too large"
    );
    buf[0] = OWCommand::MatchRom as u8;
    buf[1..9].copy_from_slice(&addr);
    buf[9..9 + bytes.len()].copy_from_slice(bytes);

    one_wire_bus.write(&buf[..9 + bytes.len()])?;
    Ok(())
}

fn compute_crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in data.iter().copied() {
        let mut byte = byte;
        for _ in 0..8 {
            let mix = (crc ^ byte) & 0x01;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0x8C;
            }
            byte >>= 1;
        }
    }
    crc
}
