// build.rs

use std::env;

fn main() -> anyhow::Result<()> {
    // Necessary because of this issue: https://github.com/rust-lang/cargo/issues/9641
    // see also https://github.com/rust-lang/cargo/issues/9554

    embuild::build::CfgArgs::output_propagated("ESP_IDF")?;
    embuild::build::LinkArgs::output_propagated("ESP_IDF")?;

    let wifi_ssid = env::var("WIFI_SSID").unwrap_or_else(|_| "internet".into());
    let wifi_pass = env::var("WIFI_PASS").unwrap_or_else(|_| "password".into());
    let api_port = env::var("API_PORT").unwrap_or_else(|_| "80".into());

    println!("cargo:rustc-env=WIFI_SSID={wifi_ssid}");
    println!("cargo:rustc-env=WIFI_PASS={wifi_pass}");
    println!("cargo:rustc-env=API_PORT={api_port}");

    Ok(())
}

// EOF
