//! Tape saturation: tanh soft clipping with drive and makeup gain.
//!
//! Drive scales the input into the knee; makeup restores unity for small
//! signals so raising drive adds harmonics rather than just level. The
//! curve is odd-symmetric, so it generates odd harmonics like a real tape
//! path rather than the even harmonics of an asymmetric stage.

use crate::AudioProcessor;

pub struct Saturation {
    drive: f32,
    makeup: f32,
}

impl Saturation {
    /// `drive_db` above 0 pushes further into the knee. 0 dB is nearly
    /// transparent for normal levels.
    pub fn new(drive_db: f32) -> Self {
        let drive = 10f32.powf(drive_db / 20.0);
        Self {
            drive,
            // tanh(x) ~= x near zero, so 1/drive keeps quiet material at
            // its original level regardless of drive.
            makeup: 1.0 / drive,
        }
    }
}

impl AudioProcessor for Saturation {
    fn process(&mut self, block: &mut [f32]) {
        for s in block.iter_mut() {
            *s = (*s * self.drive).tanh() * self.makeup;
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assert_block_size_invariant, process_in_blocks};
    use porta_testkit::meter::{peak_dbfs, rms_dbfs};
    use porta_testkit::signal::{dc, sine};
    use porta_testkit::spectral::thd_db;

    #[test]
    fn quiet_signal_stays_clean() {
        let mut s = Saturation::new(6.0);
        let input = sine(1000.0, -40.0, 48_000);
        let out = process_in_blocks(&mut s, &input, 512);
        assert!(
            thd_db(&out, 1000.0, 7) < -60.0,
            "{}",
            thd_db(&out, 1000.0, 7)
        );
        // And it did not change the level meaningfully.
        assert!((rms_dbfs(&out) - rms_dbfs(&input)).abs() < 0.1);
    }

    #[test]
    fn hot_signal_generates_harmonics() {
        let input = sine(1000.0, 0.0, 48_000);
        let clean = thd_db(&input, 1000.0, 7);
        let mut s = Saturation::new(18.0);
        let out = process_in_blocks(&mut s, &input, 512);
        let dirty = thd_db(&out, 1000.0, 7);
        assert!(
            dirty > clean + 40.0,
            "clean {clean:.1} dB, driven {dirty:.1} dB"
        );
        assert!(
            dirty > -25.0,
            "expected audible harmonics, got {dirty:.1} dB"
        );
    }

    #[test]
    fn more_drive_means_more_harmonics() {
        let input = sine(1000.0, -6.0, 48_000);
        let mut last = f32::NEG_INFINITY;
        for drive in [0.0, 6.0, 12.0, 24.0] {
            let mut s = Saturation::new(drive);
            let out = process_in_blocks(&mut s, &input, 512);
            let thd = thd_db(&out, 1000.0, 7);
            assert!(
                thd > last,
                "drive {drive} gave {thd:.1} dB, not above {last:.1}"
            );
            last = thd;
        }
    }

    #[test]
    fn output_is_bounded_and_finite_under_abuse() {
        let mut s = Saturation::new(24.0);
        let mut block = vec![50.0f32, -50.0, 1e9, -1e9, 0.0, f32::MIN_POSITIVE];
        s.process(&mut block);
        assert!(block.iter().all(|v| v.is_finite()), "{block:?}");
        assert!(peak_dbfs(&block) <= 0.01, "peak {}", peak_dbfs(&block));
    }

    #[test]
    fn dc_is_compressed_not_inverted() {
        let mut s = Saturation::new(12.0);
        let out = process_in_blocks(&mut s, &dc(0.9, 100), 25);
        assert!(out.iter().all(|&v| v > 0.0 && v < 0.9));
    }

    #[test]
    fn block_size_invariant() {
        let mut s = Saturation::new(12.0);
        assert_block_size_invariant(&mut s, &sine(997.0, -3.0, 8192));
    }
}
