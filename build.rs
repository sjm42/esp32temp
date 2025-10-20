// build.rs
fn main() -> anyhow::Result<()> {
    let _ = build_data::set_GIT_BRANCH();
    let _ = build_data::set_GIT_COMMIT();
    let _ = build_data::set_SOURCE_TIMESTAMP();
    let _ = build_data::set_RUSTC_VERSION();
    let _ = build_data::no_debug_rebuilds();

    // Necessary because of this issue: https://github.com/rust-lang/cargo/issues/9641
    // see also https://github.com/rust-lang/cargo/issues/9554

    embuild::build::CfgArgs::output_propagated("ESP_IDF")?;
    embuild::build::LinkArgs::output_propagated("ESP_IDF")?;

    Ok(())
}
// EOF
