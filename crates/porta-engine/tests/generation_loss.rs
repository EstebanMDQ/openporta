//! REQ-403, the product's acceptance test: successive generations of the
//! same material must get progressively duller and noisier, the way a
//! real 4-track does when you bounce to free up tracks.
//!
//! Since change 001 this measures REAL bounce generations, not
//! track-to-track passes standing in for them: prime the bus with one
//! bounce, then mute all four tracks and bounce again and again, so
//! each generation re-prints only the bus's own prior content through
//! the character chain. Identical input conditions per generation,
//! which is what makes the comparison across them meaningful.

use porta_engine::engine::Engine;
use porta_engine::tape::BusChannel;
use porta_engine::NUM_TRACKS;
use porta_testkit::meter::rms_dbfs;
use porta_testkit::signal::{silence, sine};
use porta_testkit::spectral::band_energy_db;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("porta-genloss-{name}"));
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
/// Tone for the first half, silence for the second: the tone shows
/// bandwidth loss, the silence shows the noise floor rising.
const TONE_LEN: usize = 48_000;
const TAIL_LEN: usize = 48_000;
const TAKE_LEN: usize = TONE_LEN + TAIL_LEN;

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
        let n = engine.process_block(&inputs, &mut l[..chunk.len()], &mut r[..chunk.len()]);
        if n == 0 {
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

/// Roll a bounce pass over `samples` from position 0.
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

fn read_bus(engine: &Engine, len: usize) -> Vec<f32> {
    let mut out = vec![0.0; len];
    engine.tape().read_bus(BusChannel::Left, 0, &mut out);
    out
}

#[test]
fn generations_get_duller_and_noisier() {
    let dir = TempDir::new("three-generations");
    // 8kHz sits below the 11kHz corner, so the first pass does not gut
    // it; what we measure is the loss compounding pass over pass.
    let mut source = sine(8000.0, -6.0, TONE_LEN);
    source.extend(silence(TAIL_LEN));

    let mut engine = Engine::create(&dir.0, 200_000, 7).unwrap();
    record_onto(&mut engine, 0, &source);

    // Generation 1 primes the bus from the track. From here on the
    // tracks are muted, so every later generation re-prints only what
    // the bus already held - the same input condition each time, which
    // is what makes the three measurements comparable.
    bounce(&mut engine, TAKE_LEN);
    for t in 0..NUM_TRACKS {
        engine.mixer().set_muted(t, true);
    }
    bounce(&mut engine, TAKE_LEN);
    let gen2 = read_bus(&engine, TAKE_LEN);
    bounce(&mut engine, TAKE_LEN);
    let gen3 = read_bus(&engine, TAKE_LEN);
    bounce(&mut engine, TAKE_LEN);
    let gen4 = read_bus(&engine, TAKE_LEN);

    // Skip the punch crossfade and delay-line fill at the head of each
    // generation, and stay clear of the tone/silence boundary. The
    // windows are generous rather than sample-aligned: each generation
    // adds ~480 samples of flutter delay, an accepted drift this test
    // deliberately does not assert alignment against.
    let tone = |g: &[f32]| band_energy_db(&g[8192..TONE_LEN - 8192], 7000.0, 9000.0);
    let floor = |g: &[f32]| rms_dbfs(&g[TONE_LEN + 8192..TAKE_LEN - 8192]);

    let tones = [tone(&gen2), tone(&gen3), tone(&gen4)];
    let floors = [floor(&gen2), floor(&gen3), floor(&gen4)];

    assert!(
        tones[1] < tones[0] && tones[2] < tones[1],
        "8kHz energy must fall each generation, got {tones:?}"
    );
    assert!(
        floors[1] > floors[0] && floors[2] > floors[1],
        "noise floor must rise each generation, got {floors:?}"
    );
    // And the effect is audible, not just measurable.
    assert!(
        tones[0] - tones[2] > 3.0,
        "three generations only lost {:.1} dB of 8kHz",
        tones[0] - tones[2]
    );
    assert!(
        floors[2] - floors[0] > 2.0,
        "noise floor only rose {:.1} dB over three generations",
        floors[2] - floors[0]
    );
}

#[test]
fn bounce_generations_are_reproducible() {
    // REQ-702 across the bounce path specifically.
    let render = |name: &str| {
        let dir = TempDir::new(name);
        let mut engine = Engine::create(&dir.0, 200_000, 1979).unwrap();
        record_onto(&mut engine, 0, &sine(1000.0, -6.0, 24_000));
        bounce(&mut engine, 24_000);
        for t in 0..NUM_TRACKS {
            engine.mixer().set_muted(t, true);
        }
        bounce(&mut engine, 24_000);
        read_bus(&engine, 24_000)
    };
    assert_eq!(
        render("bounce-repro-a"),
        render("bounce-repro-b"),
        "same cassette seed must produce identical bounce generations"
    );
}

#[test]
fn generations_are_reproducible() {
    let source = sine(1000.0, -6.0, 24_000);
    let render = |name: &str| {
        let dir = TempDir::new(name);
        let mut engine = Engine::create(&dir.0, 100_000, 42).unwrap();
        record_onto(&mut engine, 0, &source);
        let gen1 = read_track(&engine, 0, 24_000);
        record_onto(&mut engine, 1, &gen1);
        read_track(&engine, 1, 24_000)
    };
    assert_eq!(
        render("repro-a"),
        render("repro-b"),
        "same cassette seed must produce identical generations (REQ-702)"
    );
}

#[test]
fn each_pass_wobbles_independently() {
    // Two tracks recorded from the same source on the same cassette must
    // not share a flutter pattern; if they did, bouncing would not
    // compound wobble the way real generations do (REQ-304).
    let dir = TempDir::new("decorrelated");
    let source = sine(2000.0, -6.0, 48_000);
    let mut engine = Engine::create(&dir.0, 100_000, 5).unwrap();
    record_onto(&mut engine, 0, &source);
    record_onto(&mut engine, 1, &source);
    let a = read_track(&engine, 0, 48_000);
    let b = read_track(&engine, 1, 48_000);
    let diff: Vec<f32> = a[8192..]
        .iter()
        .zip(&b[8192..])
        .map(|(x, y)| x - y)
        .collect();
    assert!(
        rms_dbfs(&diff) > -30.0,
        "passes are too similar ({:.1} dB difference); flutter is not \
         decorrelating between passes",
        rms_dbfs(&diff)
    );
}
