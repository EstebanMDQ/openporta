//! Tape hiss: a seeded, gently shaped noise bed added at record time so
//! it lives on the tape and compounds across bounce generations (the
//! classic 4-track tell). Playback adds nothing.
//!
//! The noise is slightly high-shelved rather than flat white: cassette
//! hiss sits mostly in the upper mids and highs, so a first-difference
//! tilt gets closer than plain white noise at the same RMS.

use crate::AudioProcessor;

pub struct Hiss {
    level: f32,
    state: u32,
    seed: u32,
    prev: f32,
}

impl Hiss {
    /// `level_dbfs` is the approximate RMS of the noise bed.
    pub fn new(level_dbfs: f32, seed: u32) -> Self {
        let seed = seed | 1;
        // Uniform noise in [-1,1] has RMS 1/sqrt(3); the tilt filter
        // (1.5*w - 0.5*w[n-1]) multiplies variance by 2.5. Divide the two
        // out so the configured level is the RMS that actually lands.
        let shaping = (1.0 / 3f32.sqrt()) * 2.5f32.sqrt();
        Self {
            level: 10f32.powf(level_dbfs / 20.0) / shaping,
            state: seed,
            seed,
            prev: 0.0,
        }
    }

    fn white(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

impl AudioProcessor for Hiss {
    fn process(&mut self, block: &mut [f32]) {
        for s in block.iter_mut() {
            let w = self.white();
            // Mild high tilt: white plus its first difference.
            let shaped = w + (w - self.prev) * 0.5;
            self.prev = w;
            *s += shaped * self.level;
        }
    }

    fn reset(&mut self) {
        self.state = self.seed;
        self.prev = 0.0;
    }

    /// Unlike `reset` (back to the seed this instance was built with),
    /// this takes a fresh one - what `record()` needs each pass (REQ-304)
    /// without reallocating a new `Hiss`.
    fn reseed(&mut self, seed: u32) {
        self.seed = seed | 1;
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assert_block_size_invariant, process_in_blocks};
    use porta_testkit::meter::rms_dbfs;
    use porta_testkit::signal::{silence, sine};
    use porta_testkit::spectral::band_energy_db;

    #[test]
    fn noise_floor_lands_near_the_requested_level() {
        for level in [-70.0, -60.0, -50.0] {
            let mut h = Hiss::new(level, 12345);
            let out = process_in_blocks(&mut h, &silence(48_000), 512);
            let got = rms_dbfs(&out);
            assert!((got - level).abs() < 2.0, "asked {level}, got {got:.1}");
        }
    }

    #[test]
    fn same_seed_reproduces_exactly() {
        let a = process_in_blocks(&mut Hiss::new(-60.0, 99), &silence(24_000), 512);
        let b = process_in_blocks(&mut Hiss::new(-60.0, 99), &silence(24_000), 512);
        let c = process_in_blocks(&mut Hiss::new(-60.0, 100), &silence(24_000), 512);
        assert_eq!(a, b, "same seed must reproduce");
        assert_ne!(a, c, "different seed must differ");
    }

    #[test]
    fn hiss_is_tilted_toward_the_top() {
        let mut h = Hiss::new(-50.0, 7);
        let out = process_in_blocks(&mut h, &silence(16_384), 512);
        let low = band_energy_db(&out, 100.0, 1000.0);
        let high = band_energy_db(&out, 6000.0, 15_000.0);
        assert!(
            high > low,
            "high {high:.1} dB should exceed low {low:.1} dB"
        );
    }

    #[test]
    fn hiss_adds_to_program_material_without_swamping_it() {
        let signal = sine(1000.0, -12.0, 48_000);
        let mut h = Hiss::new(-60.0, 3);
        let out = process_in_blocks(&mut h, &signal, 512);
        assert!((rms_dbfs(&out) - rms_dbfs(&signal)).abs() < 0.2);
    }

    #[test]
    fn block_size_invariant() {
        let mut h = Hiss::new(-55.0, 42);
        assert_block_size_invariant(&mut h, &sine(997.0, -12.0, 8192));
    }
}
