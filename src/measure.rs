// measure.rs

use embedded_hal::digital::v2::{InputPin, OutputPin};
use esp_idf_hal::{
    delay::{Ets, FreeRtos},
    gpio,
};
use log::*;
use one_wire_bus::{Address, OneWire, OneWireError, SearchState};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::*;

#[derive(Debug)]
pub struct Measurement {
    pub device_id: String,
    pub temperature: f32,
}

pub async fn measure_temperatures<P, E>(
    one_wire_bus: &mut OneWire<P>,
    addrs: &[Address],
    max_retry: u32,
) -> Result<Vec<Measurement>, MeasurementError<E>>
where
    P: OutputPin<Error = E> + InputPin<Error = E>,
    E: std::fmt::Debug,
{
    let mut meas = Vec::new();

    for a in addrs.iter() {
        let sensor = ds18b20::Ds18b20::new::<E>(a.to_owned())?;
        sensor.set_config(
            i8::MIN,
            i8::MAX,
            ds18b20::Resolution::Bits12,
            one_wire_bus,
            &mut Ets,
        )?;
        sleep(Duration::from_millis(50)).await; // extra sleep
        sensor.start_temp_measurement(one_wire_bus, &mut Ets)?;
        ds18b20::Resolution::Bits12.delay_for_measurement_time(&mut FreeRtos);
        sleep(Duration::from_millis(10)).await; // extra sleep

        // Quite often we have to retry, CrcMismatch is observed occasionally
        let mut retries = 0;
        loop {
            match sensor.read_data(one_wire_bus, &mut Ets) {
                Ok(data) => {
                    let m = Measurement {
                        device_id: format!("{:?}", a),
                        temperature: data.temperature,
                    };
                    info!("Got meas, retry#{retries}: {m:?}");
                    meas.push(m);
                    break;
                }
                Err(e) => {
                    retries += 1;
                    error!("Sensor {a:?} read error: {e:?}");
                    if retries > max_retry {
                        break;
                    }
                }
            }
            sleep(Duration::from_millis(100)).await; // extra sleep
        }
        // let sensor_data = ?;
        sleep(Duration::from_millis(100)).await;
    }

    if meas.is_empty() {
        Err(MeasurementError::NoDeviceFound)
    } else {
        Ok(meas)
    }
}

pub fn scan_1wire<P, E>(one_wire_bus: &mut OneWire<P>) -> Result<Vec<Address>, MeasurementError<E>>
where
    P: OutputPin<Error = E> + InputPin<Error = E>,
{
    let mut devices = Vec::new();
    let mut st: SearchState;
    let mut state = None;

    loop {
        match one_wire_bus.device_search(state, false, &mut Ets)? {
            None => {
                break;
            }
            Some((device_address, s)) => {
                devices.push(device_address);
                st = s;
                state = Some(&st);
            }
        }
    }

    if devices.is_empty() {
        Err(MeasurementError::NoDeviceFound)
    } else {
        Ok(devices)
    }
}

// When performing a measurement it can happen that no device was found on the one-wire-bus
// in addition to the bus errors. Therefore we extend the error cases for proper error handling.
#[derive(Debug)]
pub enum MeasurementError<E> {
    OneWireError(OneWireError<E>),
    NoDeviceFound,
}

impl<E> From<OneWireError<E>> for MeasurementError<E> {
    fn from(value: OneWireError<E>) -> Self {
        MeasurementError::OneWireError(value)
    }
}

pub async fn poll_sensors(state: Arc<Pin<Box<MyState>>>) -> anyhow::Result<()> {
    // sleep(Duration::from_secs(10)).await;
    let poll_delay = state.config.read().await.delay;
    let max_retry = state.config.read().await.retries;
    loop {
        info!("Polling 1-wire sensors");
        let mut do_reset = false;

        {
            let mut reset = state.reset.write().await;
            if *reset {
                *reset = false;
                do_reset = true;
            }
        }

        {
            let mut onewires = state.sensors.write().await;
            let mut data = state.data.write().await;
            let mut i = 0;
            for onew in onewires.iter_mut() {
                let mut w =
                    OneWire::new(gpio::PinDriver::input_output_od(&mut onew.pin).unwrap()).unwrap();
                match Box::pin(measure_temperatures(&mut w, &onew.ids, max_retry)).await {
                    Ok(meas) => {
                        info!("Onewire response {name}:\n{meas:#?}", name = onew.name);
                        for m in meas.into_iter() {
                            data.temperatures[i] = TempData {
                                iopin: onew.name.clone(),
                                sensor: m.device_id,
                                value: m.temperature,
                            };
                            i += 1;
                        }
                    }
                    Err(e) => {
                        error!("Temp read error: {e:#?}");
                        // cannot continue measure cycle, index gets out of sync
                        break;
                    }
                }
                drop(w);
                sleep(Duration::from_millis(100)).await; // extra sleep
            }
        }

        if do_reset {
            esp_idf_hal::reset::restart();
        }

        sleep(Duration::from_secs(poll_delay)).await;
    }
}

// EOF
