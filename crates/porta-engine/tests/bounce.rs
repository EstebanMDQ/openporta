//! Bounce: tracks 1-3 summed down to track 4 (REQ-401 through REQ-403).

use porta_dsp::character::TapeCharacter;
use porta_engine::engine::{Engine, EngineError};
use porta_engine::NUM_TRACKS;
use porta_testkit::meter::rms_dbfs;
use porta_testkit::signal::{silence, sine};
use porta_testkit::spectral::{band_energy_db, dominant_freq};
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("porta-bounce-{name}"));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const BLOCK: usize = 512;
const TAPE: usize = 96_000;

fn record_onto(engine: &mut Engine, track: usize, source: &[f32]) {
    for t in 0..NUM_TRACKS {
        engine.set_armed(t, t == track);
    }
    engine.seek(0);
    engine.record();
    let quiet = silence(BLOCK);
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    let mut fed = 0;
    while fed < source.len() {
        let end = (fed + BLOCK).min(source.len());
        let chunk = &source[fed..end];
        let inputs: [&[f32]; NUM_TRACKS] = std::array::from_fn(|t| {
            if t == track {
                chunk
            } else {
                &quiet[..chunk.len()]
            }
        });
        if engine.process_block(&inputs, &mut l[..chunk.len()], &mut r[..chunk.len()]) == 0 {
            break;
        }
        fed = end;
    }
    engine.stop();
    engine.set_armed(track, false);
}

fn read_track(engine: &Engine, track: usize, len: usize) -> Vec<f32> {
    let mut out = vec![0.0; len];
    engine.tape().read(track, 0, &mut out);
    out
}

fn raw_track(engine: &Engine, track: usize, len: usize) -> Vec<i16> {
    let mut out = vec![0i16; len];
    engine.tape().read_raw(track, 0, &mut out);
    out
}

/// Three distinct tones on tracks 1-3, so the bounce can be identified
/// by its spectrum.
fn load_three_tones(engine: &mut Engine, len: usize) {
    record_onto(engine, 0, &sine(300.0, -12.0, len));
    record_onto(engine, 1, &sine(700.0, -12.0, len));
    record_onto(engine, 2, &sine(1500.0, -12.0, len));
}

#[test]
fn bounce_sums_the_source_tracks_onto_track_four() {
    let dir = TempDir::new("sums");
    let mut e = Engine::create(&dir.0, TAPE, 7).unwrap();
    load_three_tones(&mut e, 48_000);
    e.bounce().unwrap();

    let dest = read_track(&e, 3, 48_000);
    let window = &dest[8192..40_000];
    for freq in [300.0, 700.0, 1500.0] {
        let energy = band_energy_db(window, freq - 60.0, freq + 60.0);
        let away = band_energy_db(window, freq + 2000.0, freq + 2200.0);
        assert!(
            energy > away + 20.0,
            "{freq} Hz missing from the bounce ({energy:.1} vs {away:.1} dB)"
        );
    }
}

#[test]
fn bounce_leaves_the_source_tracks_alone() {
    let dir = TempDir::new("sources");
    let mut e = Engine::create(&dir.0, TAPE, 7).unwrap();
    load_three_tones(&mut e, 48_000);
    let before: Vec<Vec<i16>> = (0..3).map(|t| raw_track(&e, t, TAPE)).collect();
    e.bounce().unwrap();
    for (t, b) in before.iter().enumerate() {
        assert_eq!(&raw_track(&e, t, TAPE), b, "track {t} was modified");
    }
}

#[test]
fn bounce_respects_faders_and_ignores_pans() {
    let dir = TempDir::new("faders");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    record_onto(&mut e, 0, &sine(1000.0, -12.0, 48_000));

    // Pan hard left: the mono bounce bus must not care (REQ-603).
    e.mixer().set_pan(0, -1.0);
    e.bounce().unwrap();
    let loud = rms_dbfs(&read_track(&e, 3, 48_000)[8192..40_000]);

    e.undo().unwrap();
    e.mixer().set_fader_db(0, -12.0);
    e.bounce().unwrap();
    let quiet = rms_dbfs(&read_track(&e, 3, 48_000)[8192..40_000]);

    assert!(
        ((loud - quiet) - 12.0).abs() < 1.0,
        "fader change moved the bounce by {:.1} dB, expected 12",
        loud - quiet
    );
}

#[test]
fn bounce_excludes_muted_tracks() {
    let dir = TempDir::new("mute");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    record_onto(&mut e, 0, &sine(1000.0, -12.0, 48_000));

    e.bounce().unwrap();
    let audible = read_track(&e, 3, 48_000)[8192..40_000].to_vec();
    assert!(
        rms_dbfs(&audible) > -80.0,
        "unmuted track should show up in the bounce"
    );

    e.undo().unwrap();
    e.mixer().set_muted(0, true);
    e.bounce().unwrap();
    let silent = read_track(&e, 3, 48_000)[8192..40_000].to_vec();
    assert!(
        rms_dbfs(&silent) < -80.0,
        "a muted source track should be left out of the bounce, got {}",
        rms_dbfs(&silent)
    );
}

#[test]
fn bounce_is_undoable() {
    let dir = TempDir::new("undo");
    let mut e = Engine::create(&dir.0, TAPE, 7).unwrap();
    load_three_tones(&mut e, 48_000);
    record_onto(&mut e, 3, &sine(2500.0, -12.0, 48_000));
    let before = raw_track(&e, 3, TAPE);

    e.bounce().unwrap();
    assert_ne!(raw_track(&e, 3, TAPE), before, "bounce must overwrite");

    e.undo().unwrap();
    assert_eq!(
        raw_track(&e, 3, TAPE),
        before,
        "undo must restore track 4 byte-exactly"
    );
}

#[test]
fn bounce_applies_the_character_again() {
    // A bounced track must be duller than the same material one
    // generation earlier (REQ-402/403).
    let dir = TempDir::new("generation");
    let mut e = Engine::create(&dir.0, TAPE, 7).unwrap();
    record_onto(&mut e, 0, &sine(8000.0, -12.0, 48_000));
    e.bounce().unwrap();

    let source = read_track(&e, 0, 48_000);
    let bounced = read_track(&e, 3, 48_000);
    let energy = |s: &[f32]| band_energy_db(&s[8192..40_000], 7000.0, 9000.0);
    assert!(
        energy(&source) - energy(&bounced) > 2.0,
        "bounce only lost {:.1} dB at 8kHz",
        energy(&source) - energy(&bounced)
    );
    // Still the same note, within the wobble the character prescribes:
    // 12 cents at 8kHz is about 55Hz of smear, applied twice.
    assert!(
        (dominant_freq(&bounced[8192..40_000]) - 8000.0).abs() < 120.0,
        "bounce landed at {:.0} Hz, expected about 8000",
        dominant_freq(&bounced[8192..40_000])
    );
}

#[test]
fn bounce_is_refused_while_rolling() {
    let dir = TempDir::new("rolling");
    let mut e = Engine::create(&dir.0, TAPE, 7).unwrap();
    e.seek(0);
    e.play();
    assert!(matches!(e.bounce(), Err(EngineError::NotStopped(_))));
}

#[test]
fn bounce_is_reproducible() {
    let render = |name: &str| {
        let dir = TempDir::new(name);
        let mut e = Engine::create(&dir.0, TAPE, 99).unwrap();
        load_three_tones(&mut e, 24_000);
        e.bounce().unwrap();
        raw_track(&e, 3, 48_000)
    };
    assert_eq!(render("repro-a"), render("repro-b"));
}
