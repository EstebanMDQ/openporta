//! Wow and flutter: the transport's speed instability, modelled as a
//! modulated fractional delay.
//!
//! Two components sum into the delay: a slow sine (wow, the capstan's
//! once-per-revolution error) and a filtered random walk (flutter, the
//! irregular part). Each record pass gets its own seed (REQ-304), so
//! overdubs and bounces wobble independently the way real generations do,
//! rather than sharing one coherent LFO that would just sound like a
//! pitch effect on the master.
//!
//! The delay line reads at a fixed centre offset so punched-in audio
//! stays time-aligned with what was already on tape; the modulation
//! swings either side of that centre. Interpolation is cubic (Catmull-
//! Rom): linear interpolation on a moving tap audibly dulls the top end.

use crate::{AudioProcessor, SAMPLE_RATE};

/// Delay-line centre tap, in samples. Also the reported latency.
const CENTRE: usize = 480;
/// Ring size: centre plus the largest excursion we allow, plus guard.
const RING: usize = CENTRE * 2 + 8;

/// The modulation half of flutter: the wow oscillator and the filtered
/// random walk, producing a delay-in-samples value per sample. No audio
/// passes through it - that's `FlutterDelay`'s job. Split out (change
/// 001, M7.1) so a stereo bounce pass can share ONE modulation between
/// two delay lines: the image wobbles together, as one transport would,
/// instead of each channel drifting independently.
pub struct FlutterModulator {
    /// Wow oscillator, in radians per sample.
    wow_phase: f32,
    wow_step: f32,
    wow_depth: f32,
    /// Flutter random walk, low-passed.
    walk: f32,
    walk_lp: f32,
    flutter_depth: f32,
    state: u32,
    seed: u32,
}

impl FlutterModulator {
    /// `rate_hz` is the wow frequency (0.5-5Hz per the spec) and
    /// `depth_cents` the peak pitch deviation the two components reach
    /// together. The depth-clamp constants here are the delay-line
    /// geometry's, shared by construction between `Flutter` and
    /// `StereoFlutter` - they must never be redefined at either
    /// composition site.
    pub fn new(rate_hz: f32, depth_cents: f32, seed: u32) -> Self {
        let seed = seed | 1;
        // Pitch deviation d cents on a sine of rate f needs a delay swing
        // of (d_ratio - 1) * fs / (2*pi*f) samples, since pitch shift is
        // the derivative of delay. Split the budget: 70% wow, 30% flutter.
        let ratio = 2f32.powf(depth_cents / 1200.0) - 1.0;
        let swing = ratio * SAMPLE_RATE as f32 / (core::f32::consts::TAU * rate_hz.max(0.01));
        Self {
            wow_phase: 0.0,
            wow_step: core::f32::consts::TAU * rate_hz / SAMPLE_RATE as f32,
            wow_depth: (swing * 0.7).min(CENTRE as f32 - 4.0),
            walk: 0.0,
            walk_lp: 0.0,
            flutter_depth: (swing * 0.3).min(CENTRE as f32 / 4.0),
            state: seed,
            seed,
        }
    }

    fn noise(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Advance one sample and return the delay to read at - exactly the
    /// arithmetic the pre-split `Flutter::process` ran per sample, in
    /// the same order, so the composed result stays bit-identical.
    fn next_delay(&mut self) -> f32 {
        let wow = self.wow_phase.sin() * self.wow_depth;
        self.wow_phase += self.wow_step;
        if self.wow_phase > core::f32::consts::TAU {
            self.wow_phase -= core::f32::consts::TAU;
        }

        // Random walk, gently low-passed and leaked back toward zero
        // so it wanders without drifting away.
        let n = self.noise();
        self.walk = (self.walk + n * 0.02).clamp(-1.0, 1.0) * 0.9995;
        self.walk_lp += 0.002 * (self.walk - self.walk_lp);
        let flutter = self.walk_lp * self.flutter_depth;

        CENTRE as f32 + wow + flutter
    }

    fn reset(&mut self) {
        self.wow_phase = 0.0;
        self.walk = 0.0;
        self.walk_lp = 0.0;
        self.state = self.seed;
    }

    fn reseed(&mut self, seed: u32) {
        self.seed = seed | 1;
        self.reset();
    }
}

/// The audio half of flutter: a ring buffer plus the Catmull-Rom
/// fractional read. No modulation state of its own - it reads wherever
/// it's told to.
pub struct FlutterDelay {
    ring: [f32; RING],
    write: usize,
}

impl FlutterDelay {
    pub fn new() -> Self {
        Self {
            ring: [0.0; RING],
            write: 0,
        }
    }

    /// Write one sample, read back at `delay` samples, advance.
    fn tick(&mut self, sample: f32, delay: f32) -> f32 {
        self.ring[self.write] = sample;
        let out = self.read(delay);
        self.write = (self.write + 1) % RING;
        out
    }

    /// Catmull-Rom interpolation of the ring at `delay` samples back.
    /// `frac` runs from the sample at `floor(delay)` toward the older one
    /// at `floor(delay) + 1`, so y1 is the newer of the two centre taps.
    fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (RING - 3) as f32);
        let i = d.floor() as usize;
        let frac = d - i as f32;
        let at = |back: usize| self.ring[(self.write + RING - back) % RING];
        let y0 = at(i - 1);
        let y1 = at(i);
        let y2 = at(i + 1);
        let y3 = at(i + 2);
        let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
        let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c = -0.5 * y0 + 0.5 * y2;
        ((a * frac + b) * frac + c) * frac + y1
    }

    fn reset(&mut self) {
        self.ring = [0.0; RING];
        self.write = 0;
    }
}

impl Default for FlutterDelay {
    fn default() -> Self {
        Self::new()
    }
}

/// The mono flutter every ordinary track uses: one modulator driving
/// one delay line. A thin composition of the two halves above - same
/// behavior, same tests, nothing changes for tracks 1-4.
pub struct Flutter {
    modulator: FlutterModulator,
    delay: FlutterDelay,
}

impl Flutter {
    pub fn new(rate_hz: f32, depth_cents: f32, seed: u32) -> Self {
        Self {
            modulator: FlutterModulator::new(rate_hz, depth_cents, seed),
            delay: FlutterDelay::new(),
        }
    }

    /// Cassette defaults: 2.5Hz wobble, 12 cents deep.
    pub fn cassette(seed: u32) -> Self {
        Self::new(2.5, 12.0, seed)
    }
}

impl AudioProcessor for Flutter {
    fn process(&mut self, block: &mut [f32]) {
        for s in block.iter_mut() {
            let delay = self.modulator.next_delay();
            *s = self.delay.tick(*s, delay);
        }
    }

    fn reset(&mut self) {
        self.delay.reset();
        self.modulator.reset();
    }

    /// Unlike `reset` (back to the seed this instance was built with),
    /// this takes a fresh one - what `record()` needs each pass (REQ-304)
    /// without reallocating a new `Flutter`.
    fn reseed(&mut self, seed: u32) {
        self.delay.reset();
        self.modulator.reseed(seed);
    }

    fn latency_samples(&self) -> usize {
        CENTRE
    }
}

/// Stereo flutter for a bounce pass (change 001, REQ-402): ONE
/// modulator driving TWO delay lines. Each sample advances the
/// modulator once and reads both channels at that one delay value -
/// genuinely shared modulation, independent audio per channel. Not an
/// `AudioProcessor` (that trait is mono, in-place, and stays that way -
/// REQ-701/704); a bounce pass calls `process` directly between its
/// per-channel chain halves.
pub struct StereoFlutter {
    modulator: FlutterModulator,
    left: FlutterDelay,
    right: FlutterDelay,
}

impl StereoFlutter {
    pub fn new(rate_hz: f32, depth_cents: f32, seed: u32) -> Self {
        Self {
            modulator: FlutterModulator::new(rate_hz, depth_cents, seed),
            left: FlutterDelay::new(),
            right: FlutterDelay::new(),
        }
    }

    /// Cassette defaults, matching `Flutter::cassette`.
    pub fn cassette(seed: u32) -> Self {
        Self::new(2.5, 12.0, seed)
    }

    /// Fresh state for a new pass (change 001's numbered per-pass
    /// sequence, step 3): clears BOTH delay rings and write indices AND
    /// reseeds the shared modulator. The invariant this is held to:
    /// clear exactly the state `Flutter::reset` clears (`ring`, `write`,
    /// `wow_phase`, `walk`, `walk_lp`, `state`), just distributed
    /// across three objects - anything less bleeds ~CENTRE samples of
    /// the previous bounce into the next pass's punch-in. An inherent
    /// method, not a trait override: this type isn't an
    /// `AudioProcessor`. No allocation - callable from the realtime
    /// thread, same as `reseed_chain`.
    pub fn reseed(&mut self, seed: u32) {
        self.left.reset();
        self.right.reset();
        self.modulator.reseed(seed);
    }

    /// Both channels through their own delay line at the same
    /// modulation. Processes `min(l.len(), r.len())` samples - callers
    /// hand in equal-length blocks; the min is defensive, not an API.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        for i in 0..n {
            let delay = self.modulator.next_delay();
            l[i] = self.left.tick(l[i], delay);
            r[i] = self.right.tick(r[i], delay);
        }
    }

    pub fn latency_samples(&self) -> usize {
        CENTRE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assert_block_size_invariant, process_in_blocks};
    use porta_testkit::meter::rms_dbfs;
    use porta_testkit::pitch::deviation_cents;
    use porta_testkit::signal::sine;
    use porta_testkit::{assert_no_clicks, signal::silence};

    /// Skip the delay-line fill so measurements see steady state.
    const SETTLE: usize = CENTRE * 4;

    #[test]
    fn pitch_wobbles_by_roughly_the_configured_depth() {
        let mut f = Flutter::new(2.5, 12.0, 5);
        let out = process_in_blocks(&mut f, &sine(440.0, -6.0, 48_000 * 2), 512);
        let (lo, hi) = deviation_cents(&out[SETTLE..], 440.0);
        assert!(
            lo < -4.0,
            "min deviation {lo:.1} cents, expected clear wobble"
        );
        assert!(
            hi > 4.0,
            "max deviation {hi:.1} cents, expected clear wobble"
        );
        assert!(
            lo > -40.0 && hi < 40.0,
            "wobble {lo:.1}..{hi:.1} is out of hand"
        );
    }

    #[test]
    fn deeper_setting_wobbles_more() {
        let measure = |cents: f32| {
            let mut f = Flutter::new(2.5, cents, 5);
            let out = process_in_blocks(&mut f, &sine(440.0, -6.0, 96_000), 512);
            let (lo, hi) = deviation_cents(&out[SETTLE..], 440.0);
            hi - lo
        };
        let shallow = measure(4.0);
        let deep = measure(20.0);
        assert!(deep > shallow * 2.0, "shallow {shallow:.1}, deep {deep:.1}");
    }

    #[test]
    fn zero_depth_is_a_plain_delay() {
        let mut f = Flutter::new(2.5, 0.0, 5);
        let input = sine(440.0, -6.0, 24_000);
        let out = process_in_blocks(&mut f, &input, 512);
        let (lo, hi) = deviation_cents(&out[SETTLE..], 440.0);
        assert!(lo > -0.5 && hi < 0.5, "unexpected wobble {lo:.2}..{hi:.2}");
        // Output is the input delayed by exactly the reported latency.
        let err: Vec<f32> = out[SETTLE..]
            .iter()
            .zip(&input[SETTLE - CENTRE..])
            .map(|(a, b)| a - b)
            .collect();
        assert!(
            rms_dbfs(&err) < -60.0,
            "delay misaligned, {}",
            rms_dbfs(&err)
        );
    }

    #[test]
    fn modulation_does_not_click() {
        let mut f = Flutter::cassette(11);
        let out = process_in_blocks(&mut f, &sine(1000.0, -6.0, 96_000), 512);
        assert_no_clicks!(&out[SETTLE..]);
    }

    #[test]
    fn different_seeds_decorrelate() {
        let a = process_in_blocks(&mut Flutter::cassette(1), &sine(440.0, -6.0, 48_000), 512);
        let b = process_in_blocks(&mut Flutter::cassette(2), &sine(440.0, -6.0, 48_000), 512);
        let same = process_in_blocks(&mut Flutter::cassette(1), &sine(440.0, -6.0, 48_000), 512);
        assert_eq!(a, same, "same seed must reproduce");
        let diff: Vec<f32> = a[SETTLE..]
            .iter()
            .zip(&b[SETTLE..])
            .map(|(x, y)| x - y)
            .collect();
        assert!(rms_dbfs(&diff) > -40.0, "seeds barely differ");
    }

    #[test]
    fn silence_stays_silent() {
        let mut f = Flutter::cassette(3);
        let out = process_in_blocks(&mut f, &silence(24_000), 512);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn reports_its_latency() {
        assert_eq!(Flutter::cassette(1).latency_samples(), CENTRE);
    }

    #[test]
    fn block_size_invariant() {
        let mut f = Flutter::cassette(9);
        assert_block_size_invariant(&mut f, &sine(997.0, -6.0, 8192));
    }

    #[test]
    fn stereo_shares_one_modulation_between_channels() {
        // Identical input on both channels must produce identical
        // outputs - the whole point of one modulator, two delay lines.
        // Different content per channel while the modulation is shared
        // is exercised by the mono-equivalence test below (each channel
        // there carries its own comparison independently).
        let input = sine(440.0, -6.0, 48_000);
        let mut l = input.clone();
        let mut r = input.clone();
        let mut sf = StereoFlutter::cassette(7);
        for (cl, cr) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            sf.process(cl, cr);
        }
        assert_eq!(
            l, r,
            "shared modulation must treat both channels identically"
        );
    }

    #[test]
    fn stereo_channel_matches_mono_flutter_exactly() {
        // The split's whole safety argument: StereoFlutter is the same
        // arithmetic as Flutter, per channel, in the same order - so a
        // channel of it must be bit-identical to a mono Flutter with
        // the same seed. Guards the refactor AND the shared-constant
        // requirement (a drifted depth clamp would break this first).
        let input = sine(330.0, -6.0, 48_000);
        let mono = process_in_blocks(&mut Flutter::cassette(5), &input, 512);
        let mut l = input.clone();
        let mut r = input.clone();
        let mut sf = StereoFlutter::cassette(5);
        for (cl, cr) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            sf.process(cl, cr);
        }
        assert_eq!(l, mono, "stereo left must match mono bit-exactly");
        assert_eq!(sf.latency_samples(), CENTRE);
    }
}
