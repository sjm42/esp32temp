# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ESP32 temperature monitoring firmware in Rust. Reads DS18B20 OneWire sensors and exposes data via a web UI and optional MQTT. Supports ESP32 (Xtensa) and ESP32-C3 (RISC-V). Currently configured for **ESP32-C3**.

## Build Commands

```bash
cargo build -r          # Build release firmware
cargo run -r            # Build, flash, and open serial monitor
cargo check             # Fast compile check
cargo clippy --all-targets -- -D warnings
./flash_c3              # Build+flash+monitor for ESP32-C3 (default)
./flash_wroom32         # Build+flash+monitor for ESP-WROOM-32 (Xtensa toolchain)
./make_ota_image_c3     # Create firmware-c3.bin for OTA updates
./make_ota_image_wroom32 # Create firmware-wroom32.bin for OTA updates
```

There is no test suite — testing is done manually on hardware.

## Hardware Target Selection (ESP32-C3 vs ESP-WROOM-32)

Preferred approach: use the target-specific scripts instead of editing files.

- `esp32-c3` (default feature): `cargo run -r` / `./flash_c3`
- `esp-wroom-32`: `MCU=esp32 cargo +esp run -r --target xtensa-esp32-espidf --no-default-features --features=esp-wroom-32`

If manually changing defaults, keep these aligned:
1. `.cargo/config.toml` — target (`riscv32imc-esp-espidf` vs `xtensa-esp32-espidf`) and `MCU`
2. `rust-toolchain.toml` — `nightly` (C3) vs `esp` (Xtensa ESP32)
3. `Cargo.toml` — default feature (`esp32-c3` vs `esp-wroom-32`)

## Architecture

Single async binary (`src/bin/esp32temp.rs`) using Tokio single-threaded runtime. Runs concurrent tasks via `tokio::select!`:

- **poll_sensors** (`measure.rs`) — scans DS18B20 sensors on configured GPIO pins with retry logic using the ESP-IDF `onewire_bus` RMT backend
- **run_api_server** (`apiserver.rs`) — Axum HTTP server on port 80 with REST API + HTML UI (Askama templates in `templates/`)
- **run_mqtt** (`mqtt.rs`) — optional MQTT publisher for sensor data
- **wifi_loop** (`wifi.rs`) — WiFi connection manager with auto-reconnect, supports WPA2 Personal/Enterprise
- **pinger** — network connectivity monitor, reboots on prolonged failure
- **poll_reset** — target-specific button handler for factory reset / one-shot AP boot (GPIO9 on C3, GPIO0 on WROOM32)

Shared state: `Arc<Pin<Box<MyState>>>` with `RwLock` fields (`state.rs`).

Configuration (`config.rs`): persisted in NVS using `postcard` serialization with CRC-32 checksum. Editable via web UI (POST /config triggers reboot).
If no WiFi configuration is stored, the device boots into AP mode for initial setup.

## Key API Endpoints

`GET /temp` — JSON temperature readings, `GET /config` and `POST /config` — configuration, `POST /fw` — OTA firmware update from URL, `GET /reset_config` — factory reset. UI static assets are served at `GET /form.js`, `GET /index.css`, and `GET /favicon.ico`.

## Pin Configuration

GPIO pin assignments are feature-gated in `src/bin/esp32temp.rs`. Each chip variant has its own candidate 1-Wire GPIO list there. The ESP32-C3 onboard LED pin is intentionally excluded from sensor scanning.

## Dependencies of Note

- Native 1-Wire support comes from Espressif's `onewire_bus` component declared in `Cargo.toml` under `package.metadata.esp-idf-sys.extra_components`
- `src/rmt_ow.rs` is a small local wrapper around the ESP-IDF RMT 1-Wire API that enables the native `en_pull_up` flag explicitly
- ESP-IDF version pinned to `v5.5.4` in `.cargo/config.toml`
- `cc` build dependency pinned to exact version `=1.1.30`

## Build Profiles

- **Release**: `opt-level = "z"`, fat LTO, strip — optimized for minimal binary size
- **Dev**: `opt-level = 2` — needs optimization even in dev for reasonable embedded performance

## Flash Partitions

Dual OTA partition layout (`partitions.csv`): two 1984KB app slots for safe firmware updates, plus NVS (16KB) and OTA data (8KB).
