// rmt_ow.rs
//
// Thin local wrapper around the ESP-IDF `onewire_bus` RMT backend.
// This is derived from `esp-idf-hal::onewire`, but keeps the wrapper local
// so we can enable `onewire_bus_config_t.flags.en_pull_up`, which the HAL
// wrapper does not currently expose.

use core::{marker::PhantomData, ptr};

use esp_idf_sys::*;

use crate::*;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct OWAddress(u64);

impl OWAddress {
    pub const fn address(&self) -> u64 {
        self.0
    }

    pub const fn family_code(&self) -> u8 {
        (self.0 & 0xFF) as u8
    }
}

pub struct DeviceSearch<'a, 'b> {
    search: onewire_device_iter_handle_t,
    _bus: &'a mut OWDriver<'b>,
}

impl<'a, 'b> DeviceSearch<'a, 'b> {
    fn new(bus: &'a mut OWDriver<'b>) -> Result<Self, EspError> {
        let mut search: onewire_device_iter_handle_t = ptr::null_mut();
        esp!(unsafe { onewire_new_device_iter(bus.handle(), &mut search) })?;

        Ok(Self { search, _bus: bus })
    }

    fn next_device(&mut self) -> Result<OWAddress, EspError> {
        let mut device = onewire_device_t::default();
        esp!(unsafe { onewire_device_iter_get_next(self.search, &mut device) })?;
        Ok(OWAddress(device.address))
    }
}

impl Iterator for DeviceSearch<'_, '_> {
    type Item = Result<OWAddress, EspError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_device() {
            Ok(addr) => Some(Ok(addr)),
            Err(err) if err.code() == ESP_ERR_NOT_FOUND => None,
            Err(err) => Some(Err(err)),
        }
    }
}

impl Drop for DeviceSearch<'_, '_> {
    fn drop(&mut self) {
        esp!(unsafe { onewire_del_device_iter(self.search) }).unwrap();
    }
}

#[derive(Debug)]
pub struct OWDriver<'a> {
    handle: onewire_bus_handle_t,
    _pin: PhantomData<&'a mut ()>,
}

impl<'a> OWDriver<'a> {
    pub fn new(pin: impl gpio::Pin + 'a) -> Result<Self, EspError> {
        let mut flags = onewire_bus_config_t_onewire_bus_config_flags::default();
        flags.set_en_pull_up(1);

        let bus_config = onewire_bus_config_t {
            bus_gpio_num: pin.pin() as _,
            flags,
        };
        let rmt_config = onewire_bus_rmt_config_t { max_rx_bytes: 10 };

        let mut handle: onewire_bus_handle_t = ptr::null_mut();
        esp!(unsafe { onewire_new_bus_rmt(&bus_config, &rmt_config, &mut handle) })?;

        Ok(Self {
            handle,
            _pin: PhantomData,
        })
    }

    pub const fn handle(&self) -> onewire_bus_handle_t {
        self.handle
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<(), EspError> {
        esp!(unsafe { onewire_bus_read_bytes(self.handle(), buf.as_mut_ptr(), buf.len()) })?;
        Ok(())
    }

    pub fn write(&self, data: &[u8]) -> Result<(), EspError> {
        esp!(unsafe { onewire_bus_write_bytes(self.handle(), data.as_ptr(), data.len() as u8) })?;
        Ok(())
    }

    pub fn reset(&self) -> Result<(), EspError> {
        esp!(unsafe { onewire_bus_reset(self.handle()) })
    }

    pub fn search(&mut self) -> Result<DeviceSearch<'_, 'a>, EspError> {
        DeviceSearch::new(self)
    }
}

impl Drop for OWDriver<'_> {
    fn drop(&mut self) {
        esp!(unsafe { onewire_bus_del(self.handle()) }).unwrap();
    }
}

unsafe impl Send for OWDriver<'_> {}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum OWCommand {
    Search = 0xF0,
    MatchRom = 0x55,
    SkipRom = 0xCC,
    ReadRom = 0x33,
    SearchAlarm = 0xEC,
    ReadPowerSupply = 0xB4,
}
