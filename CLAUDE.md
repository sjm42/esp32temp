# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ESP32 temperature monitoring firmware in Rust. Reads DS18B20 OneWire sensors and exposes data via a web UI and optional MQTT. Supports ESP32 (Xtensa) and ESP32-C3 (RISC-V). Currently configured for **ESP32-C3**.

## Build Commands

```bash
cargo build -r          # Build release firmware
cargo run -r            # Build, flash, and open serial monitor
./makeimage             # Create firmware.bin for OTA updates
./flash                 # Helper script to flash firmware
```

There is no test suite — testing is done manually on hardware.

## Architecture Switching (ESP32 vs ESP32-C3)

Three files must be changed together:
1. `.cargo/config.toml` — switch target between `xtensa-esp32-espidf` and `riscv32imc-esp-espidf`, also set `MCU` env var
2. `rust-toolchain.toml` — switch channel between `esp` (ESP32) and `nightly` (ESP32-C3)
3. `Cargo.toml` — switch default feature between `esp32s` and `esp32c3`

## Architecture

Single async binary (`src/bin/esp32temp.rs`) using Tokio single-threaded runtime. Runs concurrent tasks via `tokio::select!`:

- **poll_sensors** (`measure.rs`) — scans DS18B20 sensors on configured GPIO pins with retry logic
- **run_api_server** (`apiserver.rs`) — Axum HTTP server on port 80 with REST API + HTML UI (Askama templates in `templates/`)
- **run_mqtt** (`mqtt.rs`) — optional MQTT publisher for sensor data
- **wifi_loop** (`wifi.rs`) — WiFi connection manager with auto-reconnect, supports WPA2 Personal/Enterprise
- **pinger** — network connectivity monitor, reboots on prolonged failure
- **poll_reset** — GPIO9 button handler for factory reset (hold 10s)

Shared state: `Arc<Pin<Box<MyState>>>` with `RwLock` fields (`state.rs`).

Configuration (`config.rs`): persisted in NVS using `postcard` serialization with CRC-32 checksum. Editable via web UI (POST /config triggers reboot).

## Key API Endpoints

`GET /temp` — JSON temperature readings, `GET /config` and `POST /config` — configuration, `POST /fw` — OTA firmware update from URL, `GET /reset_config` — factory reset.

## Pin Configuration

GPIO pin assignments are feature-gated in `src/bin/esp32temp.rs` (~line 80). Each chip variant has its own set of usable pins defined in the `onew_pins` array.

## Dependencies of Note

- `ds18b20` and `one-wire-bus` are **custom forks** (github.com/sjm42/) fixing negative temperature handling and migrated to embedded-hal 1.0
- ESP-IDF version pinned to v5.4.3 in `.cargo/config.toml`
- `cc` build dependency pinned to exact version `=1.1.30`

## Build Profiles

- **Release**: `opt-level = "z"`, fat LTO, strip — optimized for minimal binary size
- **Dev**: `opt-level = 2` — needs optimization even in dev for reasonable embedded performance

## Flash Partitions

Dual OTA partition layout (`partitions.csv`): two 1984KB app slots for safe firmware updates, plus NVS (16KB) and OTA data (8KB).