// build.rs
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};

fn main() -> anyhow::Result<()> {
    bd(build_data::set_GIT_BRANCH())?;
    bd(build_data::set_GIT_COMMIT())?;
    bd(build_data::set_SOURCE_TIMESTAMP())?;
    bd(build_data::set_RUSTC_VERSION())?;
    bd(build_data::no_debug_rebuilds())?;

    // Necessary because of this issue: https://github.com/rust-lang/cargo/issues/9641
    // see also https://github.com/rust-lang/cargo/issues/9554
    embuild::build::CfgArgs::output_propagated("ESP_IDF")?;
    embuild::build::LinkArgs::output_propagated("ESP_IDF")?;
    build_static_assets(&PathBuf::from(env::var("OUT_DIR")?))?;

    Ok(())
}

fn bd(result: Result<(), String>) -> anyhow::Result<()> {
    result.map_err(anyhow::Error::msg)
}

fn build_static_assets(out_dir: &Path) -> anyhow::Result<()> {
    compress_asset("static/form.js", out_dir.join("form.js.gz"))?;
    compress_asset("static/index.css", out_dir.join("index.css.gz"))?;
    compress_asset("static/favicon.ico", out_dir.join("favicon.ico.gz"))?;
    Ok(())
}

fn compress_asset(src: &str, dst: PathBuf) -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed={src}");

    let bytes = fs::read(src)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&bytes)?;
    let compressed = encoder.finish()?;
    fs::write(dst, compressed)?;
    Ok(())
}
// EOF
