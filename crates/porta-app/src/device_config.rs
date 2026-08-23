//! Remembers which period/input-channel-map worked for a given input
//! device, so `--in-map`/`--period` (or the UI's equivalent fields)
//! don't have to be retyped every launch (M6.1). Keyed by the input
//! device's resolved name, not written into any cassette: an
//! interface's channel wiring is a property of the physical setup, not
//! of any one project, and different interfaces need different
//! wiring (a Zoom L6 and a PreSonus Quantum 2626 don't wire their
//! inputs the same way).
//!
//! Also remembers the *name itself* of the last input device that
//! connected successfully, and callers substitute it for a blank
//! `--in`/input field rather than leaving that to cpal's own default
//! resolution. Found 2026-08-21 testing this on the Pi: cpal's
//! PipeWire host resolves "no device given" to a generic two-channel
//! `default_input` pseudo-device, not a real proxy to whatever's
//! actually plugged in - fine for a plain stereo mic, useless for a
//! 12-channel interface like the L6. Substituting the remembered name
//! sidesteps that pseudo-device entirely.
//!
//! History note: this started as a single contiguous channel *offset*,
//! deliberately not a per-track assignment ("no confirmed use case
//! yet... an additive change to one `DeviceSettings` entry, not a
//! rewrite" - the original comment here). The confirmed use case
//! arrived (change 002, owner-requested per-track selection) and took
//! exactly that additive path: `DeviceSettings` now stores a per-track
//! map, and offset-era files migrate on load (see `input_map.rs`,
//! where the serde types and the migration rule now live so their
//! tests run in the ungated CI gate - this module keeps only the file
//! I/O).
//!
//! There's no separate "save settings" action anywhere: a successful
//! `connect()`/`live` startup remembers whatever it just used,
//! silently. Reads and writes only ever happen on the control thread
//! (REQ-902 - this is plain synchronous file I/O, never called from
//! an audio callback).

use crate::input_map::{DeviceConfig, DeviceSettings, InputMap};
use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".config/openporta/audio.json"))
}

/// Missing file, unreadable file, or unparseable JSON all read as
/// "nothing remembered yet" rather than an error - there's no
/// first-run setup step, so a fresh install has to behave exactly
/// like reading an empty config.
pub fn load() -> DeviceConfig {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Record what actually worked for `input_device_name` and save
/// immediately. Best-effort: a write failure (e.g. no `$HOME`, a
/// read-only filesystem) just means nothing gets remembered this
/// time, not a reason to fail the connection that already succeeded.
pub fn remember(input_device_name: &str, output_device: &str, period: usize, map: InputMap) {
    let mut config = load();
    config.last_input_device = Some(input_device_name.to_string());
    config.devices.insert(
        input_device_name.to_string(),
        DeviceSettings::new(Some(output_device.to_string()), period, map),
    );
    save(&config);
}

fn save(config: &DeviceConfig) {
    let Some(path) = path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(path, json);
    }
}
