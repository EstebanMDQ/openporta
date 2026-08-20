//! Test instruments for headless audio verification: signal generators,
//! meters, spectral analysis, click detection. Dev-dependency only; never
//! ships in the app path.

pub mod asserts;
pub mod click;
pub mod meter;
pub mod pitch;
pub mod signal;
pub mod spectral;
pub mod wav;

/// Sample rate used by all generators, matching the engine.
pub const SAMPLE_RATE: u32 = 48_000;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::click::find_clicks;
    use crate::meter::{peak_dbfs, rms_dbfs, rms_envelope, FLOOR_DBFS};
    use crate::pitch::{cents, deviation_cents};
    use crate::signal::{dc, impulse, silence, sine, sweep, white_noise};
    use crate::spectral::{band_energy_db, dominant_freq, thd_db};

    #[test]
    fn full_scale_sine_rms_is_minus_3_dbfs() {
        let s = sine(1000.0, 0.0, SAMPLE_RATE as usize);
        assert!(
            (rms_dbfs(&s) - (-3.01)).abs() < 0.05,
            "got {}",
            rms_dbfs(&s)
        );
        assert!((peak_dbfs(&s) - 0.0).abs() < 0.01);
    }

    #[test]
    fn sine_level_scales() {
        let s = sine(440.0, -20.0, SAMPLE_RATE as usize);
        assert!((rms_dbfs(&s) - (-23.01)).abs() < 0.05);
    }

    #[test]
    fn silence_reports_floor() {
        let s = silence(4800);
        assert_eq!(rms_dbfs(&s), FLOOR_DBFS);
        assert_eq!(peak_dbfs(&s), FLOOR_DBFS);
    }

    #[test]
    fn dc_peak_matches_level() {
        let s = dc(0.5, 100);
        assert!((peak_dbfs(&s) - (-6.02)).abs() < 0.01);
        assert!((rms_dbfs(&s) - (-6.02)).abs() < 0.01);
    }

    #[test]
    fn impulse_has_unit_peak() {
        let s = impulse(64);
        assert!((peak_dbfs(&s) - 0.0).abs() < 1e-6);
        assert_eq!(s.iter().filter(|&&x| x != 0.0).count(), 1);
    }

    #[test]
    fn white_noise_is_seeded_and_leveled() {
        let a = white_noise(42, -10.0, 48_000);
        let b = white_noise(42, -10.0, 48_000);
        let c = white_noise(43, -10.0, 48_000);
        assert_eq!(a, b, "same seed must reproduce");
        assert_ne!(a, c, "different seed must differ");
        // Uniform noise RMS is peak/sqrt(3): -10 dBFS peak -> ~-14.77 dBFS RMS.
        assert!(
            (rms_dbfs(&a) - (-14.77)).abs() < 0.2,
            "got {}",
            rms_dbfs(&a)
        );
    }

    #[test]
    fn sweep_stays_within_level() {
        let s = sweep(20.0, 20_000.0, -6.0, 48_000);
        assert!(peak_dbfs(&s) <= -5.9);
        assert!(rms_dbfs(&s) > -12.0);
    }

    #[test]
    fn envelope_tracks_level_change() {
        let mut s = sine(1000.0, 0.0, 4800);
        s.extend(sine(1000.0, -20.0, 4800));
        let env = rms_envelope(&s, 4800);
        assert_eq!(env.len(), 2);
        assert!((env[0] - (-3.01)).abs() < 0.1);
        assert!((env[1] - (-23.01)).abs() < 0.1);
    }

    #[test]
    fn band_energy_concentrates_at_sine_frequency() {
        let s = sine(1000.0, -6.0, 8192);
        let at_tone = band_energy_db(&s, 900.0, 1100.0);
        let away = band_energy_db(&s, 5000.0, 6000.0);
        assert!(
            at_tone - away > 40.0,
            "tone band {at_tone:.1} dB vs away band {away:.1} dB"
        );
    }

    #[test]
    fn dominant_freq_finds_the_tone() {
        let s = sine(2000.0, -6.0, 48_000);
        assert!((dominant_freq(&s) - 2000.0).abs() < 5.0);
    }

    #[test]
    fn thd_low_for_pure_sine_high_for_clipped() {
        let clean = sine(1000.0, -12.0, 48_000);
        let clipped: Vec<f32> = sine(1000.0, 0.0, 48_000)
            .iter()
            .map(|&x| (x * 4.0).clamp(-1.0, 1.0))
            .collect();
        let thd_clean = thd_db(&clean, 1000.0, 5);
        let thd_clipped = thd_db(&clipped, 1000.0, 5);
        assert!(thd_clean < -60.0, "clean THD {thd_clean:.1} dB");
        assert!(thd_clipped > -20.0, "clipped THD {thd_clipped:.1} dB");
    }

    #[test]
    fn click_detector_passes_clean_signals() {
        assert_no_clicks!(&sine(1000.0, 0.0, 48_000));
        assert_no_clicks!(&sine(10_000.0, 0.0, 48_000));
        assert_no_clicks!(&white_noise(7, -6.0, 48_000));
        assert_no_clicks!(&silence(48_000));
    }

    #[test]
    fn click_detector_catches_injected_discontinuity() {
        let mut s = sine(440.0, -12.0, 48_000);
        s[24_000] += 0.5;
        let clicks = find_clicks(&s);
        assert!(!clicks.is_empty(), "discontinuity not detected");
        assert!(
            clicks.iter().any(|&i| i.abs_diff(24_000) <= 2),
            "clicks at {clicks:?}, expected near 24000"
        );
    }

    #[test]
    fn pitch_probe_tracks_a_stable_sine() {
        let s = sine(440.0, -6.0, 48_000);
        let (lo, hi) = deviation_cents(&s, 440.0);
        assert!(lo > -2.0 && hi < 2.0, "deviation {lo:.2}..{hi:.2} cents");
    }

    #[test]
    fn pitch_probe_sees_vibrato_depth() {
        // 440Hz carrier with +-30 cents of 4Hz vibrato, phase-accumulated.
        let depth = 2f32.powf(30.0 / 1200.0);
        let mut phase = 0f32;
        let s: Vec<f32> = (0..48_000)
            .map(|n| {
                let t = n as f32 / SAMPLE_RATE as f32;
                let f = 440.0 * depth.powf((2.0 * std::f32::consts::PI * 4.0 * t).sin());
                phase += 2.0 * std::f32::consts::PI * f / SAMPLE_RATE as f32;
                0.5 * phase.sin()
            })
            .collect();
        let (lo, hi) = deviation_cents(&s, 440.0);
        assert!(lo < -20.0, "min deviation {lo:.1} cents");
        assert!(hi > 20.0, "max deviation {hi:.1} cents");
    }

    #[test]
    fn cents_math() {
        assert!((cents(880.0, 440.0) - 1200.0).abs() < 1e-3);
    }

    #[test]
    fn wav_roundtrip_16bit() {
        let dir = std::env::temp_dir().join("porta-testkit-wav");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.wav");
        let s = sine(1000.0, -6.0, 4800);
        wav::write_wav_16(&path, &s);
        let back = wav::read_wav_mono(&path);
        assert_eq!(back.len(), s.len());
        let max_err = s
            .iter()
            .zip(&back)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max_err < 1.0 / 16384.0, "max err {max_err}");
    }

    #[test]
    fn attenuation_macro_works() {
        let input = sine(12_000.0, -6.0, 48_000);
        let output: Vec<f32> = input.iter().map(|&x| x * 0.01).collect();
        assert_attenuation_at_least_db!(&input, &output, 11_000.0, 13_000.0, 30.0);
        assert_rms_near_db!(&output, -49.0, 0.5);
    }
}
