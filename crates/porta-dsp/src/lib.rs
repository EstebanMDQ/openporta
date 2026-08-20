//! Lo-fi tape character DSP: the AudioProcessor trait and the processors
//! that get baked onto tape at record time.

/// Internal engine sample rate. Fixed for v1; no runtime plumbing.
pub const SAMPLE_RATE: u32 = 48_000;

/// Largest block any processor must accept in one `process` call.
pub const MAX_BLOCK: usize = 4096;

/// A mono, in-place audio processor. Implementations must not allocate,
/// lock, or touch the filesystem inside `process`.
pub trait AudioProcessor: Send {
    fn process(&mut self, block: &mut [f32]);
    fn reset(&mut self);
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

    #[test]
    fn passthrough_chain_is_identity() {
        let mut chain = Chain::passthrough();
        let mut block = vec![0.5f32, -0.25, 1.0, -1.0];
        let original = block.clone();
        chain.process(&mut block);
        assert_eq!(block, original);
    }
}
