//! Assertion macros for audio tests.

/// Assert a signal's RMS is within `tol` dB of `expected` dBFS.
#[macro_export]
macro_rules! assert_rms_near_db {
    ($signal:expr, $expected:expr, $tol:expr) => {{
        let rms = $crate::meter::rms_dbfs($signal);
        assert!(
            (rms - $expected).abs() <= $tol,
            "RMS {:.2} dBFS not within {} dB of {} dBFS",
            rms,
            $tol,
            $expected
        );
    }};
}

/// Assert the click detector finds no discontinuities.
#[macro_export]
macro_rules! assert_no_clicks {
    ($signal:expr) => {{
        let clicks = $crate::click::find_clicks($signal);
        assert!(
            clicks.is_empty(),
            "found {} click(s), first at sample {}",
            clicks.len(),
            clicks[0]
        );
    }};
}

/// Assert `output` is at least `min_db` quieter than `input` in the band
/// [lo_hz, hi_hz].
#[macro_export]
macro_rules! assert_attenuation_at_least_db {
    ($input:expr, $output:expr, $lo_hz:expr, $hi_hz:expr, $min_db:expr) => {{
        let before = $crate::spectral::band_energy_db($input, $lo_hz, $hi_hz);
        let after = $crate::spectral::band_energy_db($output, $lo_hz, $hi_hz);
        assert!(
            before - after >= $min_db,
            "band {}-{} Hz attenuated {:.2} dB, expected >= {} dB",
            $lo_hz,
            $hi_hz,
            before - after,
            $min_db
        );
    }};
}
