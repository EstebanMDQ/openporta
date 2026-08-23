//! Lo-fi tape character DSP: the AudioProcessor trait and the processors
//! that get baked onto tape at record time.

pub mod character;
pub mod crush;
pub mod filter;
pub mod flutter;
pub mod noise;
pub mod saturation;
pub mod testing;

/// Internal engine sample rate. Fixed for v1; no runtime plumbing.
pub const SAMPLE_RATE: u32 = 48_000;

/// Largest block any processor must accept in one `process` call.
pub const MAX_BLOCK: usize = 4096;

/// A mono, in-place audio processor. Implementations must not allocate,
/// lock, or touch the filesystem inside `process`.
pub trait AudioProcessor: Send {
    fn process(&mut self, block: &mut [f32]);
    fn reset(&mut self);
    /// Reset state the way `reset()` does, but also re-seed whatever
    /// randomness this stage carries for a fresh pass (REQ-304). Default
    /// no-op-beyond-`reset` for stages with no seeded state (Saturation,
    /// Bandwidth, Crush) - only `Hiss` and `Flutter` override this.
    /// Exists so a `Chain` built once, off the realtime thread, can be
    /// reused pass after pass in place rather than rebuilt (which would
    /// re-`Box` every stage on the audio callback, see
    /// `character::TapeCharacter::reseed_chain`).
    fn reseed(&mut self, _seed: u32) {
        self.reset();
    }
    fn latency_samples(&self) -> usize {
        0
    }
}

/// An ordered chain of processors, built at configuration time.
pub struct Chain {
    stages: Vec<Box<dyn AudioProcessor>>,
}

impl Chain {
    pub fn new(stages: Vec<Box<dyn AudioProcessor>>) -> Self {
        Self { stages }
    }

    /// A chain that leaves the signal untouched.
    pub fn passthrough() -> Self {
        Self { stages: Vec::new() }
    }

    pub fn latency_samples(&self) -> usize {
        self.stages.iter().map(|s| s.latency_samples()).sum()
    }

    /// Reseed one stage by its position in the `Vec` `build_chain`
    /// constructed - an internal invariant between this and whichever
    /// `build_chain`-equivalent knows the stage order, not something a
    /// caller should be deriving independently. Indexes directly
    /// (panics out of range) rather than a silent `get_mut` no-op: if
    /// the two ever drift, a loud panic in testing is far better than a
    /// stage quietly never getting reseeded again.
    pub fn reseed_stage(&mut self, index: usize, seed: u32) {
        self.stages[index].reseed(seed);
    }
}

impl AudioProcessor for Chain {
    fn process(&mut self, block: &mut [f32]) {
        for stage in &mut self.stages {
            stage.process(block);
        }
    }

    fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }

    fn latency_samples(&self) -> usize {
        Chain::latency_samples(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assert_block_size_invariant, process_in_blocks};
    use porta_testkit::signal::sine;

    /// A trivial stateful processor: one-sample delay. Used to prove the
    /// block-size invariance harness actually catches per-block state.
    struct OneSampleDelay {
        held: f32,
    }

    impl AudioProcessor for OneSampleDelay {
        fn process(&mut self, block: &mut [f32]) {
            for s in block.iter_mut() {
                std::mem::swap(s, &mut self.held);
            }
        }
        fn reset(&mut self) {
            self.held = 0.0;
        }
        fn latency_samples(&self) -> usize {
            1
        }
    }

    #[test]
    fn passthrough_chain_is_identity() {
        let mut chain = Chain::passthrough();
        let mut block = vec![0.5f32, -0.25, 1.0, -1.0];
        let original = block.clone();
        chain.process(&mut block);
        assert_eq!(block, original);
        assert_eq!(chain.latency_samples(), 0);
    }

    #[test]
    fn chain_runs_stages_in_order() {
        let mut chain = Chain::new(vec![
            Box::new(OneSampleDelay { held: 0.0 }),
            Box::new(OneSampleDelay { held: 0.0 }),
        ]);
        assert_eq!(chain.latency_samples(), 2);
        let signal = vec![1.0f32, 2.0, 3.0, 4.0, 5.0];
        let out = process_in_blocks(&mut chain, &signal, 2);
        assert_eq!(out, vec![0.0, 0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn sample_wise_processors_are_block_size_invariant() {
        let signal = sine(997.0, -6.0, 8192);
        assert_block_size_invariant(&mut Chain::passthrough(), &signal);
        assert_block_size_invariant(&mut OneSampleDelay { held: 0.0 }, &signal);
        assert_block_size_invariant(
            &mut Chain::new(vec![Box::new(OneSampleDelay { held: 0.0 })]),
            &signal,
        );
    }
}
