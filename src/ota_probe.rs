//! Explicit diagnostic image. Normal firmware never compiles this module.

#[cfg(not(esp_idf_bootloader_app_rollback_enable))]
compile_error!(
    "OTA probe requires CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y; use make_ota_probe_image"
);

use anyhow::{Context, ensure};
use esp_idf_sys as sys;

use crate::*;

const FAIL_NEXT_BOOT_KEY: &str = "ota_test_fail";

/// This runs before mark_running_slot_valid, WiFi, sensors and the Tokio runtime.
pub fn on_boot(
    details: &mut OtaDetails,
    nvs: &mut nvs::EspNvs<nvs::NvsDefault>,
) -> anyhow::Result<()> {
    let fail_requested = nvs.get_u8(FAIL_NEXT_BOOT_KEY)? == Some(1);
    // Consume the request before any reboot. A stale request never rejects a VALID image.
    nvs.remove(FAIL_NEXT_BOOT_KEY)?;

    let (message, can_test_fallback) = match details.running_state_at_boot {
        OtaImageState::Undefined => match arm_current_slot() {
            Ok(()) => {
                warn!("OTA probe: current slot armed as NEW; rebooting to examine bootloader transition");
                FreeRtos::delay_ms(1000);
                esp_idf_hal::reset::restart();
            }
            Err(error) => (format!("Probe not armed: {error:#}"), false),
        },
        OtaImageState::PendingVerification if fail_requested => {
            // Deliberately do NOT mark invalid or valid: the bootloader must notice
            // PENDING_VERIFY itself on the following boot and perform the fallback.
            warn!("OTA probe: intentional unconfirmed restart in 5 seconds; expect previous firmware");
            FreeRtos::delay_ms(5000);
            esp_idf_hal::reset::restart();
        }
        OtaImageState::PendingVerification => (
            "Confirmed: the bootloader changed NEW to PENDING_VERIFY. This probe is now being marked valid.".into(),
            true,
        ),
        OtaImageState::New => {
            details.bootloader_rollback = BootloaderRollback::NotObserved;
            (
                "Rollback was not observed: the bootloader left the probe NEW. The probe is now being marked valid; restore normal firmware through OTA.".into(),
                false,
            )
        }
        OtaImageState::Valid => (
            "Probe already validated on an earlier boot. Reinstall the probe from normal firmware to repeat the observation.".into(),
            false,
        ),
        state => (format!("Inconclusive: unexpected image state {state}; no test armed."), false),
    };
    info!("OTA probe: {message}");
    details.probe = Some(OtaProbeDetails {
        message,
        can_test_fallback,
    });
    Ok(())
}

/// Arm a deliberate unconfirmed boot only after this boot demonstrated support.
/// The caller owns both the OTA service and the application's maintenance guard.
pub fn request_fallback(
    details: &OtaDetails,
    nvs: &mut nvs::EspNvs<nvs::NvsDefault>,
) -> anyhow::Result<()> {
    ensure!(
        details
            .probe
            .as_ref()
            .is_some_and(|probe| probe.can_test_fallback),
        "Observe PENDING_VERIFY on this boot before requesting the fallback test"
    );
    // Persist first: the following boot must know to withhold confirmation.
    nvs.set_u8(FAIL_NEXT_BOOT_KEY, 1)?;
    if let Err(error) = arm_current_slot() {
        nvs.remove(FAIL_NEXT_BOOT_KEY)?;
        return Err(error);
    }
    Ok(())
}

/// IDF 6.1 re-arms the active otadata entry when selecting the running slot,
/// preserving the previous slot's entry. No application partition is written here.
fn arm_current_slot() -> anyhow::Result<()> {
    // SAFETY: IDF returns immutable, static partition descriptors.
    let running = unsafe { sys::esp_ota_get_running_partition().as_ref() }
        .context("Running partition unavailable")?;
    let boot = unsafe { sys::esp_ota_get_boot_partition().as_ref() }
        .context("Boot partition unavailable")?;
    let fallback = unsafe { sys::esp_ota_get_next_update_partition(std::ptr::null()).as_ref() }
        .context("Previous OTA partition unavailable")?;
    ensure!(
        boot.address == running.address,
        "A different boot slot is already selected"
    );
    ensure!(
        fallback.address != running.address,
        "No separate fallback slot"
    );
    // Keep this test specific to the repository's two-slot partition layout.
    ensure!(
        unsafe { sys::esp_ota_get_app_partition_count() } == 2,
        "Probe requires two OTA slots"
    );
    let mut fallback_state = sys::esp_ota_img_states_t_ESP_OTA_IMG_UNDEFINED;
    sys::esp!(unsafe { sys::esp_ota_get_state_partition(fallback, &mut fallback_state) })?;
    ensure!(
        fallback_state == sys::esp_ota_img_states_t_ESP_OTA_IMG_VALID,
        "Previous OTA slot must be marked VALID"
    );
    // This checks the previous otadata entry and verifies its actual image too.
    ensure!(
        unsafe { sys::esp_ota_check_rollback_is_possible() },
        "ESP-IDF could not verify a usable fallback image"
    );
    sys::esp!(unsafe { sys::esp_ota_set_boot_partition(running) })?;
    let mut state = sys::esp_ota_img_states_t_ESP_OTA_IMG_UNDEFINED;
    sys::esp!(unsafe { sys::esp_ota_get_state_partition(running, &mut state) })?;
    ensure!(
        state == sys::esp_ota_img_states_t_ESP_OTA_IMG_NEW,
        "Probe did not enter NEW state"
    );
    // Verify that staging this same-slot test preserved the fallback's state.
    sys::esp!(unsafe { sys::esp_ota_get_state_partition(fallback, &mut fallback_state) })?;
    ensure!(
        fallback_state == sys::esp_ota_img_states_t_ESP_OTA_IMG_VALID,
        "Fallback state changed while staging probe"
    );
    Ok(())
}
