//! Bounce: a real-time stereo pass onto the dedicated bounce bus
//! (REQ-401 through REQ-409, change 001). Rewritten wholesale from the
//! mono-sum-onto-track-4 suite this replaces - every assertion here is
//! about the new semantics, not a port of the old ones.

use porta_dsp::character::TapeCharacter;
use porta_engine::engine::Engine;
use porta_engine::tape::BusChannel;
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

fn raw_track(engine: &Engine, track: usize, len: usize) -> Vec<i16> {
    let mut out = vec![0i16; len];
    engine.tape().read_raw(track, 0, &mut out);
    out
}

/// Three distinct tones on tracks 1-3, so a bounce can be identified
/// by its spectrum.
fn load_three_tones(engine: &mut Engine, len: usize) {
    record_onto(engine, 0, &sine(300.0, -12.0, len));
    record_onto(engine, 1, &sine(700.0, -12.0, len));
    record_onto(engine, 2, &sine(1500.0, -12.0, len));
}

/// Roll a bounce pass over `samples` from position 0 - arm the bus,
/// press Record, let the transport run. There is no batch bounce
/// command any more; this IS a bounce (REQ-401).
fn bounce(engine: &mut Engine, samples: usize) {
    engine.seek(0);
    engine.set_bus_armed(true);
    engine.record();
    let quiet = silence(BLOCK);
    let inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    let mut done = 0;
    while done < samples {
        let want = BLOCK.min(samples - done);
        let n = engine.process_block(&inputs, &mut l[..want], &mut r[..want]);
        if n == 0 {
            break;
        }
        done += n;
    }
    engine.stop();
    engine.set_bus_armed(false);
}

fn read_bus(engine: &Engine, channel: BusChannel, len: usize) -> Vec<f32> {
    let mut out = vec![0.0; len];
    engine.tape().read_bus(channel, 0, &mut out);
    out
}

fn raw_bus(engine: &Engine, channel: BusChannel, len: usize) -> Vec<i16> {
    let mut out = vec![0i16; len];
    engine.tape().read_bus_raw(channel, 0, &mut out);
    out
}

const SETTLE: usize = 4096;

#[test]
fn bounce_prints_the_source_tracks_onto_the_bus() {
    let dir = TempDir::new("prints");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    load_three_tones(&mut e, 48_000);
    bounce(&mut e, 48_000);

    let left = read_bus(&e, BusChannel::Left, 48_000);
    // All three sources are present in the printed mix.
    for freq in [300.0, 700.0, 1500.0] {
        let energy = band_energy_db(&left[SETTLE..], freq - 40.0, freq + 40.0);
        assert!(
            energy > -50.0,
            "{freq}Hz missing from the print: {energy:.1} dB"
        );
    }
    assert!(
        rms_dbfs(&left[SETTLE..]) > -40.0,
        "the bus should not be silent"
    );
}

#[test]
fn bounce_respects_faders_and_honours_pans() {
    // REQ-603 is DELETED: a bounce is a stereo print, so pans are
    // honoured rather than ignored. This test is the inverse of the one
    // it replaces.
    let dir = TempDir::new("faders-pans");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    record_onto(&mut e, 0, &sine(400.0, -12.0, 48_000));
    e.mixer().set_pan(0, -1.0); // hard left
    bounce(&mut e, 48_000);

    let l = rms_dbfs(&read_bus(&e, BusChannel::Left, 48_000)[SETTLE..]);
    let r = rms_dbfs(&read_bus(&e, BusChannel::Right, 48_000)[SETTLE..]);
    assert!(
        l - r > 20.0,
        "a hard-panned source must print to one side: L {l:.1} vs R {r:.1}"
    );

    // And the fader scales what gets printed.
    let dir2 = TempDir::new("faders-pans-2");
    let mut q = Engine::create_with_character(&dir2.0, TAPE, TapeCharacter::clean()).unwrap();
    record_onto(&mut q, 0, &sine(400.0, -12.0, 48_000));
    q.mixer().set_fader_db(0, -12.0);
    bounce(&mut q, 48_000);
    let loud = rms_dbfs(&read_bus(&e, BusChannel::Left, 48_000)[SETTLE..]);
    let quiet = rms_dbfs(&read_bus(&q, BusChannel::Left, 48_000)[SETTLE..]);
    assert!(
        loud - quiet > 6.0,
        "the fader must scale the print: {loud:.1} vs {quiet:.1}"
    );
}

#[test]
fn bounce_excludes_muted_tracks() {
    let dir = TempDir::new("muted");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    load_three_tones(&mut e, 48_000);
    e.mixer().set_muted(1, true); // the 700Hz tone
    bounce(&mut e, 48_000);

    let left = read_bus(&e, BusChannel::Left, 48_000);
    let muted = band_energy_db(&left[SETTLE..], 660.0, 740.0);
    let present = band_energy_db(&left[SETTLE..], 260.0, 340.0);
    assert!(
        present - muted > 20.0,
        "a muted track must stay out of the print: 300Hz {present:.1} vs 700Hz {muted:.1}"
    );
}

#[test]
fn bounce_leaves_the_source_tracks_alone() {
    // REQ-306: a bounce reads tracks, it never writes them. The old
    // bounce consumed track 4; this one consumes nothing.
    let dir = TempDir::new("sources-untouched");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    load_three_tones(&mut e, 48_000);
    let before: Vec<Vec<i16>> = (0..NUM_TRACKS).map(|t| raw_track(&e, t, 48_000)).collect();
    bounce(&mut e, 48_000);
    for (t, want) in before.iter().enumerate() {
        assert_eq!(&raw_track(&e, t, 48_000), want, "track {t} was modified");
    }
}

#[test]
fn bounce_is_undoable() {
    let dir = TempDir::new("undoable");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    load_three_tones(&mut e, 48_000);
    let before = (
        raw_bus(&e, BusChannel::Left, 48_000),
        raw_bus(&e, BusChannel::Right, 48_000),
    );
    bounce(&mut e, 48_000);
    let after = (
        raw_bus(&e, BusChannel::Left, 48_000),
        raw_bus(&e, BusChannel::Right, 48_000),
    );
    assert_ne!(after, before);

    e.undo().unwrap();
    assert_eq!(
        (
            raw_bus(&e, BusChannel::Left, 48_000),
            raw_bus(&e, BusChannel::Right, 48_000)
        ),
        before,
        "one undo must revert both channels byte-exactly"
    );
    e.redo().unwrap();
    assert_eq!(
        (
            raw_bus(&e, BusChannel::Left, 48_000),
            raw_bus(&e, BusChannel::Right, 48_000)
        ),
        after
    );
}

#[test]
fn bounce_folds_the_previous_generation_forward() {
    // REQ-407: the bus's prior content is read before it is written, so
    // bouncing again keeps what was already printed rather than
    // replacing it. This is what the old one-shot bounce could not do.
    let dir = TempDir::new("fold-forward");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    record_onto(&mut e, 0, &sine(300.0, -12.0, 48_000));
    bounce(&mut e, 48_000);

    // Second generation: a different source, tracks re-armed and the
    // first generation still on the bus.
    record_onto(&mut e, 1, &sine(1500.0, -12.0, 48_000));
    bounce(&mut e, 48_000);

    let left = read_bus(&e, BusChannel::Left, 48_000);
    let first = band_energy_db(&left[SETTLE..], 260.0, 340.0);
    let second = band_energy_db(&left[SETTLE..], 1460.0, 1540.0);
    assert!(
        first > -50.0,
        "the first generation must survive the second bounce: {first:.1} dB"
    );
    assert!(
        second > -50.0,
        "the second source must be printed too: {second:.1} dB"
    );
}

#[test]
fn bounce_applies_the_character_again() {
    // REQ-402: the print goes through the character chain, so a bounced
    // copy is measurably noisier than the source it came from.
    let dir = TempDir::new("character");
    let mut e = Engine::create(&dir.0, TAPE, 4242).unwrap();
    record_onto(&mut e, 0, &silence(48_000));
    bounce(&mut e, 48_000);
    let printed = read_bus(&e, BusChannel::Left, 48_000);
    let floor = rms_dbfs(&printed[SETTLE..]);
    assert!(
        floor > -90.0,
        "the character chain should add an audible noise floor: {floor:.1} dBFS"
    );
}

#[test]
fn bounce_is_refused_while_a_track_pass_is_open() {
    // REQ-405: arming the bus clears every track arm, so a bounce can
    // never overlap a live input pass. Enforced in the engine, not left
    // to callers to sequence.
    let dir = TempDir::new("exclusion");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    e.set_armed(0, true);
    e.set_bus_armed(true);
    assert!(!e.is_armed(0), "arming the bus must clear the track arm");
    assert!(e.is_bus_armed());
    e.set_armed(2, true);
    assert!(!e.is_bus_armed(), "arming a track must clear the bus arm");
}

#[test]
fn bounce_is_reproducible() {
    // REQ-702: same cassette seed, same ops, same bytes.
    let run = |name: &str| {
        let dir = TempDir::new(name);
        let mut e = Engine::create(&dir.0, TAPE, 1979).unwrap();
        record_onto(&mut e, 0, &sine(300.0, -12.0, 24_000));
        e.mixer().set_pan(0, 0.3);
        bounce(&mut e, 24_000);
        (
            raw_bus(&e, BusChannel::Left, 24_000),
            raw_bus(&e, BusChannel::Right, 24_000),
        )
    };
    let a = run("repro-a");
    let b = run("repro-b");
    assert_eq!(a, b, "identical sessions must print identical bytes");
    assert_ne!(a.0, a.1, "and the two channels must genuinely differ");
}

#[test]
fn a_bounced_mix_still_identifies_its_loudest_source() {
    let dir = TempDir::new("dominant");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    record_onto(&mut e, 0, &sine(300.0, -6.0, 48_000));
    record_onto(&mut e, 1, &sine(1500.0, -24.0, 48_000));
    bounce(&mut e, 48_000);
    let left = read_bus(&e, BusChannel::Left, 48_000);
    let dominant = dominant_freq(&left[SETTLE..]);
    assert!(
        (dominant - 300.0).abs() < 30.0,
        "expected the loud 300Hz source to dominate, got {dominant:.0}Hz"
    );
}
