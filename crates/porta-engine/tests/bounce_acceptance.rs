//! The bounce's four numeric acceptance assertions (change 001,
//! M7.12): stereo image, hiss decorrelation, master invariance, and
//! clamp engagement. Separate from `bounce.rs`'s behavioural suite -
//! these are the measured claims the proposal argued the feature on.

use porta_dsp::character::TapeCharacter;
use porta_engine::engine::Engine;
use porta_engine::tape::BusChannel;
use porta_engine::NUM_TRACKS;
use porta_testkit::signal::{silence, sine};
use porta_testkit::spectral::{band_energy_db, pearson};
use std::path::PathBuf;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("porta-bounceacc-{name}"));
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
const SETTLE: usize = 8192;

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

fn bus_f32(engine: &Engine, channel: BusChannel, len: usize) -> Vec<f32> {
    let mut out = vec![0.0; len];
    engine.tape().read_bus(channel, 0, &mut out);
    out
}

fn bus_raw(engine: &Engine, channel: BusChannel, len: usize) -> Vec<i16> {
    let mut out = vec![0i16; len];
    engine.tape().read_bus_raw(channel, 0, &mut out);
    out
}

#[test]
fn a_hard_panned_source_keeps_its_side_across_two_generations() {
    // The stereo image has to survive folding forward, not just the
    // first print - that is the whole difference from the old mono
    // bounce, and the failure mode v1 of the proposal had.
    let dir = TempDir::new("stereo-image");
    let mut e = Engine::create_with_character(&dir.0, 200_000, TapeCharacter::clean()).unwrap();
    record_onto(&mut e, 0, &sine(1000.0, -9.0, 48_000));
    e.mixer().set_pan(0, -1.0); // hard left
    bounce(&mut e, 48_000);
    for t in 0..NUM_TRACKS {
        e.mixer().set_muted(t, true);
    }
    bounce(&mut e, 48_000);

    let left = bus_f32(&e, BusChannel::Left, 48_000);
    let right = bus_f32(&e, BusChannel::Right, 48_000);
    let l_energy = band_energy_db(&left[SETTLE..40_000], 900.0, 1100.0);
    let r_energy = band_energy_db(&right[SETTLE..40_000], 900.0, 1100.0);
    assert!(
        l_energy - r_energy >= 10.0,
        "after two generations the image collapsed: L {l_energy:.1} vs R {r_energy:.1} dB \
         (need >= 10dB separation)"
    );
}

#[test]
fn the_two_channels_hiss_independently() {
    // REQ-702's per-channel seed term, measured: correlated hiss would
    // read as a centred mono noise bed instead of a stereo tape floor.
    let dir = TempDir::new("hiss-decorrelation");
    let mut e = Engine::create(&dir.0, 300_000, 31).unwrap();
    // Nothing recorded: the bus prints silence through the character
    // chain, so what lands is exactly the two channels' own hiss.
    bounce(&mut e, 240_000);
    let left = bus_f32(&e, BusChannel::Left, 240_000);
    let right = bus_f32(&e, BusChannel::Right, 240_000);
    let r = pearson(&left[SETTLE..], &right[SETTLE..]);
    assert!(
        r.abs() < 0.1,
        "the two channels' hiss correlates at {r:.3}; they must be independently seeded"
    );
}

#[test]
fn the_master_fader_never_reaches_tape() {
    // REQ-406, byte-exact: two fresh same-seed cassettes, identical op
    // sequences differing ONLY in the master position.
    let render = |name: &str, master_db: f32| {
        let dir = TempDir::new(name);
        let mut e = Engine::create(&dir.0, 200_000, 5150).unwrap();
        record_onto(&mut e, 0, &sine(440.0, -9.0, 48_000));
        record_onto(&mut e, 1, &sine(660.0, -12.0, 48_000));
        e.mixer().set_fader_db(0, -3.0);
        e.mixer().set_pan(1, 0.5);
        e.mixer().set_master_db(master_db);
        bounce(&mut e, 48_000);
        (
            bus_raw(&e, BusChannel::Left, 48_000),
            bus_raw(&e, BusChannel::Right, 48_000),
        )
    };
    let unity = render("master-unity", 0.0);
    let cut = render("master-cut", -18.0);
    assert_eq!(
        unity, cut,
        "the master fader changed what was printed to tape"
    );
    assert!(
        unity.0.iter().any(|&s| s != 0),
        "the test is vacuous if nothing was printed"
    );
}

#[test]
fn hot_generations_engage_the_quantize_clamp() {
    // Five generations of full-scale material must eventually pin at an
    // extreme value - and pin for a RUN of consecutive samples (the
    // flat-top signature of real clipping), not a lone sample sitting
    // at the boundary, which an unclipped full-scale signal reaches
    // naturally.
    //
    // Deliberately a LOW-DRIVE character. Measured while writing this:
    // the default formulation's tanh saturation (drive +9dB, makeup
    // 1/drive) caps its own output at ~0.355 of full scale, so on a
    // default cassette the i16 clamp is mathematically unreachable no
    // matter how hot the faders are - saturation gets there first, by
    // design. The clamp exists as a safety net for low-drive
    // formulations like this one, and that is the only condition under
    // which this assertion can mean anything.
    let dir = TempDir::new("clamp");
    let character = TapeCharacter {
        noise_seed: 77,
        ..TapeCharacter::clean()
    };
    let mut e = Engine::create_with_character(&dir.0, 400_000, character).unwrap();
    record_onto(&mut e, 0, &sine(220.0, 0.0, 48_000));
    record_onto(&mut e, 1, &sine(220.0, 0.0, 48_000));
    e.mixer().set_fader_db(0, 12.0);
    e.mixer().set_fader_db(1, 12.0);

    for gen in 0..5 {
        bounce(&mut e, 48_000);
        if gen == 0 {
            // Later generations re-print only the bus, compounding.
            for t in 0..NUM_TRACKS {
                e.mixer().set_muted(t, true);
            }
        }
    }

    let printed = bus_raw(&e, BusChannel::Left, 48_000);
    let mut longest = 0usize;
    let mut run = 0usize;
    let mut clipped = 0usize;
    for w in printed[SETTLE..].windows(2) {
        let extreme = w[0] == i16::MAX || w[0] == i16::MIN;
        if extreme {
            clipped += 1;
            if w[1] == w[0] {
                run += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        } else {
            run = 0;
        }
    }
    assert!(
        longest >= 4,
        "no flat-topped plateau: longest run of identical extreme samples was {longest} \
         ({clipped} samples at an extreme). A lone boundary sample is not evidence of clamping."
    );
    // Regression bound from the measured figure, not a guessed one -
    // see the assertion message if this ever trips.
    let fraction = clipped as f32 / (printed.len() - SETTLE) as f32;
    assert!(
        fraction < 0.90,
        "clipping fraction {fraction:.3} is past what five hot generations produced when \
         this bound was measured"
    );
}
