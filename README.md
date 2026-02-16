# esp32temp

Temperature measurement with ESP32 and DS18B20 sensor(s).

Provides a web interface for monitoring temperatures and configuring the device,
with optional MQTT publishing for IoT integration. Supports over-the-air (OTA) firmware updates.

## How to build it

This has been tested on a freshly installed Debian 12 system.
It should not be too hard to adapt for other distros out there.

First install OS dependencies for building the bits and pieces, and then install Rust itself.

```lang=bash
sudo apt -y install build-essential curl git libssl-dev libudev-dev pkg-config python3-venv

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs > rustup.sh
chmod 755 rustup.sh
./rustup.sh

. $HOME/.cargo/env
rustup toolchain add nightly
```

Then install a bunch of tools for cross compiling, flashing etc.

```lang=bash
# optional & useful
cargo install cargo-binutils cargo-embed cargo-flash cargo-generate cargo-update probe-run

cargo install espmonitor espup ldproxy flip-link
cargo install cargo-espflash --version 3.0.0-rc.2
cargo install espflash --version 3.0.0-rc.2
espup install
```

Now actually obtain the source code and build out firmware.

```lang=bash
mkdir $HOME/git && cd $HOME/git
git clone https://github.com/sjm42/esp32temp.git
cd esp32temp

cargo build -r
```

Flash it on the chip!

```lang=bash
cargo run -r
```

Occasionally, we run these _spells_ to keep our tools up to date.

```lang=bash
rustup update
espup update
cargo install-update -a
```

## Support for ESP32 and ESP32-C3

By default, the source code and configs are made for supporting ESP32.
Support for ESP32-C3 is easy to do with 3 changes.

- in `.cargo/config.toml` change the target `xtensa-esp32-espidf` to `riscv32imc-esp-espidf` i.e. comment out the first one and remove the comment mark from the other
- in `rust-toolchain.toml` comment out the `channel="esp"` line and remove the comment chars on two lines below the "for ESP32-C3" comment lines.
- optionally in `Cargo.toml` change `default = ["esp32s"]` to `default = ["esp32c3"]` in the `[feature]` section.

## Support for other ESP32 boards?

It should be easy to support almost any ESP32 versions with WiFi.
Just check the pin assignments inside `src/bin/esp32temp.rs` starting after line 80 (at the time of writing).
There we have an assignment to Boxed array `onew_pins` that will hold the pins we are probing for sensors.
Those assignments are behind _feature gates_ aka conditional compilation and we can easily add more of them
to support new hardware variants.

Just add more _features_ into `Cargo.toml` and assign the usable gpio pins accordingly and there you go.

## Internals

### Runtime Architecture

The firmware runs on a **Tokio single-threaded async runtime** on top of ESP-IDF / FreeRTOS.
The main task stack is set to 20 KB (`sdkconfig.defaults`) to accommodate Tokio.
The entry point (`src/bin/esp32temp.rs`) launches six concurrent tasks via `tokio::select!`,
meaning all tasks run cooperatively and the firmware reboots if any of them exits:

| Task | Source | Purpose |
|------|--------|---------|
| `poll_sensors` | `measure.rs` | Reads DS18B20 sensors at a configurable interval (default 60 s) |
| `run_api_server` | `apiserver.rs` | Axum HTTP server on port 80 (web UI + REST API) |
| `run_mqtt` | `mqtt.rs` | Publishes temperature data to an MQTT broker (optional) |
| `wifi_loop` | `wifi.rs` | Manages WiFi connection, reconnects on drop |
| `pinger` | `esp32temp.rs` | Pings the gateway every 5 minutes, reboots on failure |
| `poll_reset` | `esp32temp.rs` | Tracks uptime, monitors GPIO9 button for factory reset |

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
  with embedded JavaScript (`src/form.js`) for auto-refreshing temperature and uptime data
- `GET /temp` — JSON array of current sensor readings (filters out invalid values)
- `GET /uptime` — JSON uptime in seconds and human-readable string
- `GET /config` / `POST /config` — read or update device configuration (POST triggers reboot)
- `GET /reset_config` — restore factory defaults and reboot
- `POST /fw` — OTA firmware update: provide an HTTP URL to a firmware binary,
  which is streamed directly into the inactive OTA partition and activated on reboot

Static assets (favicon, JavaScript) are embedded in the binary via `include_bytes!`.

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

Holding the button on GPIO9 for approximately 5 seconds (9 half-second countdown ticks)
triggers a factory reset: default config is written to NVS and the device reboots.
