//! Per-track input channel selection (REQ-907/908/909): the parse/format
//! pair, the per-device serde config types, the connect-time routing
//! decision, and the status formatters - everything about the map that
//! is plain data.
//!
//! Deliberately ungated (no `realtime` feature cfg, no cpal types):
//! `device_config`/`realtime` are compiled out of the default-feature
//! build that CI's `cargo test --workspace` gate runs, so any test that
//! lives there never runs in CI - a hole a review of change 002 caught
//! this proposal's own verification plan falling into. Keeping the
//! logic and its tests here is what makes REQ-907/909's claims actually
//! verified on every commit; only the thin cpal wiring stays gated.
//!
//! User-facing channel numbers are 1-based, matching what `porta-app
//! probe` prints (`ch1: ...`) - the probe command is how users identify
//! channels, so the numbers they type must be the numbers it shows.
//! Internally 0-based; this module's parse/format pair is the one place
//! the conversion happens.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use porta_engine::NUM_TRACKS;

/// Which device channel (0-based) feeds each track; `None` = unassigned
/// (that track records silence).
pub type InputMap = [Option<usize>; NUM_TRACKS];

/// Parse-boundary channel ceiling. Generous beyond any real interface;
/// exists because the connect path casts channel counts through `u16`,
/// so an unbounded value like 65537 would silently truncate to channel
/// 1 - a wrong-channel recording with no error.
const MAX_CHANNEL: usize = 1024;

/// The old contiguous default: track t reads channel offset+t. Only
/// used to migrate pre-map `audio.json` entries (see `DeviceSettings`).
fn map_from_offset(offset: usize) -> InputMap {
    std::array::from_fn(|t| Some(offset + t))
}

/// Parse a user-facing map string: 1-based channels, comma-separated,
/// `-` for an unassigned track, e.g. "3,4,5,6" or "3,-,5,6". Fewer than
/// NUM_TRACKS entries leaves the rest unassigned. Errors (not silent
/// fallbacks - a typo here must block the connect, REQ-908): empty
/// input, more than NUM_TRACKS entries, channel 0 (1-based lists have
/// no channel 0; a 0 is almost certainly a mental off-by-one), channels
/// beyond MAX_CHANNEL, and anything non-numeric that isn't `-`.
pub fn parse_input_map(s: &str) -> Result<InputMap, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("input map is empty - e.g. 3,4,5,6 (see `porta-app probe`)".to_string());
    }
    let entries: Vec<&str> = s.split(',').map(str::trim).collect();
    if entries.len() > NUM_TRACKS {
        return Err(format!(
            "input map has {} entries; at most {NUM_TRACKS} tracks",
            entries.len()
        ));
    }
    let mut map: InputMap = [None; NUM_TRACKS];
    for (t, entry) in entries.iter().enumerate() {
        if *entry == "-" {
            continue;
        }
        let ch: usize = entry
            .parse()
            .map_err(|_| format!("bad input map entry {entry:?} - a channel number or -"))?;
        if ch == 0 {
            return Err("input channels are 1-based; there is no channel 0".to_string());
        }
        if ch > MAX_CHANNEL {
            return Err(format!("channel {ch} is out of range (1-{MAX_CHANNEL})"));
        }
        map[t] = Some(ch - 1);
    }
    Ok(map)
}

/// Canonical user-facing form of a map: 1-based, `-` for holes,
/// trailing unassigned tracks trimmed ("3" not "3,-,-,-"); an
/// all-unassigned map formats as a single "-" so the result always
/// parses back (the empty string deliberately doesn't).
// Used by the UI's Settings prefill; unused (not dead) in a
// realtime-only build.
#[cfg_attr(not(feature = "ui"), allow(dead_code))]
pub fn format_input_map(map: &InputMap) -> String {
    let last = map.iter().rposition(Option::is_some);
    let Some(last) = last else {
        return "-".to_string();
    };
    map[..=last]
        .iter()
        .map(|c| match c {
            Some(ch) => (ch + 1).to_string(),
            None => "-".to_string(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The connect-time routing decision (REQ-907), pure: which device
/// channel actually feeds each track given what the device provides,
/// and how many channels to ask the stream for. A track mapped beyond
/// the device's channels degrades to silence alone (`None` here),
/// without affecting the others.
pub struct RoutingPlan {
    /// Validated per-track source channel; `None` records silence.
    pub per_track: InputMap,
    /// Channels to request from the device: highest assigned + 1,
    /// clamped to what the device has. `None` = nothing assigned at
    /// all - open no input stream (same as having no input device).
    pub request_channels: Option<usize>,
}

pub fn plan_routing(map: &InputMap, device_channels: usize) -> RoutingPlan {
    let per_track: InputMap = std::array::from_fn(|t| map[t].filter(|&ch| ch < device_channels));
    let request_channels = per_track.iter().flatten().max().map(|&hi| hi + 1);
    RoutingPlan {
        per_track,
        request_channels,
    }
}

/// Compact status form for the UI's persistent connection line:
/// "[3,-,5,6]", 1-based, `-` for tracks recording silence. Takes the
/// *validated* plan, so an out-of-range assignment reads as `-` - the
/// line reports what is actually happening, not what was asked for.
// Used by the UI's status line; unused (not dead) in a realtime-only
// build.
#[cfg_attr(not(feature = "ui"), allow(dead_code))]
pub fn format_status_short(per_track: &InputMap) -> String {
    let cells: Vec<String> = per_track
        .iter()
        .map(|c| match c {
            Some(ch) => (ch + 1).to_string(),
            None => "-".to_string(),
        })
        .collect();
    format!("[{}]", cells.join(","))
}

/// Long status form for the CLI connect banner:
/// "inputs: track1<-ch3 track2<-ch4 track3<-silent track4<-ch6".
pub fn format_status_long(per_track: &InputMap) -> String {
    let cells: Vec<String> = per_track
        .iter()
        .enumerate()
        .map(|(t, c)| match c {
            Some(ch) => format!("track{}<-ch{}", t + 1, ch + 1),
            None => format!("track{}<-silent", t + 1),
        })
        .collect();
    format!("inputs: {}", cells.join(" "))
}

/// Per-device audio settings (serde form - lives here, not in
/// `device_config.rs`, so its migration tests run in the ungated CI
/// gate; the file I/O stays there). `input_channel_offset` is the
/// pre-map field: still deserialized, never serialized again, and only
/// consulted when `input_channels` is absent - keyed on *absent*, not
/// empty, so an old file and a deliberately-set new one can never be
/// confused (which is also why `parse_input_map` rejects empty input
/// outright: no in-memory state ever needs the empty case).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceSettings {
    pub output_device: Option<String>,
    pub period: usize,
    #[serde(default, skip_serializing)]
    input_channel_offset: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_channels: Option<Vec<Option<usize>>>,
}

impl DeviceSettings {
    pub fn new(output_device: Option<String>, period: usize, map: InputMap) -> Self {
        Self {
            output_device,
            period,
            input_channel_offset: None,
            input_channels: Some(map.to_vec()),
        }
    }

    /// The effective map: the stored one if present, otherwise migrated
    /// from the old offset field ([offset, offset+1, ...] - byte-for-
    /// byte the wiring the offset produced), otherwise the historical
    /// default of offset 0.
    pub fn input_map(&self) -> InputMap {
        if let Some(channels) = &self.input_channels {
            let mut map: InputMap = [None; NUM_TRACKS];
            for (t, ch) in channels.iter().take(NUM_TRACKS).enumerate() {
                map[t] = *ch;
            }
            return map;
        }
        map_from_offset(self.input_channel_offset.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceConfig {
    #[serde(default)]
    pub(crate) last_input_device: Option<String>,
    #[serde(default)]
    pub(crate) devices: HashMap<String, DeviceSettings>,
}

impl DeviceConfig {
    /// What to substitute for a blank `--in`/input field - see
    /// `device_config.rs`'s module doc comment for why that can't just
    /// be left blank.
    pub fn last_input_device(&self) -> Option<&str> {
        self.last_input_device.as_deref()
    }

    pub fn get(&self, input_device_name: &str) -> Option<&DeviceSettings> {
        self.devices.get(input_device_name)
    }

    /// Migrate every offset-era entry to map form, eagerly, at load
    /// time. Without this, re-saving the config (any successful
    /// connect does) would silently drop the legacy offset of every
    /// entry NOT being touched - `input_channel_offset` is never
    /// serialized again, and an entry that was never re-connected has
    /// no `input_channels` to write in its place, so its wiring would
    /// read as offset 0 from then on. Found on the real Pi the first
    /// time a migrated config was re-saved, not in review - the serde
    /// round-trip tests all exercised one entry at a time.
    pub fn normalize(&mut self) {
        for settings in self.devices.values_mut() {
            if settings.input_channels.is_none() {
                settings.input_channels = Some(settings.input_map().to_vec());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_map_one_based() {
        assert_eq!(
            parse_input_map("3,4,5,6").unwrap(),
            [Some(2), Some(3), Some(4), Some(5)]
        );
    }

    #[test]
    fn parses_interior_holes_and_short_lists() {
        assert_eq!(
            parse_input_map("3,-,5,6").unwrap(),
            [Some(2), None, Some(4), Some(5)]
        );
        assert_eq!(parse_input_map("3").unwrap(), [Some(2), None, None, None]);
        assert_eq!(parse_input_map(" 1 , 2 ").unwrap()[..2], [Some(0), Some(1)]);
    }

    #[test]
    fn rejects_empty_zero_junk_oversize_and_out_of_range() {
        assert!(parse_input_map("").is_err(), "empty must not parse");
        assert!(parse_input_map("  ").is_err());
        assert!(parse_input_map("0,1,2,3").is_err(), "1-based: no channel 0");
        assert!(
            parse_input_map("3;4;5;6").is_err(),
            "junk must error, not default"
        );
        assert!(
            parse_input_map("1,2,3,4,5").is_err(),
            "more entries than tracks"
        );
        assert!(
            parse_input_map("1025").is_err(),
            "beyond the u16-safety bound"
        );
        assert!(parse_input_map("65537").is_err());
    }

    #[test]
    fn format_round_trips_and_trims_trailing_holes() {
        for s in ["3,4,5,6", "3,-,5,6", "3", "-,2"] {
            let map = parse_input_map(s).unwrap();
            assert_eq!(parse_input_map(&format_input_map(&map)).unwrap(), map);
        }
        assert_eq!(format_input_map(&[Some(2), None, None, None]), "3");
        assert_eq!(format_input_map(&[None; NUM_TRACKS]), "-");
        assert_eq!(
            parse_input_map(&format_input_map(&[None; NUM_TRACKS])).unwrap(),
            [None; NUM_TRACKS],
            "even all-unassigned must format to something that parses back"
        );
    }

    #[test]
    fn routing_validates_per_track_not_as_a_prefix() {
        // The misrouting case a review caught: track 2 unassigned,
        // track 3 assigned - a positional prefix would shift track 3's
        // audio onto track 2.
        let map = parse_input_map("3,-,5,6").unwrap();
        let plan = plan_routing(&map, 12);
        assert_eq!(plan.per_track, [Some(2), None, Some(4), Some(5)]);
        assert_eq!(plan.request_channels, Some(6), "highest assigned + 1");
    }

    #[test]
    fn out_of_range_degrades_only_that_track() {
        let map = parse_input_map("1,2,9,10").unwrap();
        let plan = plan_routing(&map, 2);
        assert_eq!(plan.per_track, [Some(0), Some(1), None, None]);
        assert_eq!(plan.request_channels, Some(2));
    }

    #[test]
    fn duplicates_feed_both_tracks() {
        let map = parse_input_map("1,1").unwrap();
        let plan = plan_routing(&map, 2);
        assert_eq!(plan.per_track[..2], [Some(0), Some(0)]);
    }

    #[test]
    fn nothing_assigned_opens_no_stream() {
        let all_holes = parse_input_map("-").unwrap();
        assert_eq!(plan_routing(&all_holes, 12).request_channels, None);
        let all_out_of_range = parse_input_map("9,10,11,12").unwrap();
        assert_eq!(plan_routing(&all_out_of_range, 2).request_channels, None);
    }

    #[test]
    fn status_formats_report_the_validated_plan() {
        let plan = plan_routing(&parse_input_map("3,-,5,99").unwrap(), 6);
        assert_eq!(format_status_short(&plan.per_track), "[3,-,5,-]");
        assert_eq!(
            format_status_long(&plan.per_track),
            "inputs: track1<-ch3 track2<-silent track3<-ch5 track4<-silent"
        );
    }

    #[test]
    fn old_offset_config_migrates_to_the_same_wiring() {
        // A pre-map audio.json entry: offset only, no input_channels.
        let old =
            r#"{"output_device":"L6 Analog Surround 4.0","period":256,"input_channel_offset":2}"#;
        let settings: DeviceSettings = serde_json::from_str(old).unwrap();
        assert_eq!(
            settings.input_map(),
            [Some(2), Some(3), Some(4), Some(5)],
            "offset 2 must migrate to exactly the wiring it produced"
        );
    }

    #[test]
    fn new_config_round_trips_and_never_writes_the_old_field() {
        let settings = DeviceSettings::new(None, 256, [Some(2), None, Some(4), Some(5)]);
        let json = serde_json::to_string(&settings).unwrap();
        assert!(
            !json.contains("input_channel_offset"),
            "the legacy field must not be re-serialized"
        );
        let back: DeviceSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.input_map(), [Some(2), None, Some(4), Some(5)]);
    }

    #[test]
    fn a_file_with_both_fields_prefers_the_map() {
        let both = r#"{"output_device":null,"period":256,"input_channel_offset":2,"input_channels":[0,1,null,null]}"#;
        let settings: DeviceSettings = serde_json::from_str(both).unwrap();
        assert_eq!(settings.input_map(), [Some(0), Some(1), None, None]);
    }

    #[test]
    fn device_config_round_trips_through_json() {
        let mut config = DeviceConfig {
            last_input_device: Some("L6 Multichannel".to_string()),
            ..Default::default()
        };
        config.devices.insert(
            "L6 Multichannel".to_string(),
            DeviceSettings::new(
                Some("L6 Analog Surround 4.0".to_string()),
                256,
                [Some(2), Some(3), Some(4), Some(5)],
            ),
        );
        let json = serde_json::to_string(&config).unwrap();
        let reloaded: DeviceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded.last_input_device(), Some("L6 Multichannel"));
        let entry = reloaded.get("L6 Multichannel").unwrap();
        assert_eq!(entry.input_map(), [Some(2), Some(3), Some(4), Some(5)]);
        assert_eq!(entry.period, 256);
    }

    #[test]
    fn resaving_a_config_keeps_untouched_entries_wiring() {
        // Regression for a real bug found on the Pi, not in review: a
        // connect re-saves the whole config, and an offset-era entry
        // NOT being touched by that connect lost its wiring - the
        // legacy field is never re-serialized and nothing had written a
        // map in its place. normalize() (called by load) migrates every
        // entry eagerly so a round trip preserves all of them.
        let two_old_entries = r#"{
            "last_input_device": "L6 Multichannel",
            "devices": {
                "L6 Multichannel": {"output_device": null, "period": 256, "input_channel_offset": 2},
                "default_input":   {"output_device": null, "period": 256, "input_channel_offset": 2}
            }
        }"#;
        let mut config: DeviceConfig = serde_json::from_str(two_old_entries).unwrap();
        config.normalize();
        // Simulate a connect touching only the L6 entry, then a save.
        config.devices.insert(
            "L6 Multichannel".to_string(),
            DeviceSettings::new(None, 256, [Some(2), Some(3), Some(4), Some(5)]),
        );
        let json = serde_json::to_string(&config).unwrap();
        let reloaded: DeviceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            reloaded.get("default_input").unwrap().input_map(),
            [Some(2), Some(3), Some(4), Some(5)],
            "the untouched entry's offset wiring must survive the round trip"
        );
    }

    #[test]
    fn missing_or_unparseable_config_reads_as_empty() {
        let config = DeviceConfig::default();
        assert!(config.get("L6 Multichannel").is_none());
        assert!(config.last_input_device().is_none());
        let garbage: Result<DeviceConfig, _> = serde_json::from_str("not json");
        assert!(garbage.is_err());
    }
}
