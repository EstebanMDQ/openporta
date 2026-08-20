//! The portastudio engine: tape, transport, record/bounce/undo, mixer,
//! project persistence. Hardware-agnostic: buffers in, buffers out.

/// Number of tape tracks. Fixed: this is a 4-track machine.
pub const NUM_TRACKS: usize = 4;

pub mod mixer;
pub mod tape;
pub mod transport;

pub use porta_dsp::SAMPLE_RATE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_tracks() {
        assert_eq!(NUM_TRACKS, 4);
    }
}
