//! Test instruments for headless audio verification: signal generators,
//! meters, spectral analysis, click detection. Dev-dependency only; never
//! ships in the app path.

/// Sample rate used by all generators, matching the engine.
pub const SAMPLE_RATE: u32 = 48_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rate_matches_engine() {
        assert_eq!(SAMPLE_RATE, 48_000);
    }
}
