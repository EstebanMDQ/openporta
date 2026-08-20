//! Bandwidth limiting: the cassette's frequency response.
//!
//! Two cascaded Butterworth low-pass biquads near 11kHz give a fourth-
//! order top-end rolloff, and one high-pass biquad near 60Hz thins the
//! bottom the way a small tape transport does. Biquads rather than
//! one-poles because a one-pole cascade flattens out near Nyquist (the
//! bilinear warping works against us there) and leaves 20kHz only ~9 dB
//! down, which reads as hi-fi rather than cassette.

use crate::{AudioProcessor, SAMPLE_RATE};

/// Direct-form-1 biquad. Coefficients follow the RBJ cookbook.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

/// Butterworth Q: maximally flat passband, no resonant peak.
const BUTTERWORTH_Q: f32 = core::f32::consts::FRAC_1_SQRT_2;

impl Biquad {
    fn low_pass(cutoff_hz: f32) -> Self {
        let w0 = core::f32::consts::TAU * cutoff_hz / SAMPLE_RATE as f32;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * BUTTERWORTH_Q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos) / 2.0) / a0,
            b1: (1.0 - cos) / a0,
            b2: ((1.0 - cos) / 2.0) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn high_pass(cutoff_hz: f32) -> Self {
        let w0 = core::f32::consts::TAU * cutoff_hz / SAMPLE_RATE as f32;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * BUTTERWORTH_Q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 + cos) / 2.0) / a0,
            b1: (-(1.0 + cos)) / a0,
            b2: ((1.0 + cos) / 2.0) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    fn clear(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

pub struct Bandwidth {
    lows: [Biquad; 2],
    high: Biquad,
}

impl Bandwidth {
    pub fn new(lpf_hz: f32, hpf_hz: f32) -> Self {
        Self {
            lows: [Biquad::low_pass(lpf_hz), Biquad::low_pass(lpf_hz)],
            high: Biquad::high_pass(hpf_hz),
        }
    }

    /// Cassette defaults: 11kHz corner, 60Hz bottom.
    pub fn cassette() -> Self {
        Self::new(11_000.0, 60.0)
    }
}

impl AudioProcessor for Bandwidth {
    fn process(&mut self, block: &mut [f32]) {
        for s in block.iter_mut() {
            let mut x = *s;
            for low in &mut self.lows {
                x = low.tick(x);
            }
            *s = self.high.tick(x);
        }
    }

    fn reset(&mut self) {
        for low in &mut self.lows {
            low.clear();
        }
        self.high.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assert_block_size_invariant, process_in_blocks};
    use porta_testkit::meter::rms_dbfs;
    use porta_testkit::signal::sine;

    /// Level change at `freq` after the filter, in dB.
    fn response_db(freq: f32) -> f32 {
        let input = sine(freq, -6.0, 48_000);
        let mut f = Bandwidth::cassette();
        let out = process_in_blocks(&mut f, &input, 512);
        // Skip the settling transient.
        rms_dbfs(&out[4800..]) - rms_dbfs(&input[4800..])
    }

    #[test]
    fn midrange_passes_nearly_flat() {
        for freq in [300.0, 1000.0, 3000.0] {
            let r = response_db(freq);
            assert!(r > -1.0 && r < 0.5, "{freq} Hz responded {r:.2} dB");
        }
    }

    #[test]
    fn high_end_rolls_off_like_tape() {
        assert!(response_db(15_000.0) < -12.0, "{}", response_db(15_000.0));
        assert!(response_db(20_000.0) < -25.0, "{}", response_db(20_000.0));
        // Monotonic, not resonant.
        assert!(response_db(20_000.0) < response_db(15_000.0));
        assert!(response_db(15_000.0) < response_db(8_000.0));
        assert!(response_db(8_000.0) < response_db(3_000.0));
    }

    #[test]
    fn low_end_thins_out() {
        assert!(response_db(20.0) < -12.0, "{}", response_db(20.0));
        assert!(response_db(20.0) < response_db(100.0));
    }

    #[test]
    fn no_resonant_peak_in_the_passband() {
        for freq in [500.0, 2000.0, 5000.0, 8000.0] {
            assert!(response_db(freq) < 0.5, "{freq} Hz peaked");
        }
    }

    #[test]
    fn block_size_invariant() {
        let mut f = Bandwidth::cassette();
        assert_block_size_invariant(&mut f, &sine(997.0, -6.0, 8192));
    }

    #[test]
    fn reset_clears_state() {
        let mut f = Bandwidth::cassette();
        let signal = sine(1000.0, -6.0, 4800);
        let first = process_in_blocks(&mut f, &signal, 512);
        let second = process_in_blocks(&mut f, &signal, 512);
        assert_eq!(first, second, "reset must make runs reproducible");
    }
}
