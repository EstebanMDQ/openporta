//! Simulated realtime: drive the engine the way an audio callback will,
//! with commands arriving between blocks at arbitrary block sizes, and
//! prove the result is identical whatever the device block size is
//! (REQ-203).
//!
//! This is what makes M4 verifiable without audio hardware. The cpal
//! adapter's job is then only to move bytes; the behaviour it depends on
//! is pinned here.
//!
//! One rule the adapter must follow, established here: **split the
//! callback buffer at command boundaries**. Applying commands only at
//! device block boundaries makes the moment a command takes effect
//! depend on the period size, so a stop at sample 36000 lands at 36032
//! with 64-frame periods and at 36000 with 480-frame ones, and the two
//! renders differ in length. Processing up to the next pending command,
//! applying it, then continuing costs one extra loop iteration and makes
//! command timing sample-accurate on any device.

use porta_dsp::character::TapeCharacter;
use porta_engine::command::{apply, Command, EngineEvent};
use porta_engine::engine::Engine;
use porta_engine::transport::TransportState;
use porta_engine::NUM_TRACKS;
use porta_testkit::signal::sine;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("porta-rtsim-{name}"));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const TAPE: usize = 96_000;

/// A command scheduled to fire once the transport reaches `at_sample`.
struct Scheduled {
    at_sample: usize,
    command: Command,
}

/// Run `input` through the engine in `block` sized chunks, applying
/// scheduled commands at block boundaries the way a real callback would.
/// Returns the stereo left channel and the events observed.
fn simulate(
    engine: &mut Engine,
    input: &[f32],
    block: usize,
    schedule: &[Scheduled],
) -> (Vec<f32>, Vec<EngineEvent>) {
    let mut out = Vec::new();
    let mut events = Vec::new();
    let mut l = vec![0.0; block];
    let mut r = vec![0.0; block];
    let mut pending = 0usize;
    let mut consumed = 0usize;

    loop {
        // Commands are applied between callbacks, at whatever block
        // boundary the audio device happens to give us.
        while pending < schedule.len() && schedule[pending].at_sample <= engine.playhead() {
            let c = schedule[pending].command;
            if c.is_blocking() && engine.state() != TransportState::Stopped {
                events.push(EngineEvent::Rejected(c));
            } else {
                apply(engine, c).expect("apply command");
                events.push(EngineEvent::State(engine.state()));
            }
            pending += 1;
        }
        if pending >= schedule.len() && engine.state() == TransportState::Stopped {
            break;
        }

        // Never process past a pending command: split the buffer there
        // so the command lands on the sample it was scheduled for,
        // whatever the device period happens to be.
        let mut limit = block;
        if let Some(next) = schedule.get(pending) {
            let until = next.at_sample.saturating_sub(engine.playhead());
            if until > 0 {
                limit = limit.min(until);
            }
        }
        let end = (consumed + limit).min(input.len());
        let chunk = &input[consumed..end];
        if chunk.is_empty() {
            break;
        }
        let inputs: [&[f32]; NUM_TRACKS] = [chunk, chunk, chunk, chunk];
        let n = engine.process_block(&inputs, &mut l[..chunk.len()], &mut r[..chunk.len()]);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&l[..n]);
        consumed = end;
    }
    (out, events)
}

fn session(dir: &std::path::Path, block: usize) -> Vec<f32> {
    let mut e = Engine::create(dir, TAPE, 31).unwrap();
    let input = sine(500.0, -9.0, 48_000);
    let schedule = [
        Scheduled {
            at_sample: 0,
            command: Command::Arm { track: 0, on: true },
        },
        Scheduled {
            at_sample: 0,
            command: Command::Record,
        },
        Scheduled {
            at_sample: 12_000,
            command: Command::Fader { track: 0, db: -6.0 },
        },
        Scheduled {
            at_sample: 24_000,
            command: Command::Pan {
                track: 0,
                value: -0.5,
            },
        },
        Scheduled {
            at_sample: 36_000,
            command: Command::Stop,
        },
    ];
    simulate(&mut e, &input, block, &schedule).0
}

#[test]
fn block_size_does_not_change_the_render() {
    // 64 is an aggressive USB period, 480 is 10ms, 1024 is a lazy one.
    let a = session(&TempDir::new("blk64").0, 64);
    let b = session(&TempDir::new("blk480").0, 480);
    let c = session(&TempDir::new("blk1024").0, 1024);
    assert_eq!(a.len(), b.len(), "64 vs 480 produced different lengths");
    assert_eq!(b.len(), c.len(), "480 vs 1024 produced different lengths");
    assert_eq!(a, b, "64 vs 480 diverged");
    assert_eq!(b, c, "480 vs 1024 diverged");
}

#[test]
fn commands_landing_mid_block_still_take_effect() {
    // 37 is deliberately not a divisor of anything in the schedule, so
    // every command lands mid-stream.
    let odd = session(&TempDir::new("odd").0, 37);
    let even = session(&TempDir::new("even").0, 512);
    assert_eq!(odd, even, "odd block size changed the result");
}

#[test]
fn blocking_commands_are_rejected_while_rolling() {
    let dir = TempDir::new("blocking");
    let mut e = Engine::create_with_character(&dir.0, TAPE, TapeCharacter::clean()).unwrap();
    let input = sine(440.0, -9.0, 24_000);
    let schedule = [
        Scheduled {
            at_sample: 0,
            command: Command::Arm { track: 0, on: true },
        },
        Scheduled {
            at_sample: 0,
            command: Command::Record,
        },
        // Arrives while the tape is rolling: must be refused, not
        // allowed to block the callback on disk I/O.
        Scheduled {
            at_sample: 8_000,
            command: Command::Save,
        },
        Scheduled {
            at_sample: 16_000,
            command: Command::Stop,
        },
        Scheduled {
            at_sample: 16_000,
            command: Command::Save,
        },
    ];
    let (_, events) = simulate(&mut e, &input, 256, &schedule);
    assert!(
        events.contains(&EngineEvent::Rejected(Command::Save)),
        "save during playback should have been rejected, got {events:?}"
    );
    // The one issued while stopped went through.
    assert!(dir.0.join("manifest.json").exists());
}

#[test]
fn transport_commands_move_the_playhead_predictably() {
    let dir = TempDir::new("transport");
    let mut e = Engine::create(&dir.0, TAPE, 1).unwrap();
    apply(&mut e, Command::Seek { sample: 40_000 }).unwrap();
    assert_eq!(e.playhead(), 40_000);
    apply(&mut e, Command::Rewind { samples: 10_000 }).unwrap();
    assert_eq!(e.playhead(), 30_000);
    apply(&mut e, Command::FastForward { samples: 5_000 }).unwrap();
    assert_eq!(e.playhead(), 35_000);
    apply(
        &mut e,
        Command::Rewind {
            samples: usize::MAX,
        },
    )
    .unwrap();
    assert_eq!(e.playhead(), 0);
    apply(
        &mut e,
        Command::FastForward {
            samples: usize::MAX,
        },
    )
    .unwrap();
    assert_eq!(e.playhead(), TAPE, "fast-forward clamps at the tape end");
}
