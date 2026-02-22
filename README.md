# esp32temp

Temperature measurement with ESP32 and DS18B20 sensor(s).

Provides a web interface for monitoring temperatures and configuring the device,
with optional MQTT publishing for IoT integration. Supports over-the-air (OTA) firmware updates.

## Hardware Targets

This firmware supports two hardware targets using Cargo features and target-specific build commands:

- **ESP32-C3** (RISC-V) via feature `esp32-c3` (default)
- **ESP-WROOM-32** (Xtensa ESP32) via feature `esp-wroom-32`

Factory reset button GPIO differs by target:

- `esp32-c3`: GPIO9
- `esp-wroom-32`: GPIO0

OneWire probe pin lists are selected with `#[cfg(feature = "...")]` in `src/bin/esp32temp.rs`.
Adding support for other ESP32 boards is mostly a matter of defining a new feature and pin map there.

## Building & Flashing

### Prerequisites

- Rust nightly with `rust-src` (default path in `rust-toolchain.toml`)
- ESP tools: `espflash`, `ldproxy`, `espup`
- Xtensa builds (`ESP-WROOM-32`) also require the `esp` Rust toolchain (`cargo +esp`)

Debian/Ubuntu packages and Rust bootstrap example:

```bash
sudo apt -y install build-essential curl git libssl-dev libudev-dev pkg-config python3-venv clang-18
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
chmod 755 rustup.sh
./rustup.sh

. "$HOME/.cargo/env"
rustup toolchain add nightly
espup install

cargo install espmonitor espup ldproxy flip-link cargo-espflash espflash

# optional & useful
cargo install cargo-binutils cargo-embed cargo-flash cargo-generate cargo-update probe-run
```

### Utility Scripts

Optional helper environment file:

```bash
source env.sh   # WIFI_SSID/WIFI_PASS defaults for build-time config
```

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

## Internals

### Runtime Architecture

The firmware runs on a **Tokio single-threaded async runtime** on top of ESP-IDF / FreeRTOS.
The main task stack is set to 20 KB (`sdkconfig.defaults`) to accommodate Tokio.
The entry point (`src/bin/esp32temp.rs`) launches six concurrent tasks via `tokio::select!`,
meaning all tasks run cooperatively and the firmware reboots if any of them exits:

| Task             | Source         | Purpose                                                           |
|------------------|----------------|-------------------------------------------------------------------|
| `poll_sensors`   | `measure.rs`   | Reads DS18B20 sensors at a configurable interval (default 60 s)   |
| `run_api_server` | `apiserver.rs` | Axum HTTP server on port 80 (web UI + REST API)                   |
| `run_mqtt`       | `mqtt.rs`      | Publishes temperature data to an MQTT broker (optional)           |
| `wifi_loop`      | `wifi.rs`      | Manages WiFi connection, reconnects on drop                       |
| `pinger`         | `esp32temp.rs` | Pings the gateway every 5 minutes, reboots on failure             |
| `poll_reset`     | `esp32temp.rs` | Tracks uptime, monitors target-specific factory-reset button GPIO |

### Startup Sequence

1. ESP-IDF patches and logger initialization
2. Eventfd registration (required by Tokio's mio poll backend)
3. OTA slot validation — marks current slot as valid
4. NVS (Non-Volatile Storage) config load — falls back to defaults if missing or corrupt
5. GPIO pin setup and OneWire bus scan for DS18B20 sensors
6. WiFi driver creation
7. Shared state construction and Tokio runtime launch
8. All concurrent tasks start — `poll_sensors` and `run_api_server` block until WiFi is up,
   `poll_sensors` additionally waits for NTP time sync before beginning measurements

### Shared State

All tasks share application state through `Arc<Pin<Box<MyState>>>` (`state.rs`).
Individual fields are protected by `tokio::sync::RwLock` for async-safe concurrent access.
The config itself (`MyConfig`) is immutable at runtime — changing it via the web UI saves to NVS
and triggers a reboot.

### Configuration Persistence

Configuration (`config.rs`) is serialized with `postcard` (compact binary format) and protected
with a CRC-32 checksum. It is stored in the ESP32 NVS partition as a raw byte blob (max 256 bytes).
On boot, if the NVS data is missing or fails CRC validation, defaults are used and saved back.

Default WiFi credentials can be injected at build time via environment variables
`WIFI_SSID` and `WIFI_PASS`.

### Temperature Measurement

Each configured GPIO pin is scanned for DS18B20 devices on its OneWire bus at startup.
During polling (`measure.rs`), sensors are read at 12-bit resolution with configurable retries
(default 5) to handle CRC errors that occur occasionally on the bus. The `ds18b20` and
`one-wire-bus` crates are custom forks that fix negative temperature handling and use
embedded-hal 1.0.

### HTTP API

The Axum web server (`apiserver.rs`) provides:

- `GET /` — HTML dashboard rendered from an Askama template (`templates/index.html.ask`),
  with embedded JavaScript (`src/form.js`) and stylesheet (`src/index.css`)
- `GET /favicon.ico` — embedded favicon
- `GET /form.js` — embedded JavaScript for UI polling/form submissions
- `GET /index.css` — embedded stylesheet for the web UI
- `GET /temp` — JSON object with current sensor readings and metadata (invalid values filtered out)
- `GET /uptime` — JSON uptime in seconds and human-readable string
- `GET /config` / `POST /config` — read or update device configuration (POST triggers reboot)
- `GET /reset_config` — restore factory defaults and reboot
- `POST /fw` — OTA firmware update: provide an HTTP URL to a firmware binary,
  which is streamed directly into the inactive OTA partition and activated on reboot

Static assets (favicon, JavaScript, CSS) are embedded in the binary via `include_bytes!`.

### MQTT Publishing

When enabled in config, `mqtt.rs` connects to the configured broker and publishes JSON messages
on each sensor poll cycle:

- `{topic}/uptime` → `{ "uptime": <seconds> }`
- `{topic}/{sensor_id}` → `{ "temperature": <value> }`

Uses QoS AtLeastOnce with a 25-second keep-alive interval.

### WiFi Management

`wifi.rs` supports WPA2 Personal, WPA2 Enterprise (EAP-PEAP), and open networks.
Static IP or DHCP can be configured. The device ID is derived from the WiFi MAC address
(`esp32temp-XXXXXXXXXXXX`). On initial connection failure (30 s timeout), the device reboots.
After connecting, the `stay_connected` loop handles reconnection automatically.

### OTA Firmware Updates

The flash is partitioned with two OTA app slots (each ~2 MB) defined in `partitions.csv`.
On boot, the running slot is marked valid. A new firmware image can be flashed via
`POST /fw` with an HTTP URL — it is streamed into the inactive slot using `EspOta`,
and the device reboots into it. If the new firmware fails, the previous slot remains available.

### Factory Reset

Holding the factory-reset button for approximately 5 seconds (9 half-second countdown ticks)
triggers a factory reset: default config is written to NVS and the device reboots.
The reset GPIO is `GPIO9` for `esp32-c3` builds and `GPIO0` for `esp-wroom-32` builds.
