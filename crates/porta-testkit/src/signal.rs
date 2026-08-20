//! Deterministic test-signal generators. Levels are peak amplitude in dBFS
//! (0 dBFS = peak of 1.0).

use crate::SAMPLE_RATE;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use std::f32::consts::TAU;

pub fn db_to_amp(dbfs: f32) -> f32 {
    10f32.powf(dbfs / 20.0)
}

pub fn sine(freq_hz: f32, dbfs: f32, len: usize) -> Vec<f32> {
    let amp = db_to_amp(dbfs);
    let step = TAU * freq_hz / SAMPLE_RATE as f32;
    (0..len).map(|n| amp * (step * n as f32).sin()).collect()
}

/// Exponential sweep from `f0` to `f1` over the whole buffer.
pub fn sweep(f0: f32, f1: f32, dbfs: f32, len: usize) -> Vec<f32> {
    let amp = db_to_amp(dbfs);
    let ratio = f1 / f0;
    let mut phase = 0f32;
    let mut out = Vec::with_capacity(len);
    for n in 0..len {
        out.push(amp * phase.sin());
        let t = n as f32 / len as f32;
        let freq = f0 * ratio.powf(t);
        phase += TAU * freq / SAMPLE_RATE as f32;
    }
    out
}

/// Uniform white noise in [-amp, amp), seeded for reproducibility.
pub fn white_noise(seed: u64, dbfs: f32, len: usize) -> Vec<f32> {
    let amp = db_to_amp(dbfs);
    let mut rng = SmallRng::seed_from_u64(seed);
    (0..len).map(|_| rng.random_range(-amp..amp)).collect()
}

/// A single unit impulse at sample 0.
pub fn impulse(len: usize) -> Vec<f32> {
    let mut out = vec![0.0; len];
    if let Some(first) = out.first_mut() {
        *first = 1.0;
    }
    out
}

pub fn silence(len: usize) -> Vec<f32> {
    vec![0.0; len]
}

pub fn dc(level: f32, len: usize) -> Vec<f32> {
    vec![level; len]
}
