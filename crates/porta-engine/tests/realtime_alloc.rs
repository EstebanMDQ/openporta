//! REQ-902, made load-bearing instead of inferred.
//!
//! Every other realtime-safety guarantee in this project is argued
//! structurally - pre-reserved pools, `mem::take` give-backs, chains
//! reseeded in place - and structural arguments are exactly what three
//! separate shipped bugs slipped past. This harness counts real
//! allocations through a global allocator while the simulated realtime
//! path runs, so a regression to a completely different implementation
//! is caught by the count rather than by whether anyone remembered to
//! reason about it.
//!
//! Deliberately a whole test binary of its own: a counting global
//! allocator is process-wide, so it must not be installed alongside
//! tests that allocate freely on other threads.
//!
//! This file is the workspace's single exception to `unsafe_code =
//! "deny"`, and the exception is scoped to this file rather than
//! loosened anywhere else. Implementing `GlobalAlloc` cannot be done
//! in safe Rust - the trait is unsafe by definition - and the deny
//! exists to keep unsafe out of the shipped product, which this is
//! not: it is test-only, it never compiles into a release binary, and
//! every unsafe block below is a direct delegation to `System` with
//! nothing else in it. Refusing the exception here would mean giving
//! up the only mechanism that can actually prove REQ-902 rather than
//! argue it.
#![allow(unsafe_code)]

use porta_dsp::character::TapeCharacter;
use porta_engine::engine::Engine;
use porta_engine::NUM_TRACKS;
use porta_testkit::signal::{silence, sine};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCS: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNTING.load(Ordering::Relaxed) {
            DEALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Serializes the whole file. The counting allocator is process-wide,
/// so two tests running concurrently would each count the other's
/// allocations - which is exactly what happened the first time this
/// was written, and produced numbers that looked like real bugs.
/// Every test here holds this for its entire body, setup included,
/// not just around `count`.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Run `f` with allocation counting on, returning (allocs, deallocs).
/// Everything `f` needs must already exist - the point is to measure
/// only what `f` itself does.
fn count<R>(f: impl FnOnce() -> R) -> (usize, usize, R) {
    ALLOCS.store(0, Ordering::Relaxed);
    DEALLOCS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let out = f();
    COUNTING.store(false, Ordering::Relaxed);
    (
        ALLOCS.load(Ordering::Relaxed),
        DEALLOCS.load(Ordering::Relaxed),
        out,
    )
}

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("porta-rtalloc-{name}"));
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

#[test]
fn the_harness_itself_detects_an_allocation() {
    let _serial = serial();
    // Guard against the whole suite silently passing because counting
    // was never actually on.
    let (allocs, _, _) = count(|| {
        let v: Vec<u8> = Vec::with_capacity(4096);
        std::hint::black_box(&v);
    });
    assert!(
        allocs > 0,
        "the counting allocator did not observe a deliberate allocation; \
         every assertion in this file would be vacuous"
    );
}

#[test]
fn an_ordinary_track_pass_does_not_allocate_on_the_realtime_path() {
    let _serial = serial();
    let dir = TempDir::new("track-pass");
    let mut e = Engine::create_with_character(&dir.0, 200_000, TapeCharacter::clean()).unwrap();
    let tone = sine(440.0, -6.0, BLOCK);
    let quiet = silence(BLOCK);
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);

    // Warm up off-measurement: first-touch effects (lazily grown Vecs
    // in the journal's own bookkeeping, etc.) are not what this is
    // about, and counting them would make the test about warm-up order
    // rather than about the steady state a real session runs in.
    e.set_armed(0, true);
    e.seek(0);
    e.record();
    let inputs: [&[f32]; NUM_TRACKS] = [&tone, &quiet, &quiet, &quiet];
    for _ in 0..8 {
        e.process_block(&inputs, &mut l, &mut r);
    }
    e.stop();

    e.seek(0);
    let (allocs, deallocs, _) = count(|| {
        e.record();
        let inputs: [&[f32]; NUM_TRACKS] = [&tone, &quiet, &quiet, &quiet];
        for _ in 0..64 {
            e.process_block(&inputs, &mut l, &mut r);
        }
        e.stop();
    });
    assert_eq!(
        (allocs, deallocs),
        (0, 0),
        "a track pass allocated {allocs} time(s) and freed {deallocs} time(s) on the \
         realtime path (record -> process_block -> stop)"
    );
}

#[test]
fn a_bounce_pass_does_not_allocate_on_the_realtime_path() {
    let _serial = serial();
    let dir = TempDir::new("bounce-pass");
    let mut e = Engine::create_with_character(&dir.0, 200_000, TapeCharacter::clean()).unwrap();

    // Give the bus something to fold forward, and warm everything up,
    // outside the measurement.
    let tone = sine(440.0, -6.0, BLOCK);
    let quiet = silence(BLOCK);
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    e.set_armed(0, true);
    e.seek(0);
    e.record();
    let inputs: [&[f32]; NUM_TRACKS] = [&tone, &quiet, &quiet, &quiet];
    for _ in 0..8 {
        e.process_block(&inputs, &mut l, &mut r);
    }
    e.stop();
    e.set_armed(0, false);

    e.seek(0);
    e.set_bus_armed(true);
    e.record();
    let quiet_in: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
    for _ in 0..8 {
        e.process_block(&quiet_in, &mut l, &mut r);
    }
    e.stop();

    // Measured: a second bounce, which is the case the double-buffered
    // reserve exists for - the first one's buffers are still pending a
    // flush, so this must come from the second pair without allocating.
    e.seek(0);
    e.set_bus_armed(true);
    let (allocs, deallocs, _) = count(|| {
        e.record();
        let quiet_in: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
        for _ in 0..64 {
            e.process_block(&quiet_in, &mut l, &mut r);
        }
        e.stop();
    });
    assert_eq!(
        (allocs, deallocs),
        (0, 0),
        "a bounce pass allocated {allocs} time(s) and freed {deallocs} time(s) on the \
         realtime path - the reserve exists precisely so this is zero"
    );
    assert_eq!(
        e.pass_buffer_fallbacks(),
        0,
        "the bounce fell back to allocating its own buffers"
    );
}

#[test]
fn transport_and_mixer_commands_do_not_allocate() {
    let _serial = serial();
    // The other things a UI sends while audio is running.
    use porta_engine::command::{apply, Command};
    let dir = TempDir::new("commands");
    let mut e = Engine::create_with_character(&dir.0, 200_000, TapeCharacter::clean()).unwrap();
    let quiet = silence(BLOCK);
    let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
    e.play();
    for _ in 0..8 {
        e.process_block(&inputs, &mut l, &mut r);
    }

    let (allocs, deallocs, _) = count(|| {
        for cmd in [
            Command::Fader { track: 0, db: -6.0 },
            Command::Pan {
                track: 1,
                value: 0.5,
            },
            Command::Mute { track: 2, on: true },
            Command::Master { db: -3.0 },
            Command::BounceFader { db: -6.0 },
            Command::BounceMute { on: true },
            Command::BounceArm { on: true },
            Command::Arm { track: 0, on: true },
            Command::Seek { sample: 1000 },
        ] {
            let _ = apply(&mut e, cmd);
        }
    });
    assert_eq!(
        (allocs, deallocs),
        (0, 0),
        "non-blocking commands allocated {allocs} / freed {deallocs}"
    );
}
