//! Helpers for exercising processors from tests. Kept in the crate (not
//! behind cfg(test)) so the engine and app test suites can use them too.

use crate::AudioProcessor;

/// Run `signal` through `p` in `block` sized chunks, returning the output.
/// Resets the processor first so results are reproducible.
pub fn process_in_blocks<P: AudioProcessor>(p: &mut P, signal: &[f32], block: usize) -> Vec<f32> {
    p.reset();
    let mut out = signal.to_vec();
    for chunk in out.chunks_mut(block) {
        p.process(chunk);
    }
    out
}

/// Assert a processor is block-size invariant: the same input split into
/// different block sizes must produce bit-identical output. Any processor
/// with per-block (rather than per-sample) state fails this.
pub fn assert_block_size_invariant<P: AudioProcessor>(p: &mut P, signal: &[f32]) {
    let reference = process_in_blocks(p, signal, 64);
    for block in [1usize, 37, 128, 512, 4096] {
        let other = process_in_blocks(p, signal, block);
        assert_eq!(
            reference.len(),
            other.len(),
            "length changed at block size {block}"
        );
        for (i, (a, b)) in reference.iter().zip(&other).enumerate() {
            assert_eq!(
                a, b,
                "block size {block} diverges at sample {i}: {a} vs {b}"
            );
        }
    }
}
