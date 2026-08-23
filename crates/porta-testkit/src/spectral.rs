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

/// Pearson correlation between two equal-length signals, in [-1, 1].
///
/// For asking whether two channels carry *related* content rather than
/// merely similar levels: independently seeded hiss correlates near 0
/// however loud it is, while one signal duplicated into both channels
/// correlates at 1. Returns 0 for empty or constant input (no variance
/// to correlate), which reads as "unrelated" - the safe answer for a
/// decorrelation assertion.
pub fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mean_a = a[..n].iter().sum::<f32>() / n as f32;
    let mean_b = b[..n].iter().sum::<f32>() / n as f32;
    let (mut cov, mut var_a, mut var_b) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let da = (a[i] - mean_a) as f64;
        let db = (b[i] - mean_b) as f64;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }
    if var_a <= 0.0 || var_b <= 0.0 {
        return 0.0;
    }
    (cov / (var_a.sqrt() * var_b.sqrt())) as f32
}

#[cfg(test)]
mod pearson_tests {
    use super::pearson;
    use crate::signal::{sine, white_noise};

    #[test]
    fn identical_signals_correlate_at_one() {
        let s = sine(440.0, -6.0, 4800);
        assert!((pearson(&s, &s) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn inverted_signals_correlate_at_minus_one() {
        let s = sine(440.0, -6.0, 4800);
        let inv: Vec<f32> = s.iter().map(|x| -x).collect();
        assert!((pearson(&s, &inv) + 1.0).abs() < 1e-4);
    }

    #[test]
    fn independently_seeded_noise_is_uncorrelated() {
        let a = white_noise(1, -20.0, 48_000);
        let b = white_noise(2, -20.0, 48_000);
        assert!(pearson(&a, &b).abs() < 0.05, "got {}", pearson(&a, &b));
    }

    #[test]
    fn constant_or_empty_input_reads_as_unrelated() {
        assert_eq!(pearson(&[], &[]), 0.0);
        assert_eq!(pearson(&[1.0; 100], &[1.0; 100]), 0.0);
    }
}
