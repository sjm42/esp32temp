# esp32temp

Temperature measurement with ESP32 and DS18b20 sensor(s)

## How to build it

This example has been tested on a freshly installed Debian 12 system.
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
