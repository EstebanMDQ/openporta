//! Pitch measurement via interpolated zero crossings. Good enough to
//! verify flutter depth and rate numerically on sine material.

use crate::SAMPLE_RATE;

/// Instantaneous frequency estimates: (sample_position, hz) at each
/// positive-going zero crossing after the first.
pub fn pitch_track(signal: &[f32]) -> Vec<(f32, f32)> {
    let mut crossings = Vec::new();
    for n in 1..signal.len() {
        let (a, b) = (signal[n - 1], signal[n]);
        if a < 0.0 && b >= 0.0 {
            // Linear interpolation of the exact crossing position.
            let frac = -a / (b - a);
            crossings.push((n - 1) as f32 + frac);
        }
    }
    crossings
        .windows(2)
        .map(|w| {
            let period = w[1] - w[0];
            (w[1], SAMPLE_RATE as f32 / period)
        })
        .collect()
}

pub fn cents(freq_hz: f32, reference_hz: f32) -> f32 {
    1200.0 * (freq_hz / reference_hz).log2()
}

/// (min, max) pitch deviation from `nominal_hz` in cents across the track.
pub fn deviation_cents(signal: &[f32], nominal_hz: f32) -> (f32, f32) {
    let track = pitch_track(signal);
    assert!(!track.is_empty(), "no zero crossings found");
    track
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &(_, f)| {
            let c = cents(f, nominal_hz);
            (lo.min(c), hi.max(c))
        })
}
