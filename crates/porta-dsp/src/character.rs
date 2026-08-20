//! `TapeCharacter`: the formulation of a cassette. Fixed at creation and
//! stored in the project manifest (REQ-103), so a cassette sounds like
//! itself for its whole life.
//!
//! Stage order matters and follows the signal path of a real machine:
//! the record amp saturates first, the head and tape limit bandwidth,
//! the transport wobbles the whole thing, and hiss is on the tape
//! underneath it all. Crush, when enabled, sits last as an outboard
//! flavour rather than part of the tape path.

use crate::crush::{Crush, CrushParams};
use crate::filter::Bandwidth;
use crate::flutter::Flutter;
use crate::noise::Hiss;
use crate::saturation::Saturation;
use crate::Chain;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct TapeCharacter {
    pub drive_db: f32,
    pub lpf_cutoff_hz: f32,
    pub hpf_cutoff_hz: f32,
    pub flutter_rate_hz: f32,
    pub flutter_depth_cents: f32,
    pub hiss_dbfs: f32,
    pub crush: Option<CrushParams>,
    /// Cassette-wide seed; each pass derives its own from this.
    pub noise_seed: u64,
}

impl Default for TapeCharacter {
    fn default() -> Self {
        Self {
            drive_db: 9.0,
            lpf_cutoff_hz: 11_000.0,
            hpf_cutoff_hz: 60.0,
            flutter_rate_hz: 2.5,
            flutter_depth_cents: 12.0,
            hiss_dbfs: -66.0,
            crush: None,
            noise_seed: 0,
        }
    }
}

impl TapeCharacter {
    pub fn new(noise_seed: u64) -> Self {
        Self {
            noise_seed,
            ..Default::default()
        }
    }

    /// A near-transparent character, for tests that want the transport
    /// and tape mechanics without the colour. Drive is well below unity
    /// because tanh at unity drive still bends a -12 dBFS signal by about
    /// 2 percent: "no extra drive" is not the same as "no saturation".
    pub fn clean() -> Self {
        Self {
            drive_db: -30.0,
            lpf_cutoff_hz: 20_000.0,
            hpf_cutoff_hz: 5.0,
            flutter_rate_hz: 2.5,
            flutter_depth_cents: 0.0,
            hiss_dbfs: -140.0,
            crush: None,
            noise_seed: 0,
        }
    }

    /// Build the record-path chain for one pass. `pass_seed` decorrelates
    /// flutter and hiss between passes so bounces compound (REQ-304).
    pub fn build_chain(&self, pass_seed: u32) -> Chain {
        let mut stages: Vec<Box<dyn crate::AudioProcessor>> = vec![
            Box::new(Saturation::new(self.drive_db)),
            Box::new(Bandwidth::new(self.lpf_cutoff_hz, self.hpf_cutoff_hz)),
            Box::new(Flutter::new(
                self.flutter_rate_hz,
                self.flutter_depth_cents,
                pass_seed,
            )),
            Box::new(Hiss::new(self.hiss_dbfs, pass_seed ^ 0x5f5f_5f5f)),
        ];
        if let Some(params) = self.crush {
            stages.push(Box::new(Crush::new(params)));
        }
        Chain::new(stages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::process_in_blocks;
    use porta_testkit::meter::rms_dbfs;
    use porta_testkit::signal::{silence, sine};
    use porta_testkit::spectral::{band_energy_db, thd_db};

    /// The flutter delay dominates the chain's latency; skip past it.
    const SETTLE: usize = 4096;

    #[test]
    fn default_character_colours_the_signal() {
        let input = sine(1000.0, -6.0, 96_000);
        let mut chain = TapeCharacter::new(1).build_chain(7);
        let out = process_in_blocks(&mut chain, &input, 512);

        // Saturation: measurable harmonics that the input did not have.
        let before = thd_db(&input[SETTLE..], 1000.0, 7);
        let after = thd_db(&out[SETTLE..], 1000.0, 7);
        assert!(after > before + 20.0, "THD {before:.1} -> {after:.1} dB");
    }

    #[test]
    fn default_character_kills_the_top_end() {
        // Measured with a loud 16kHz tone rather than by looking at the
        // HF band of a 1kHz tone: hiss is added after the filter and is
        // deliberately high-tilted, so it dominates any empty HF band.
        let input = sine(16_000.0, -6.0, 96_000);
        let mut chain = TapeCharacter::new(1).build_chain(7);
        let out = process_in_blocks(&mut chain, &input, 512);
        let before = band_energy_db(&input[SETTLE..], 15_000.0, 17_000.0);
        let after = band_energy_db(&out[SETTLE..], 15_000.0, 17_000.0);
        assert!(
            before - after > 15.0,
            "16kHz only dropped {:.1} dB",
            before - after
        );
    }

    #[test]
    fn hiss_reaches_the_output() {
        let mut chain = TapeCharacter::new(1).build_chain(7);
        let out = process_in_blocks(&mut chain, &silence(96_000), 512);
        let floor = rms_dbfs(&out[SETTLE..]);
        assert!(
            (-80.0..-50.0).contains(&floor),
            "noise floor {floor:.1} dBFS"
        );
    }

    #[test]
    fn clean_character_adds_no_colour() {
        // Compared by level, harmonics, and noise floor rather than
        // sample-by-sample: even a 20kHz low-pass shifts phase at 1kHz,
        // so a bit-exact comparison would fail on a chain that is
        // musically transparent.
        let input = sine(1000.0, -12.0, 96_000);
        let mut chain = TapeCharacter::clean().build_chain(7);
        let out = process_in_blocks(&mut chain, &input, 512);

        let level_change = rms_dbfs(&out[SETTLE..]) - rms_dbfs(&input[SETTLE..]);
        assert!(level_change.abs() < 0.5, "level moved {level_change:.2} dB");
        assert!(
            thd_db(&out[SETTLE..], 1000.0, 7) < -50.0,
            "clean chain distorted: {:.1} dB",
            thd_db(&out[SETTLE..], 1000.0, 7)
        );

        let mut quiet_chain = TapeCharacter::clean().build_chain(7);
        let quiet = process_in_blocks(&mut quiet_chain, &silence(48_000), 512);
        assert!(
            rms_dbfs(&quiet[SETTLE..]) < -100.0,
            "clean chain is noisy: {:.1} dBFS",
            rms_dbfs(&quiet[SETTLE..])
        );
    }

    #[test]
    fn same_pass_seed_reproduces_exactly() {
        let input = sine(440.0, -6.0, 48_000);
        let c = TapeCharacter::new(42);
        let a = process_in_blocks(&mut c.build_chain(3), &input, 512);
        let b = process_in_blocks(&mut c.build_chain(3), &input, 512);
        let other = process_in_blocks(&mut c.build_chain(4), &input, 512);
        assert_eq!(a, b, "same pass seed must reproduce");
        assert_ne!(a, other, "different pass seed must differ");
    }

    #[test]
    fn crush_is_opt_in() {
        let input = sine(1000.0, -6.0, 48_000);
        let mut plain = TapeCharacter::new(1);
        plain.crush = None;
        let without = process_in_blocks(&mut plain.build_chain(1), &input, 512);

        let mut crushed = plain;
        crushed.crush = Some(CrushParams {
            bits: 4,
            rate_hz: 12_000.0,
        });
        let with = process_in_blocks(&mut crushed.build_chain(1), &input, 512);
        assert_ne!(without, with, "crush must change the sound when enabled");
        assert!(
            band_energy_db(&with[SETTLE..], 3000.0, 20_000.0)
                > band_energy_db(&without[SETTLE..], 3000.0, 20_000.0) + 6.0,
            "crush should add grit"
        );
    }

    #[test]
    fn chain_reports_flutter_latency() {
        let chain = TapeCharacter::new(1).build_chain(1);
        assert!(chain.latency_samples() > 0);
    }
}
