// measure.rs

use embedded_hal::digital::v2::{InputPin, OutputPin};
use esp_idf_hal::delay::{Ets, FreeRtos};
use one_wire_bus::{OneWire, OneWireError};

#[derive(Debug)]
pub struct Measurement {
    pub device_id: String,
    pub temperature: f32,
}
pub fn measure_temperature<P, E>(
    one_wire_bus: &mut OneWire<P>,
) -> Result<Measurement, MeasurementError<E>>
where
    P: OutputPin<Error = E> + InputPin<Error = E>,
{
    ds18b20::start_simultaneous_temp_measurement(one_wire_bus, &mut Ets)?;
    ds18b20::Resolution::Bits12.delay_for_measurement_time(&mut FreeRtos);
    FreeRtos::delay_ms(100);

    if let Some((device_address, _)) = one_wire_bus.device_search(None, false, &mut Ets)? {
        let sensor = ds18b20::Ds18b20::new::<E>(device_address)?;
        let sensor_data = sensor.read_data(one_wire_bus, &mut Ets)?;
        return Ok(Measurement {
            device_id: format!("{:?}", device_address),
            temperature: sensor_data.temperature,
        });
    }

    Err(MeasurementError::NoDeviceFound)
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

// EOF
