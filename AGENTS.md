# Repository Guidelines

## Project Structure & Module Organization
`src/bin/esp32temp.rs` is the firmware entrypoint. Shared logic lives in `src/` modules such as `wifi.rs`, `mqtt.rs`, `apiserver.rs`, `measure.rs`, `state.rs`, `config.rs`, and `rmt_ow.rs`, with `src/lib.rs` wiring exports.

UI assets are embedded into the firmware: `templates/index.html.ask` (Askama template) and the files in `static/` (`form.js`, `index.css`, `favicon.ico`). Build/runtime configuration files are at the repo root (`sdkconfig.defaults`, `partitions.csv`, `rust-toolchain.toml`). The ESP-IDF component lockfile for native `onewire_bus` integration is `components_esp32c3.lock`. Helper scripts include `flash_c3`, `flash_wroom32`, `make_ota_image_c3`, and `make_ota_image_wroom32`.

## Build, Test, and Development Commands
- `cargo check` — fast validation of Rust code without producing a firmware binary.
- `cargo build -r` — build optimized firmware (standard release build from `README.md`).
- `cargo run -r` — build and flash/run the firmware via the configured ESP toolchain.
- `./flash_c3` — convenience wrapper for ESP32-C3 flash + monitor.
- `./flash_wroom32` — convenience wrapper for ESP-WROOM-32 flash + monitor.
- `./make_ota_image_c3` / `./make_ota_image_wroom32` — build OTA image for the target board.
- `cargo fmt` — format Rust code (use before commits).
- `cargo clippy --all-targets -- -D warnings` — lint; the repo includes `clippy.toml`.

## Coding Style & Naming Conventions
Use standard Rust formatting (`cargo fmt`) with 4-space indentation. Follow idiomatic Rust naming: `snake_case` for functions/modules/files, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.

Keep modules focused by subsystem (WiFi, MQTT, API, measurement). Prefer explicit error propagation (`anyhow::Result`) and keep embedded-specific constants near the hardware logic that uses them.

The 1-Wire implementation is RMT-backed through Espressif's native `onewire_bus` component. `src/rmt_ow.rs` is a thin local wrapper around that API so the native pull-up flag can be enabled explicitly; preserve that behavior unless you are intentionally reworking the 1-Wire backend.

## Testing Guidelines
There is currently no dedicated `tests/` directory or unit-test suite in the repository. At minimum, run `cargo check`, `cargo clippy`, and a release build (`cargo build -r`) before opening a PR.

For behavior changes, document manual validation on hardware (board type, sensor setup, WiFi mode, and observed API/MQTT behavior).

## Commit & Pull Request Guidelines
Recent history uses short, imperative commit subjects (for example, `cargo update`). Keep commit titles concise and action-oriented; include a body when changing runtime behavior, config defaults, or flashing flows.

PRs should include: what changed, why, how it was tested (commands + device/manual checks), and any required target/feature setup (ESP32 vs ESP32-C3). Add screenshots only for web UI changes.
