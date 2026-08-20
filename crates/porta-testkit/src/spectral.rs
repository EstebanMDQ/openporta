//! FFT-based measurements. All analyses use a Hann window; energies are
//! relative dB, meant for comparisons (band A vs band B, generation N vs
//! N+1), not absolute calibration.

use crate::SAMPLE_RATE;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::f32::consts::PI;

/// Hann-windowed magnitude spectrum, bins 0..len/2.
pub fn magnitude_spectrum(signal: &[f32]) -> Vec<f32> {
    let len = signal.len();
    assert!(len >= 16, "signal too short for spectral analysis");
    let mut buf: Vec<Complex<f32>> = signal
        .iter()
        .enumerate()
        .map(|(n, &s)| {
            let w = 0.5 - 0.5 * ((2.0 * PI * n as f32) / len as f32).cos();
            Complex::new(s * w, 0.0)
        })
        .collect();
    FftPlanner::new().plan_fft_forward(len).process(&mut buf);
    buf[..len / 2].iter().map(|c| c.norm()).collect()
}

pub fn bin_hz(fft_len: usize) -> f32 {
    SAMPLE_RATE as f32 / fft_len as f32
}

/// Total energy in [lo_hz, hi_hz] as relative dB.
pub fn band_energy_db(signal: &[f32], lo_hz: f32, hi_hz: f32) -> f32 {
    let mags = magnitude_spectrum(signal);
    let hz = bin_hz(signal.len());
    let lo = (lo_hz / hz).floor().max(0.0) as usize;
    let hi = ((hi_hz / hz).ceil() as usize).min(mags.len().saturating_sub(1));
    let energy: f32 = mags[lo..=hi].iter().map(|m| m * m).sum();
    10.0 * energy.max(1e-20).log10()
}

/// Frequency of the strongest bin.
pub fn dominant_freq(signal: &[f32]) -> f32 {
    let mags = magnitude_spectrum(signal);
    let peak_bin = mags
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0);
    peak_bin as f32 * bin_hz(signal.len())
}

/// Energy of harmonics 2..=n_harmonics relative to the fundamental, in dB.
/// More negative = cleaner. Each partial is measured in a +-2 bin window.
pub fn thd_db(signal: &[f32], fundamental_hz: f32, n_harmonics: usize) -> f32 {
    let mags = magnitude_spectrum(signal);
    let hz = bin_hz(signal.len());
    let window_energy = |freq: f32| -> f32 {
        let bin = (freq / hz).round() as isize;
        (bin - 2..=bin + 2)
            .filter_map(|b| mags.get(usize::try_from(b).ok()?))
            .map(|m| m * m)
            .sum()
    };
    let fund = window_energy(fundamental_hz).max(1e-20);
    let harm: f32 = (2..=n_harmonics)
        .map(|k| window_energy(fundamental_hz * k as f32))
        .sum();
    10.0 * (harm.max(1e-20) / fund).log10()
}
