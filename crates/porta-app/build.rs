//! Compiles ui/main.slint into Rust when the `ui` feature is on. Cargo
//! exposes enabled features to build scripts as CARGO_FEATURE_<NAME>
//! env vars, so this is a no-op (and needs no display/graphics stack)
//! for every other build.

fn main() {
    if std::env::var("CARGO_FEATURE_UI").is_ok() {
        slint_build::compile("ui/main.slint").unwrap();
    }
}
