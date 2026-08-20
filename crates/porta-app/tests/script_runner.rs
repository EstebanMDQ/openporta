//! End-to-end: drive the engine through a session script and assert on
//! the exported WAV, the way every later integration test will.

use porta_testkit::meter::rms_dbfs;
use porta_testkit::signal::sine;
use porta_testkit::spectral::{band_energy_db, dominant_freq, thd_db};
use porta_testkit::wav::write_wav_16;
use std::path::PathBuf;
use std::process::Command;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("porta-script-{name}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_stereo(path: &std::path::Path) -> (Vec<f32>, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).expect("open export");
    let spec = reader.spec();
    assert_eq!(spec.channels, 2, "export must be stereo");
    assert_eq!(spec.sample_rate, 48_000);
    assert_eq!(spec.bits_per_sample, 16);
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.expect("sample") as f32 / 32768.0)
        .collect();
    let l = samples.iter().step_by(2).copied().collect();
    let r = samples.iter().skip(1).step_by(2).copied().collect();
    (l, r)
}

fn run_script(dir: &std::path::Path, name: &str, json: &str) {
    let path = dir.join(name);
    std::fs::write(&path, json).unwrap();
    let exe = env!("CARGO_BIN_EXE_porta-app");
    let out = Command::new(exe).arg("script").arg(&path).output().unwrap();
    assert!(
        out.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn record_playback_export_roundtrip() {
    let dir = TempDir::new("roundtrip");
    write_wav_16(dir.0.join("take.wav"), &sine(1000.0, -6.0, 48_000));

    run_script(
        &dir.0,
        "session.json",
        r#"{"ops":[
            {"op":"new","dir":"cassette.porta","minutes":1,"seed":7,"character":"clean"},
            {"op":"arm","track":0},
            {"op":"record","input_wav":"take.wav"},
            {"op":"arm","track":0,"on":false},
            {"op":"seek","seconds":0},
            {"op":"export","out":"monitor.wav"},
            {"op":"play","seconds":1},
            {"op":"export","out":"playback.wav"},
            {"op":"save"}
        ]}"#,
    );

    let (l, r) = read_stereo(&dir.0.join("playback.wav"));
    assert_eq!(l.len(), 48_000, "one second of tape");
    // -6 dBFS peak sine is -9 dB RMS; center pan costs another 3 dB.
    assert!(
        (rms_dbfs(&l) - (-12.0)).abs() < 0.5,
        "left {} dBFS",
        rms_dbfs(&l)
    );
    assert!((rms_dbfs(&l) - rms_dbfs(&r)).abs() < 0.1, "center pan");
    assert!((dominant_freq(&l) - 1000.0).abs() < 5.0, "tone preserved");
    assert!(dir.0.join("cassette.porta/manifest.json").exists());
}

#[test]
fn undo_erases_the_take() {
    let dir = TempDir::new("undo");
    write_wav_16(dir.0.join("take.wav"), &sine(440.0, -3.0, 24_000));

    run_script(
        &dir.0,
        "session.json",
        r#"{"ops":[
            {"op":"new","dir":"cassette.porta","minutes":1,"seed":3,"character":"clean"},
            {"op":"arm","track":2},
            {"op":"record","input_wav":"take.wav"},
            {"op":"undo"},
            {"op":"seek","seconds":0},
            {"op":"export","out":"discard.wav"},
            {"op":"play","seconds":0.5},
            {"op":"export","out":"after_undo.wav"}
        ]}"#,
    );

    let (l, _) = read_stereo(&dir.0.join("after_undo.wav"));
    assert!(
        rms_dbfs(&l) < -80.0,
        "tape should be blank, {}",
        rms_dbfs(&l)
    );
}

#[test]
fn mixer_settings_reach_the_export() {
    let dir = TempDir::new("mixer");
    write_wav_16(dir.0.join("take.wav"), &sine(500.0, -6.0, 24_000));

    run_script(
        &dir.0,
        "session.json",
        r#"{"ops":[
            {"op":"new","dir":"cassette.porta","minutes":1,"seed":1,"character":"clean"},
            {"op":"arm","track":1},
            {"op":"record","input_wav":"take.wav"},
            {"op":"arm","track":1,"on":false},
            {"op":"pan","track":1,"value":-1.0},
            {"op":"fader","track":1,"db":-6.0},
            {"op":"seek","seconds":0},
            {"op":"export","out":"discard.wav"},
            {"op":"play","seconds":0.5},
            {"op":"export","out":"mixed.wav"}
        ]}"#,
    );

    let (l, r) = read_stereo(&dir.0.join("mixed.wav"));
    // Hard left: -9 dB RMS take, -6 dB fader, no pan loss on the left.
    assert!(
        (rms_dbfs(&l) - (-15.0)).abs() < 0.6,
        "left {}",
        rms_dbfs(&l)
    );
    // The pan moved after the take, so the first block ramps from center
    // to hard left (REQ-602). Measure once the ramp has settled.
    let settled = &r[1024..];
    assert!(
        rms_dbfs(settled) < -70.0,
        "right should be silent, {}",
        rms_dbfs(settled)
    );
}

#[test]
fn cassette_character_colours_the_tape() {
    let dir = TempDir::new("character");
    write_wav_16(dir.0.join("take.wav"), &sine(1000.0, -6.0, 48_000));

    // Same take, same seed, once clean and once through the cassette.
    for (name, character) in [("clean", "clean"), ("tape", "cassette")] {
        run_script(
            &dir.0,
            &format!("{name}.json"),
            &format!(
                r#"{{"ops":[
                    {{"op":"new","dir":"{name}.porta","minutes":1,"seed":7,"character":"{character}"}},
                    {{"op":"arm","track":0}},
                    {{"op":"record","input_wav":"take.wav"}},
                    {{"op":"arm","track":0,"on":false}},
                    {{"op":"seek","seconds":0}},
                    {{"op":"export","out":"discard_{name}.wav"}},
                    {{"op":"play","seconds":1}},
                    {{"op":"export","out":"{name}.wav"}}
                ]}}"#
            ),
        );
    }

    let (clean, _) = read_stereo(&dir.0.join("clean.wav"));
    let (tape, _) = read_stereo(&dir.0.join("tape.wav"));

    // Saturation put harmonics on the tape that the clean pass lacks.
    let clean_thd = thd_db(&clean[4096..], 1000.0, 7);
    let tape_thd = thd_db(&tape[4096..], 1000.0, 7);
    assert!(
        tape_thd > clean_thd + 15.0,
        "clean THD {clean_thd:.1} dB vs tape {tape_thd:.1} dB"
    );

    // And the noise floor rose: hiss is on the tape, not on playback.
    let quiet = |s: &[f32]| band_energy_db(&s[4096..24_000], 13_000.0, 20_000.0);
    assert!(
        quiet(&tape) > quiet(&clean) + 10.0,
        "clean floor {:.1} dB vs tape {:.1} dB",
        quiet(&clean),
        quiet(&tape)
    );
}
