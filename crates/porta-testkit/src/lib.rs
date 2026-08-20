//! Test instruments for headless audio verification: signal generators,
//! meters, spectral analysis, click detection. Dev-dependency only; never
//! ships in the app path.

pub mod meter;
pub mod signal;

/// Sample rate used by all generators, matching the engine.
pub const SAMPLE_RATE: u32 = 48_000;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::{peak_dbfs, rms_dbfs, rms_envelope, FLOOR_DBFS};
    use crate::signal::{dc, impulse, silence, sine, sweep, white_noise};

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
}
