//! Optional bitcrush and sample-rate reduction. Off by default: this is
//! flavour on top of the tape character, not part of it. A cassette does
//! not sound like a sampler, but a 4-track feeding a cheap sampler does,
//! and that is a sound people want.

use crate::AudioProcessor;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct CrushParams {
    /// Effective word length, 1-16 bits.
    pub bits: u8,
    /// Effective sample rate in Hz; samples are held between updates.
    pub rate_hz: f32,
}

impl Default for CrushParams {
    fn default() -> Self {
        Self {
            bits: 12,
            rate_hz: 24_000.0,
        }
    }
}

pub struct Crush {
    step: f32,
    inv_step: f32,
    phase_inc: f32,
    phase: f32,
    held: f32,
}

impl Crush {
    pub fn new(params: CrushParams) -> Self {
        let bits = params.bits.clamp(1, 16);
        let levels = (1u32 << (bits - 1)) as f32;
        let rate = params.rate_hz.clamp(1000.0, crate::SAMPLE_RATE as f32);
        Self {
            step: 1.0 / levels,
            inv_step: levels,
            phase_inc: rate / crate::SAMPLE_RATE as f32,
            phase: 1.0,
            held: 0.0,
        }
    }
}

impl AudioProcessor for Crush {
    fn process(&mut self, block: &mut [f32]) {
        for s in block.iter_mut() {
            self.phase += self.phase_inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                self.held = (*s * self.inv_step).round() * self.step;
            }
            *s = self.held;
        }
    }

    fn reset(&mut self) {
        self.phase = 1.0;
        self.held = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assert_block_size_invariant, process_in_blocks};
    use porta_testkit::signal::sine;
    use porta_testkit::spectral::band_energy_db;

    #[test]
    fn quantizes_to_the_configured_step() {
        let mut c = Crush::new(CrushParams {
            bits: 4,
            rate_hz: 48_000.0,
        });
        let out = process_in_blocks(&mut c, &sine(1000.0, 0.0, 4800), 512);
        let step = 1.0 / 8.0;
        for &s in &out {
            let ratio = s / step;
            assert!(
                (ratio - ratio.round()).abs() < 1e-4,
                "{s} is not on the {step} grid"
            );
        }
    }

    #[test]
    fn fewer_bits_means_more_quantization_noise() {
        let input = sine(1000.0, -6.0, 48_000);
        let noise_at = |bits: u8| {
            let mut c = Crush::new(CrushParams {
                bits,
                rate_hz: 48_000.0,
            });
            let out = process_in_blocks(&mut c, &input, 512);
            // Energy away from the fundamental is the error.
            band_energy_db(&out, 3000.0, 20_000.0)
        };
        assert!(noise_at(4) > noise_at(8) + 10.0);
        assert!(noise_at(8) > noise_at(14) + 10.0);
    }

    #[test]
    fn rate_reduction_creates_alias_energy() {
        let input = sine(6000.0, -6.0, 48_000);
        let mut c = Crush::new(CrushParams {
            bits: 16,
            rate_hz: 8000.0,
        });
        let out = process_in_blocks(&mut c, &input, 512);
        // A 6kHz tone sampled at 8kHz folds down to 2kHz.
        let alias = band_energy_db(&out, 1800.0, 2200.0);
        let quiet = band_energy_db(&input, 1800.0, 2200.0);
        assert!(alias > quiet + 30.0, "alias {alias:.1} vs {quiet:.1}");
    }

    #[test]
    fn holds_samples_between_updates() {
        let mut c = Crush::new(CrushParams {
            bits: 16,
            rate_hz: 12_000.0,
        });
        let out = process_in_blocks(&mut c, &sine(500.0, -6.0, 480), 480);
        // At a quarter rate, samples come in runs of four.
        let changes = out.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(
            changes < out.len() / 3,
            "expected held runs, {changes} changes in {} samples",
            out.len()
        );
    }

    #[test]
    fn block_size_invariant() {
        let mut c = Crush::new(CrushParams::default());
        assert_block_size_invariant(&mut c, &sine(997.0, -6.0, 8192));
    }
}
