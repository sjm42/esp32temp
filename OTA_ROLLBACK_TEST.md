# Controlled test of the installed bootloader

This diagnostic tests the **installed** bootloader without replacing it. The normal
firmware keeps rollback disabled. The diagnostic identifies itself as
`2.2.0-ota-probe` and enables OTA state handling only in its separate build directory.
Use the OTA web form throughout this procedure. `cargo run`, `flash_c3`,
`flash_wroom32` and serial full-image flashing can install a different bootloader or
erase `otadata`, which would invalidate this test.

The expected verification and fallback transitions follow Espressif's
[OTA rollback documentation](https://docs.espressif.com/projects/esp-idf/en/v5.5.1/esp32/api-reference/system/ota.html#rollback-process).

## Build and serve the images

Choose the board actually fitted to the device:

```sh
# ESP32-C3
./make_ota_image_c3
./make_ota_probe_image c3
mkdir -p target/ota-test-images
cp firmware-c3.bin firmware-c3-ota-probe.bin target/ota-test-images/

# Or ESP-WROOM-32 (use these four commands instead)
./make_ota_image_wroom32
./make_ota_probe_image wroom32
mkdir -p target/ota-test-images
cp firmware-wroom32.bin firmware-wroom32-ota-probe.bin target/ota-test-images/

python3 -m http.server 8000 --bind 0.0.0.0 --directory target/ota-test-images
```

The device needs to reach the computer's LAN address on TCP port 8000. Use an actual
LAN IP in the URLs below, not `localhost`. Check that the image URL downloads from
another machine first. The generated files are application-only images; the scripts
check that they fit both OTA slots.

Before starting, record the installed bootloader version/build time, the running
slot and firmware version from the index page. Confirm that the current normal
firmware connects to WiFi and its index page works. Have serial access available
for recovery if a hardware or flash problem occurs. Do not overwrite the previous
normal image with another OTA while the diagnostic is running.

## Observe the bootloader's verification transition

1. On the current firmware's index page, submit the probe URL in the usual OTA form:
   `http://COMPUTER_LAN_IP:8000/firmware-c3-ota-probe.bin` (or the WROOM32 name).
2. Allow the download and reboot. Since the current updater has rollback disabled,
   it normally stages the probe as `UNDEFINED`. On that first boot, the probe verifies
   that the other slot is marked `VALID` and contains an image ESP-IDF accepts. It
   then uses ESP-IDF 6.1 to re-arm **its own running slot** as `NEW`, preserving the
   previous slot's OTA entry, and reboots once more. This second reboot happens
   before WiFi starts; it is expected.
3. Wait for WiFi to reconnect, reload the index page, and save the result before
   any further reboot. The page should show `2.2.0-ota-probe` and the same installed
   bootloader metadata you recorded earlier.

Interpret the **Image state at boot** and **Controlled rollback probe** message:

| Result | Meaning and next step |
| --- | --- |
| `Pending verification`, with `Confirmed` | The bootloader performed `NEW → PENDING_VERIFY`: its application rollback handling is enabled. The probe immediately marks itself valid, so ordinary later reboots are safe. Continue with the optional fallback test below. |
| `New`, with `Rollback was not observed` | The bootloader did not perform the transition. For a standard ESP-IDF bootloader this means application rollback was compiled out. The probe marks itself valid and keeps serving the UI. Restore normal firmware through OTA. |
| `Probe not armed` | A suitable previous image or OTA entry was unavailable, or staging failed. No capability conclusion can be drawn. Restore normal firmware through OTA and inspect the reported reason. |
| `Valid` / `Probe already validated` | The relevant first-boot observation was lost to a subsequent restart. Restore normal firmware, then install the probe again to repeat it. |
| Any other state | Inconclusive; no automatic test is armed. Restore normal firmware and inspect the logs. |

The page captures the state **before** marking the image valid; the slot card showing
`Valid` afterward is expected. The probe does not wait for NTP, sensors or WiFi to
confirm itself. The boot transition test therefore does not depend on those services.

## Optional: demonstrate actual fallback

Do this only after `Pending verification` was observed and while the diagnostic's
**Test fallback to previous firmware** button is available. Do not upload anything
else first: an OTA would overwrite the intended previous image.

1. Record the current probe slot and previous slot. With the original screenshot's
   layout, the probe should be in `ota_0` and the previous normal firmware in `ota_1`.
2. Press **Test fallback to previous firmware** and accept its confirmation. The
   handler verifies the fallback again, writes a one-shot request to NVS, and marks
   the probe's own slot `NEW` without changing the previous image.
3. Expect two restarts: one into the probe as `PENDING_VERIFY`, then an intentional
   restart after five seconds **without marking the image valid or invalid**. The
   one-shot request is consumed before that second restart.
4. Wait for the previous normal firmware to reconnect and reload its index page.
   Confirm the original firmware version and running slot have returned, the
   installed bootloader metadata is unchanged, and the rejected probe slot is
   `Aborted`. For the screenshot's initial layout, expect normal `2.2.0` running
   from `ota_1`, with `ota_0` shown as `Aborted` in the update-target card.

That result demonstrates the bootloader's actual automatic fallback, rather than
an application explicitly selecting the old slot. The restored normal firmware
still reports its bootloader capability as `Unknown`: it does not import the probe's
observation, and its own first-boot state is `Valid`. Keep the saved results as evidence.

The same action can be requested without the browser button:

```sh
curl --fail-with-body -X POST http://DEVICE_LAN_IP/ota-test/rollback
```

The route exists only in the diagnostic build and refuses the request unless that
boot observed pending verification. If the diagnostic returns after the requested
test instead of the previous firmware, save its startup state and serial logs;
the expected fallback was not demonstrated.

## Restore normal firmware if the probe is still running

Use its regular OTA form with `http://COMPUTER_LAN_IP:8000/firmware-c3.bin` (or
`firmware-wroom32.bin`). Wait for the reboot and verify the firmware version no
longer has `-ota-probe`, the app rollback build setting is `Disabled`, and the
bootloader metadata is unchanged. This OTA writes the other slot; the normal
firmware immediately confirms itself on startup.

If actual fallback succeeded, normal firmware is already restored and no upload
is necessary. Stop the temporary HTTP server when finished.

## Development validation

The helper also supports `./make_ota_probe_image c3 check` and
`./make_ota_probe_image c3 clippy`, or `wroom32` instead of `c3`. The probe uses a
compile-time guard to reject builds without native rollback state handling enabled.
The separate `target/ota-probe-*` directories prevent its native configuration from
being reused by normal builds. Hardware execution is still required to establish
the installed bootloader's behavior; compiling these images establishes no result.
