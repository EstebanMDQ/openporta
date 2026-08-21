//! Remembers which period/channel-offset worked for a given input
//! device, so `--in-offset`/`--period` (or the UI's equivalent fields)
//! don't have to be retyped every launch (M6.1). Keyed by the input
//! device's resolved name, not written into any cassette: an
//! interface's channel wiring is a property of the physical setup, not
//! of any one project, and different interfaces need different
//! offsets (a Zoom L6 and a PreSonus Quantum 2626 don't wire their
//! inputs the same way).
//!
//! Deliberately just an offset, not a full per-track channel
//! assignment - every device this has actually been run against wants
//! a contiguous block starting somewhere, and a free-form assignment
//! UI is real added complexity (more CLI surface, more UI surface, a
//! capture-wiring rewrite in realtime.rs) with no confirmed use case
//! yet. If a real interface ever needs non-contiguous channels, this
//! is an additive change to one `DeviceSettings` entry, not a rewrite.
//!
//! There's no separate "save settings" action anywhere: a successful
//! `connect()`/`live` startup remembers whatever it just used,
//! silently. Reads and writes only ever happen on the control thread
//! (REQ-902 - this is plain synchronous file I/O, never called from
//! an audio callback).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceSettings {
    pub output_device: Option<String>,
    pub period: usize,
    pub input_channel_offset: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceConfig {
    #[serde(default)]
    devices: HashMap<String, DeviceSettings>,
}

impl DeviceConfig {
    #[cfg_attr(not(feature = "realtime"), allow(dead_code))]
    fn path() -> Option<PathBuf> {
        Some(PathBuf::from(std::env::var_os("HOME")?).join(".config/openporta/audio.json"))
    }

    /// Missing file, unreadable file, or unparseable JSON all read as
    /// "nothing remembered yet" rather than an error - there's no
    /// first-run setup step, so a fresh install has to behave exactly
    /// like reading an empty config.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn get(&self, input_device_name: &str) -> Option<&DeviceSettings> {
        self.devices.get(input_device_name)
    }

    /// Record what actually worked for `input_device_name` and save
    /// immediately. Best-effort: a write failure (e.g. no `$HOME`, a
    /// read-only filesystem) just means nothing gets remembered this
    /// time, not a reason to fail the connection that already
    /// succeeded.
    #[cfg_attr(not(feature = "realtime"), allow(dead_code))]
    pub fn remember(
        input_device_name: &str,
        output_device: &str,
        period: usize,
        input_channel_offset: usize,
    ) {
        let mut config = Self::load();
        config.devices.insert(
            input_device_name.to_string(),
            DeviceSettings {
                output_device: Some(output_device.to_string()),
                period,
                input_channel_offset,
            },
        );
        config.save();
    }

    fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_unparseable_config_reads_as_empty() {
        assert!(DeviceConfig::default().get("L6 Multichannel").is_none());
        let garbage: Result<DeviceConfig, _> = serde_json::from_str("not json");
        assert!(garbage.is_err());
    }

    #[test]
    fn round_trips_through_json() {
        let mut config = DeviceConfig::default();
        config.devices.insert(
            "L6 Multichannel".to_string(),
            DeviceSettings {
                output_device: Some("L6 Analog Surround 4.0".to_string()),
                period: 256,
                input_channel_offset: 2,
            },
        );
        let json = serde_json::to_string(&config).unwrap();
        let reloaded: DeviceConfig = serde_json::from_str(&json).unwrap();
        let entry = reloaded.get("L6 Multichannel").unwrap();
        assert_eq!(entry.input_channel_offset, 2);
        assert_eq!(entry.period, 256);
        assert_eq!(
            entry.output_device.as_deref(),
            Some("L6 Analog Surround 4.0")
        );
    }

    #[test]
    fn different_devices_keep_independent_settings() {
        let mut config = DeviceConfig::default();
        config.devices.insert(
            "L6 Multichannel".to_string(),
            DeviceSettings {
                output_device: None,
                period: 256,
                input_channel_offset: 2,
            },
        );
        config.devices.insert(
            "Quantum 2626".to_string(),
            DeviceSettings {
                output_device: None,
                period: 128,
                input_channel_offset: 0,
            },
        );
        assert_eq!(
            config.get("L6 Multichannel").unwrap().input_channel_offset,
            2
        );
        assert_eq!(config.get("Quantum 2626").unwrap().input_channel_offset, 0);
    }
}
