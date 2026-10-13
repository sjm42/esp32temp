# esp32temp

Temperature measurement with ESP32 and DS18B20 sensor(s).

Provides a web interface for monitoring temperatures and configuring the device,
with optional MQTT publishing and ESPHome native API integration. Supports over-the-air (OTA)
firmware updates.

## Hardware Targets

This firmware supports two hardware targets using Cargo features and target-specific build commands:

- **ESP32-C3** (RISC-V) via feature `esp32-c3` (default in `Cargo.toml` and `.cargo/config.toml`)
- **ESP-WROOM-32** (Xtensa ESP32) via feature `esp-wroom-32`

Factory reset button GPIO differs by target:

- `esp32-c3`: GPIO9
- `esp-wroom-32`: GPIO0

OneWire probe pin lists are selected with `#[cfg(feature = "...")]` in `src/bin/esp32temp.rs`.
Adding support for other ESP32 boards is mostly a matter of defining a new feature and pin map there.

## Building & Flashing

### Prerequisites

- The Espressif `esp` Rust toolchain installed with `espup` (selected by `rust-toolchain.toml`)
- ESP tools: `espflash`, `ldproxy`, `espup`

ESP-IDF is pinned to `v6.1`, with `esp-idf-hal` 0.47, `esp-idf-svc` 0.53 and
`esp-idf-sys` 0.38. MQTT uses the managed `espressif/mqtt` 1.x component.
After upgrading ESP-IDF, clear old native build caches with `cargo clean -p esp-idf-sys`
and `cargo clean -p esp-idf-sys --release` for each target being built.

Debian/Ubuntu packages and Rust bootstrap example:

```bash
sudo apt -y install build-essential curl git libssl-dev libudev-dev pkg-config python3-venv clang-18
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
chmod 755 rustup.sh
./rustup.sh

. "$HOME/.cargo/env"
cargo install espmonitor espup ldproxy flip-link cargo-espflash espflash
espup install
. "$HOME/export-esp.sh"

# optional & useful
cargo install cargo-binutils cargo-embed cargo-flash cargo-generate cargo-update probe-run
```

### Utility Scripts

Optional helper environment file:

```bash
source env.sh   # WIFI_SSID/WIFI_PASS defaults for build-time config
```

`env.sh` is local convenience setup for C3 builds. The target-specific scripts set the necessary
toolchain, target, MCU, and feature flags for their board.

Hardware-specific scripts:

```bash
./flash_c3
./flash_wroom32

./make_ota_image_c3
./make_ota_image_wroom32
```

Tooling updates (optional):

```bash
rustup update
espup update
cargo install-update -a
```

Dependency checks:

```bash
cargo outdated --root-deps-only
cargo update
```

`cargo outdated --root-deps-only` shows direct dependency updates worth considering for
`Cargo.toml`. `cargo update` refreshes compatible locked versions without changing declared
requirements. Full `cargo outdated` output may include transitive dependencies that are owned by
upstream crates rather than this firmware's manifest.

Both OTA-image scripts reject images larger than the smallest OTA slot in `partitions.csv`.
Their outputs are `firmware-c3.bin` and `firmware-wroom32.bin`.

ESP-IDF component manager lockfiles for MQTT and the RMT-backed 1-Wire bus are committed for both
supported chips: `components_esp32c3.lock` and `components_esp32.lock`.

## Internals

### Runtime Architecture

The firmware runs on a **Tokio single-threaded async runtime** on top of ESP-IDF / FreeRTOS.
The main task stack is set to 20 KB (`sdkconfig.defaults`) to accommodate Tokio.
The entry point (`src/bin/esp32temp.rs`) launches seven concurrent tasks via `tokio::select!`,
meaning all tasks run cooperatively and the firmware reboots if any of them exits:

Firmware downloads and gateway pings run on one blocking worker with a 20 KB stack,
so synchronous network calls do not block the async services. Fatal startup and task
errors are logged before a controlled reboot. Clippy denies explicit panic, unwrap and expect calls.

| Task              | Source           | Purpose                                                               |
|-------------------|------------------|-----------------------------------------------------------------------|
| `poll_sensors`    | `measure.rs`     | Reads DS18B20 sensors at a configurable interval (default 60 s)       |
| `run_api_server`  | `apiserver.rs`   | Axum HTTP server on port 80 (web UI + REST API)                       |
| `run_mqtt`        | `mqtt.rs`        | Publishes temperature data to an MQTT broker (optional)               |
| `run_esphome_api` | `esphome_api.rs` | Exposes sensors through the ESPHome native API on port 6053 (optional) |
| `wifi_loop`       | `wifi.rs`        | Manages WiFi connection, reconnects on drop                           |
| `pinger`          | `esp32temp.rs`   | Pings the gateway every 5 minutes, reboots on failure                 |
| `poll_reset`      | `esp32temp.rs`   | Tracks uptime, monitors target-specific reset/AP-mode button GPIO     |

### Startup Sequence

1. ESP-IDF patches and logger initialization
2. Eventfd registration (required by Tokio's mio poll backend)
3. OTA slot validation — marks current slot as valid
4. NVS (Non-Volatile Storage) config load — falls back to defaults if missing or corrupt
5. GPIO pin setup and OneWire bus scan for DS18B20 sensors
6. WiFi driver creation
7. Shared state construction and Tokio runtime launch
8. All concurrent tasks start — HTTP, MQTT and ESPHome wait until WiFi is up;
   `poll_sensors` waits for NTP time sync before beginning measurements. A missing NTP
   response leaves measurements waiting while the configuration UI remains available.

If no WiFi SSID is stored, or if a one-shot AP boot was requested by the reset button, the device
starts an open access point named `esp32temp` at `10.42.42.1` for manual configuration. Sensor
polling, MQTT, and ESPHome API serving are disabled while in AP mode.

### Shared State

All tasks share application state through `Arc<Pin<Box<MyState>>>` (`state.rs`).
Individual fields are protected by `tokio::sync::RwLock` for async-safe concurrent access.
The config itself (`MyConfig`) is immutable at runtime — changing it via the web UI saves to NVS
and triggers a reboot.

### Configuration Persistence

Configuration (`config.rs`) is serialized with `postcard` (compact binary format) and protected
with a CRC-32 checksum. Its application serialization buffer is 512 bytes, including the CRC;
only the actual serialized bytes are stored in NVS. On boot, if the data is missing, corrupt
or invalid, defaults are used and saved back. Configuration saves reject values that exceed
the storage buffer, invalid WiFi credentials, invalid static IPv4 settings and MQTT URLs/topics.
Polling intervals must be 1–86400 seconds, and read retries must be 0–20.

Default WiFi credentials can be injected at build time via environment variables
`WIFI_SSID` and `WIFI_PASS`.

The persisted config includes WiFi, IPv4/DHCP, ESPHome API enablement, MQTT settings, sensor retry
count, and sensor poll interval. `reset_settings` can be enabled as a Cargo feature to rewrite NVS
with default config during boot.

### Temperature Measurement

Each configured GPIO pin is scanned for DS18B20 devices on its OneWire bus at startup.
The current implementation uses Espressif's `onewire_bus` ESP-IDF component with a small
local wrapper in `src/rmt_ow.rs`, so bus timing is handled by the ESP32 RMT peripheral
instead of bit-banged software delays. During polling (`measure.rs`), sensors are read at
12-bit resolution with configurable retries (default 5) to handle occasional read/CRC
failures. The local wrapper exists so the native 1-Wire pull-up flag can be enabled
explicitly.

Each cycle replaces the reading snapshot by sensor ROM and GPIO. A failing sensor or bus
does not stop other measurements; failed readings become unavailable. The boot inventory
remains available to HTTP and ESPHome clients during conversion and read retries.

### HTTP API

The Axum web server (`apiserver.rs`) provides:

- `GET /` — HTML dashboard rendered from an Askama template (`templates/index.html.ask`),
  with embedded JavaScript (`static/form.js`) and stylesheet (`static/index.css`)
- `GET /favicon.ico` — embedded favicon
- `GET /form.js` — embedded JavaScript for UI polling/form submissions
- `GET /index.css` — embedded stylesheet for the web UI
- `GET /sensors` — JSON inventory of DS18B20 sensors detected at boot
- `GET /temp` — JSON object with current sensor readings and metadata (invalid values filtered out)
- `GET /uptime` — JSON uptime in seconds and human-readable string
- `GET /config` / `POST /config` — read or update device configuration (POST triggers reboot)
- `GET /reset_config` — restore factory defaults and reboot
- `POST /fw` — OTA firmware update: provide an HTTP URL to a firmware binary,
  which is streamed directly into the inactive OTA partition and activated on reboot

Static assets (favicon, JavaScript, CSS) are gzip-compressed by `build.rs` and embedded in the
binary via `include_bytes!`; handlers return them with `Content-Encoding: gzip`.

The index page includes an **OTA details** panel populated from `OtaDetails` in runtime state.
It shows the application's ESP-IDF version and rollback build setting, the installed primary
bootloader's version, ESP-IDF version, build date/time and security version, plus the running,
next-boot and update-target slot states, capacities and offsets. These details are captured at
startup; the initial image state is preserved before the existing application-validation call.
Bootloader rollback support is confirmed only when pending verification was observed at boot;
otherwise it is unknown, since bootloader metadata does not contain that setting. Missing or
unreadable bootloader metadata is displayed as unavailable and does not prevent startup.

### MQTT Publishing

When enabled in config, `mqtt.rs` connects to the configured broker and publishes JSON messages
on each sensor poll cycle:

- `{topic}/uptime` → `{ "uptime": <seconds> }`
- `{topic}/{sensor_id}` → `{ "temperature": <value> }`

Uses QoS AtLeastOnce with a 25-second keep-alive interval.

MQTT is disabled in AP mode.

### ESPHome Native API

When `esphome_enable` is set in config, `esphome_api.rs` listens on TCP port 6053 after WiFi is up.
It implements the unencrypted ESPHome native API framing needed for discovery/listing and state
subscriptions. Exposed entities are:

- `uptime` sensor in seconds
- `last_update` text sensor
- one temperature sensor per DS18B20 device detected at boot

ESPHome API serving is disabled in AP mode.

Frame reads retain partial data across poll timeouts. The API limits clients to four,
incoming payloads to 4096 bytes, partial frames to 15 seconds, writes to 10 seconds and
idle clients to 90 seconds. Duplicate ROM addresses advertise one entity, preserving
existing Home Assistant entity keys.

### WiFi Management

`wifi.rs` supports station mode with WPA2 Personal, WPA2 Enterprise, or open networks, plus open
AP mode for initial/manual configuration. Static IP or DHCP can be configured for station mode.
The device ID is derived from the WiFi MAC address (`esp32temp-XXXXXXXXXXXX`). On initial station
connection failure, the device reboots. Association and IP acquisition have bounded waits;
IP acquisition allows 30 seconds. Later failures retry after five seconds, resetting a
station left associated after a DHCP timeout. Every reconnection refreshes cached IP,
gateway and interface state. Disconnect events increment a generation counter so a
ping failure cannot reboot a device whose network changed during the check.

### OTA Firmware Updates

The flash is partitioned with two OTA app slots (each ~2 MB) defined in `partitions.csv`.
On boot, the running slot is marked valid. A new firmware image can be flashed via
`POST /fw` with an HTTP URL — it is streamed into the inactive slot using `EspOta`,
and the device reboots into it. The normal application build has rollback disabled;
the installed bootloader's setting is independent and may be unknown. Startup still
immediately marks the running slot valid. Automatic health-based fallback is not implemented.

An opt-in diagnostic image can test the installed bootloader's verification transition
and, after observing support, intentionally withhold confirmation to test actual fallback.
Build it with `./make_ota_probe_image c3` or `./make_ota_probe_image wroom32`. Follow the
[controlled rollback test procedure](OTA_ROLLBACK_TEST.md) using OTA only, preserving the
installed bootloader and the previous normal firmware. Normal builds exclude its test route.

HTTP request bodies are limited to 8 KiB. Firmware downloads have a 30-second HTTP I/O
timeout and a five-minute transfer deadline (a pending read can take up to the I/O timeout).
Declared and streamed sizes are checked against the inactive slot, and truncated or invalid
images fail before activation. Failed updates abort without scheduling a reboot. Successful
config saves and updates return JSON and schedule a reboot after two seconds. Concurrent
configuration changes and firmware updates receive HTTP 409 until the operation fails or
the device restarts. Cancelling an HTTP request does not cancel an active flash operation.

### Factory Reset

Holding the reset button for approximately 5 seconds (9 half-second countdown ticks) triggers a
factory reset: default config is written to NVS, any one-shot AP request is cleared, and the device
reboots. A short press while running in station mode stores a one-shot AP-mode request and reboots
for manual configuration. The reset GPIO is `GPIO9` for `esp32-c3` builds and `GPIO0` for
`esp-wroom-32` builds.

## Validation

Host regression tests cover fragmented ESPHome frames, cancellation and deadlines,
varint overflow, firmware transfer failures and OTA image-size checks:

```bash
cargo +stable test --locked --manifest-path tests/host/Cargo.toml --target x86_64-unknown-linux-gnu
python3 -B -m unittest discover -s tests -p 'test_*.py'
```

For firmware changes, run `cargo check`, `cargo clippy --all-targets -- -D warnings` and
`cargo build -r` for C3, then the equivalent commands with `MCU=esp32`, `cargo +esp`,
`--target xtensa-esp32-espidf --no-default-features --features=esp-wroom-32` for WROOM32.

Hardware validation remains necessary on both targets:

- DHCP and static-IP disconnect/reconnect, IP/gateway refresh and guarded ping recovery.
- NTP unavailable at boot, followed by recovery and timestamped measurements.
- Multiple sensors/buses with one sensor disconnected; verify HTTP, MQTT and ESPHome results.
- AP configuration, short button press, long-press reset and persistent settings after reboot.
- ESPHome discovery/subscription, stalled clients, and MQTT broker outage/recovery.
- OTA from existing firmware into each slot, then a second update into the other slot.
- OTA failures: HTTP 404, interrupted download, oversized image, wrong chip and invalid image;
  verify the old firmware keeps running and errors reach the browser.
