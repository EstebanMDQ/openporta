//! `TapeCharacter`: the formulation of a cassette. Fixed at creation and
//! stored in the project manifest (REQ-103), so a cassette sounds like
//! itself for its whole life.
//!
//! Stage order matters and follows the signal path of a real machine:
//! the record amp saturates first, hiss joins the signal at the tape
//! itself, the head and tape response then limit the bandwidth of both,
//! and the transport wobbles what comes back. Crush, when enabled, sits
//! last as an outboard flavour rather than part of the tape path.
//!
//! Hiss goes in before the bandwidth stage rather than after because
//! that is what makes generations pile up: hiss printed inside the
//! passband survives the next pass's filter and adds to that pass's own
//! hiss. Adding it after the filter puts most of its energy above the
//! corner, where the next generation simply removes it again, and the
//! noise floor then barely moves across bounces.

use crate::crush::{Crush, CrushParams};
use crate::filter::Bandwidth;
use crate::flutter::{Flutter, StereoFlutter};
use crate::noise::Hiss;
use crate::saturation::Saturation;
use crate::{AudioProcessor, Chain};

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

    /// Stage positions `build_chain` constructs at, in order - kept next
    /// to it so `reseed_chain` (which has to reseed the same two seeded
    /// stages without knowing their types) can't drift out of sync with
    /// what's actually built here.
    const HISS_STAGE: usize = 1;
    const FLUTTER_STAGE: usize = 3;

    /// Build the record-path chain for one pass. `pass_seed` decorrelates
    /// flutter and hiss between passes so bounces compound (REQ-304).
    /// Only for building a chain fresh, off the realtime thread (cassette
    /// open/create) - reseeding an existing one for a new pass is
    /// `reseed_chain`, which does the same thing without allocating.
    pub fn build_chain(&self, pass_seed: u32) -> Chain {
        let mut stages: Vec<Box<dyn crate::AudioProcessor>> = vec![
            Box::new(Saturation::new(self.drive_db)),
            Box::new(Hiss::new(self.hiss_dbfs, pass_seed ^ 0x5f5f_5f5f)),
            Box::new(Bandwidth::new(self.lpf_cutoff_hz, self.hpf_cutoff_hz)),
            Box::new(Flutter::new(
                self.flutter_rate_hz,
                self.flutter_depth_cents,
                pass_seed,
            )),
        ];
        if let Some(params) = self.crush {
            stages.push(Box::new(Crush::new(params)));
        }
        Chain::new(stages)
    }

    /// Realtime-safe equivalent of `build_chain`: resets every stage of
    /// an already-built chain in place and reseeds the two that carry
    /// their own randomness (Hiss, Flutter), the same derivation
    /// `build_chain` uses - no allocation, so `Engine::record()` (which
    /// runs on the audio callback) can call this instead of rebuilding a
    /// `Chain` from scratch every time recording engages. `chain` MUST
    /// have been built by this same `TapeCharacter`'s `build_chain` (same
    /// stage count/order - `Crush` present or not changes nothing here
    /// since only indices 1 and 3 are touched); `TapeCharacter` is fixed
    /// for a cassette's whole life (REQ-103), so that invariant holds for
    /// as long as one `Engine` is open.
    pub fn reseed_chain(&self, chain: &mut Chain, pass_seed: u32) {
        chain.reset();
        chain.reseed_stage(Self::HISS_STAGE, pass_seed ^ 0x5f5f_5f5f);
        chain.reseed_stage(Self::FLUTTER_STAGE, pass_seed);
    }

    /// Hiss's position in the *split* pre-flutter chain (change 001,
    /// M7.2) - kept beside `build_split_chain` for the same
    /// can't-drift reason `HISS_STAGE`/`FLUTTER_STAGE` sit beside
    /// `build_chain`. (It happens to equal `HISS_STAGE` today because
    /// flutter sits after bandwidth in the unsplit order; a separate
    /// constant so that stays a coincidence, not a dependency.)
    const SPLIT_HISS_STAGE: usize = 1;

    /// One channel's halves of a split bounce chain (change 001,
    /// REQ-402): everything before flutter, and everything after it.
    /// The stereo pass runs, per channel: pre -> (shared
    /// `StereoFlutter`) -> post, with `build_stereo_flutter` supplying
    /// the shared middle. Stage order matches `build_chain` exactly
    /// (Saturation, Hiss, Bandwidth | Flutter | Crush) - the module
    /// doc's signal-path reasoning is unchanged, the chain is just cut
    /// where flutter sits. `pass_seed` is this channel's own (REQ-702's
    /// per-channel derivation happens at the engine; hiss's `^`
    /// constant here matches `build_chain`'s). The post half is EMPTY
    /// when crush is off - `Chain`'s own reset/process iterate whatever
    /// stages exist, so the empty case is safe by construction.
    /// Off-thread only, like `build_chain`: build once at cassette
    /// open/create, reseed per pass via `reseed_split_chain`.
    pub fn build_split_chain(&self, pass_seed: u32) -> (Chain, Chain) {
        let pre: Vec<Box<dyn crate::AudioProcessor>> = vec![
            Box::new(Saturation::new(self.drive_db)),
            Box::new(Hiss::new(self.hiss_dbfs, pass_seed ^ 0x5f5f_5f5f)),
            Box::new(Bandwidth::new(self.lpf_cutoff_hz, self.hpf_cutoff_hz)),
        ];
        let mut post: Vec<Box<dyn crate::AudioProcessor>> = Vec::new();
        if let Some(params) = self.crush {
            post.push(Box::new(Crush::new(params)));
        }
        (Chain::new(pre), Chain::new(post))
    }

    /// The shared stereo flutter for a bounce pass. Seeded at channel
    /// term 0 by convention (the caller passes that channel's seed):
    /// there is exactly one modulator, so which channel's seed it uses
    /// must be a fixed choice, not an implementation coin-flip REQ-702's
    /// bit-reproducibility would silently depend on.
    pub fn build_stereo_flutter(&self, pass_seed: u32) -> StereoFlutter {
        StereoFlutter::new(self.flutter_rate_hz, self.flutter_depth_cents, pass_seed)
    }

    /// Realtime-safe per-pass reset for one channel's split halves -
    /// `reseed_chain`'s exact shape, minus flutter (that's the shared
    /// `StereoFlutter::reseed`, called once, not per channel): reset
    /// BOTH sub-chains (Bandwidth's biquads live in the pre half and
    /// must not carry over between passes; the post half may be empty,
    /// which resets as a no-op), then reseed Hiss, the only seeded
    /// stage in either half.
    pub fn reseed_split_chain(&self, pre: &mut Chain, post: &mut Chain, pass_seed: u32) {
        pre.reset();
        post.reset();
        pre.reseed_stage(Self::SPLIT_HISS_STAGE, pass_seed ^ 0x5f5f_5f5f);
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
    fn reseed_chain_matches_a_freshly_built_one() {
        // The property record() now depends on instead of allocating a
        // fresh Chain every pass (a real, pre-existing REQ-902 violation
        // found independent of the bounce proposal, fixed here): reusing
        // one Chain across passes via reseed_chain must be indistinguishable
        // from build_chain(seed) on a brand new one, for any seed and
        // regardless of what the chain's state looked like before.
        let input = sine(440.0, -6.0, 48_000);
        let c = TapeCharacter::new(42);

        let mut reused = c.build_chain(1);
        let _ = process_in_blocks(&mut reused, &sine(220.0, -3.0, 48_000), 512);
        c.reseed_chain(&mut reused, 3);
        let from_reuse = process_in_blocks(&mut reused, &input, 512);

        let fresh = process_in_blocks(&mut c.build_chain(3), &input, 512);

        assert_eq!(
            from_reuse, fresh,
            "reseeding a used chain must match a fresh build_chain with the same seed"
        );
    }

    #[test]
    fn reseeding_a_split_setup_matches_a_freshly_built_one() {
        // The stereo analogue of reseed_chain_matches_a_freshly_built_one
        // (change 001, M7.2): a bounce pass setup reused across passes -
        // reset both sub-chains per channel, reseed hiss, reseed the
        // shared StereoFlutter - must be indistinguishable from building
        // everything fresh with the same seeds, regardless of what ran
        // through it before. Anything left over (Bandwidth biquads, the
        // flutter delay rings - both real bugs a review caught being
        // dropped from an earlier version of the reseed sequence) breaks
        // this first.
        let c = TapeCharacter::new(42);
        let (seed_l, seed_r) = (3u32, 4u32); // channel-term derivation is the engine's job
        let input_l = sine(440.0, -6.0, 24_000);
        let input_r = sine(330.0, -6.0, 24_000);

        let render = |pre_l: &mut Chain,
                      post_l: &mut Chain,
                      pre_r: &mut Chain,
                      post_r: &mut Chain,
                      sf: &mut crate::flutter::StereoFlutter| {
            let mut l = input_l.clone();
            let mut r = input_r.clone();
            for (cl, cr) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
                pre_l.process(cl);
                pre_r.process(cr);
                sf.process(cl, cr);
                post_l.process(cl);
                post_r.process(cr);
            }
            (l, r)
        };

        // Reused path: build once, dirty it with a different pass, then
        // reseed back to the target seeds.
        let (mut pre_l, mut post_l) = c.build_split_chain(1);
        let (mut pre_r, mut post_r) = c.build_split_chain(2);
        let mut sf = c.build_stereo_flutter(1);
        let _ = render(&mut pre_l, &mut post_l, &mut pre_r, &mut post_r, &mut sf);
        c.reseed_split_chain(&mut pre_l, &mut post_l, seed_l);
        c.reseed_split_chain(&mut pre_r, &mut post_r, seed_r);
        sf.reseed(seed_l); // channel term 0 = left, by convention
        let reused = render(&mut pre_l, &mut post_l, &mut pre_r, &mut post_r, &mut sf);

        // Fresh path: everything built directly at the target seeds.
        let (mut fpre_l, mut fpost_l) = c.build_split_chain(seed_l);
        let (mut fpre_r, mut fpost_r) = c.build_split_chain(seed_r);
        let mut fsf = c.build_stereo_flutter(seed_l);
        let fresh = render(
            &mut fpre_l,
            &mut fpost_l,
            &mut fpre_r,
            &mut fpost_r,
            &mut fsf,
        );

        assert_eq!(reused, fresh, "reused+reseeded must match freshly built");
    }

    #[test]
    fn empty_post_flutter_chain_is_safe_and_crush_lands_there() {
        let plain = TapeCharacter::new(1); // crush: None
        let (_, mut post) = plain.build_split_chain(7);
        let mut block = sine(440.0, -6.0, 512);
        let before = block.clone();
        post.reset(); // must not panic on empty
        post.process(&mut block); // identity on empty
        assert_eq!(block, before, "empty post half must pass audio untouched");

        let mut crushed = plain;
        crushed.crush = Some(CrushParams {
            bits: 4,
            rate_hz: 12_000.0,
        });
        let (_, mut post) = crushed.build_split_chain(7);
        post.process(&mut block);
        assert_ne!(block, before, "crush must land in the post half");
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
