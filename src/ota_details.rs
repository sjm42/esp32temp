//! OTA diagnostics captured at startup, before and after application validation.

use std::{ffi::CStr, fmt};

use esp_idf_sys as sys;

use crate::*;

#[derive(Clone, Debug, Serialize)]
pub struct OtaDetails {
    pub application_version: &'static str,
    pub application_idf_version: String,
    pub application_rollback_enabled: bool,
    pub bootloader: Option<BootloaderDetails>,
    pub bootloader_error: Option<String>,
    pub bootloader_rollback: BootloaderRollback,
    pub running_state_at_boot: OtaImageState,
    pub running_slot: Option<OtaSlotDetails>,
    pub boot_slot: Option<OtaSlotDetails>,
    pub update_slot: Option<OtaSlotDetails>,
    pub probe: Option<OtaProbeDetails>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OtaProbeDetails {
    pub message: String,
    pub can_test_fallback: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BootloaderDetails {
    pub version: u32,
    pub idf_version: String,
    pub built_at: String,
    pub secure_version: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct OtaSlotDetails {
    pub label: String,
    pub address: u32,
    pub size_bytes: u32,
    pub state: OtaImageState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum OtaImageState {
    New,
    PendingVerification,
    Valid,
    Invalid,
    Aborted,
    Undefined,
    NotRecorded,
    Unavailable,
}

impl fmt::Display for OtaImageState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::New => "New",
            Self::PendingVerification => "Pending verification",
            Self::Valid => "Valid",
            Self::Invalid => "Invalid",
            Self::Aborted => "Aborted",
            Self::Undefined => "Undefined",
            Self::NotRecorded => "Not recorded",
            Self::Unavailable => "Unavailable",
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum BootloaderRollback {
    Observed,
    NotObserved,
    Unknown,
}

impl fmt::Display for BootloaderRollback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Observed => "Confirmed by pending verification at boot",
            Self::NotObserved => "Not observed — probe remained NEW at boot",
            Self::Unknown => "Unknown — not reported by bootloader metadata",
        })
    }
}

impl OtaDetails {
    /// Read diagnostics while the startup code owns the OTA service.
    /// Missing metadata is diagnostic information, not a fatal startup error.
    pub fn capture() -> Self {
        // SAFETY: ESP-IDF returns a static, NUL-terminated version string.
        let application_idf_version = unsafe { CStr::from_ptr(sys::esp_get_idf_version()) }
            .to_string_lossy()
            .into_owned();
        let (bootloader, bootloader_error) = match BootloaderDetails::read() {
            Ok(details) => (Some(details), None),
            Err(error) => {
                warn!("Bootloader description unavailable: {error}");
                (None, Some(error.to_string()))
            }
        };
        let running_slot = read_slot(unsafe { sys::esp_ota_get_running_partition() });
        let running_state_at_boot = running_slot
            .as_ref()
            .map_or(OtaImageState::Unavailable, |slot| slot.state);
        // NEW can be written by an application even if the bootloader lacks rollback.
        // Only PENDING_VERIFY records the bootloader's transition on this boot.
        let bootloader_rollback = if running_state_at_boot == OtaImageState::PendingVerification {
            BootloaderRollback::Observed
        } else {
            BootloaderRollback::Unknown
        };

        Self {
            application_version: FW_VERSION,
            application_idf_version,
            application_rollback_enabled: cfg!(esp_idf_bootloader_app_rollback_enable),
            bootloader,
            bootloader_error,
            bootloader_rollback,
            running_state_at_boot,
            running_slot,
            boot_slot: read_slot(unsafe { sys::esp_ota_get_boot_partition() }),
            update_slot: read_slot(unsafe {
                sys::esp_ota_get_next_update_partition(std::ptr::null())
            }),
            probe: None,
        }
    }

    /// Refresh slot states after validation without discarding the initial boot state.
    pub fn refresh_slots(&mut self) {
        self.running_slot = read_slot(unsafe { sys::esp_ota_get_running_partition() });
        self.boot_slot = read_slot(unsafe { sys::esp_ota_get_boot_partition() });
        self.update_slot =
            read_slot(unsafe { sys::esp_ota_get_next_update_partition(std::ptr::null()) });
    }
}

impl BootloaderDetails {
    fn read() -> Result<Self, sys::EspError> {
        let mut desc = sys::esp_bootloader_desc_t::default();
        // SAFETY: desc is writable and NULL selects the installed primary bootloader.
        sys::esp!(unsafe { sys::esp_ota_get_bootloader_description(std::ptr::null(), &mut desc) })?;
        Ok(Self {
            version: desc.version,
            idf_version: bounded_c_string(&desc.idf_ver),
            built_at: bounded_c_string(&desc.date_time),
            secure_version: desc.secure_version,
        })
    }
}

fn read_slot(partition: *const sys::esp_partition_t) -> Option<OtaSlotDetails> {
    // SAFETY: callers only pass pointers returned by ESP-IDF's partition APIs.
    // These reference static partition descriptors; NULL means no matching slot.
    let partition = unsafe { partition.as_ref() }?;
    let mut state = sys::esp_ota_img_states_t_ESP_OTA_IMG_UNDEFINED;
    // SAFETY: partition is an IDF-owned descriptor and state is writable.
    let result = unsafe { sys::esp_ota_get_state_partition(partition, &mut state) };
    let state = match sys::EspError::from(result) {
        None => match state {
            sys::esp_ota_img_states_t_ESP_OTA_IMG_NEW => OtaImageState::New,
            sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY => {
                OtaImageState::PendingVerification
            }
            sys::esp_ota_img_states_t_ESP_OTA_IMG_VALID => OtaImageState::Valid,
            sys::esp_ota_img_states_t_ESP_OTA_IMG_INVALID => OtaImageState::Invalid,
            sys::esp_ota_img_states_t_ESP_OTA_IMG_ABORTED => OtaImageState::Aborted,
            sys::esp_ota_img_states_t_ESP_OTA_IMG_UNDEFINED => OtaImageState::Undefined,
            _ => OtaImageState::Unavailable,
        },
        Some(error) if error.code() == sys::ESP_ERR_NOT_FOUND => OtaImageState::NotRecorded,
        Some(error) => {
            warn!(
                "OTA state unavailable for {}: {error}",
                bounded_c_string(&partition.label)
            );
            OtaImageState::Unavailable
        }
    };
    Some(OtaSlotDetails {
        label: bounded_c_string(&partition.label),
        address: partition.address,
        size_bytes: partition.size,
        state,
    })
}

// Bound reads to the actual array: older or malformed metadata need not contain a NUL.
fn bounded_c_string(value: &[std::ffi::c_char]) -> String {
    let bytes = value
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte.to_ne_bytes()[0])
        .collect::<Vec<_>>();
    String::from_utf8_lossy(&bytes).into_owned()
}
