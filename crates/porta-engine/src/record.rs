//! Record passes: the unit of recording and of undo.
//!
//! A pass opens when record engages on an armed track and closes on punch
//! out, stop, or end of tape. Before any sample is overwritten, the
//! displaced tape content is captured, so undo restores the region
//! byte-exactly (REQ-501) and the cost is proportional to what was
//! recorded, not to tape length.
//!
//! Punch boundaries get a 5ms linear crossfade (REQ-302): the in-fade is
//! applied as audio is written, the out-fade is applied retroactively to
//! the tail of the written region when the pass closes, blending back
//! toward the displaced content.

use crate::tape::Tape;
use porta_dsp::SAMPLE_RATE;

/// Punch crossfade length: 5ms.
pub const XFADE_SAMPLES: usize = (SAMPLE_RATE as usize) * 5 / 1000;

/// TPDF dither at one LSB, then quantize to i16 (REQ-102). The dither
/// state is carried by the caller so successive blocks stay decorrelated.
pub struct Dither {
    prev: f32,
    state: u32,
}

impl Dither {
    pub fn new(seed: u32) -> Self {
        Self {
            prev: 0.0,
            state: seed | 1,
        }
    }

    fn next_uniform(&mut self) -> f32 {
        // xorshift32; deterministic given the seed (REQ-702).
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state as f32 / u32::MAX as f32) - 0.5
    }

    pub fn quantize(&mut self, sample: f32) -> i16 {
        let lsb = 1.0 / 32768.0;
        let r = self.next_uniform();
        let tpdf = (r - self.prev) * lsb;
        self.prev = r;
        let v = (sample + tpdf) * 32768.0;
        v.round().clamp(-32768.0, 32767.0) as i16
    }
}

/// An open or closed record pass. `displaced` holds the tape content that
/// the pass overwrote, in order, starting at `start`.
pub struct RecordPass {
    pub track: usize,
    pub start: usize,
    pub displaced: Vec<i16>,
    dither: Dither,
    scratch: Vec<i16>,
}

impl RecordPass {
    pub fn new(track: usize, start: usize, seed: u32) -> Self {
        Self {
            track,
            start,
            displaced: Vec::new(),
            dither: Dither::new(seed),
            scratch: Vec::new(),
        }
    }

    /// Samples written so far.
    pub fn len(&self) -> usize {
        self.displaced.len()
    }

    pub fn is_empty(&self) -> bool {
        self.displaced.is_empty()
    }

    /// Write one block of already-processed audio to tape, capturing what
    /// it displaces and applying the punch-in fade over the first
    /// `XFADE_SAMPLES` of the pass. Returns samples written.
    pub fn write_block(&mut self, tape: &mut Tape, block: &[f32]) -> usize {
        let pos = self.start + self.len();
        let room = tape.len_samples().saturating_sub(pos);
        let n = block.len().min(room);
        if n == 0 {
            return 0;
        }

        let old_len = self.displaced.len();
        self.displaced.resize(old_len + n, 0);
        tape.read_raw(self.track, pos, &mut self.displaced[old_len..]);

        self.scratch.clear();
        self.scratch.reserve(n);
        for (i, &s) in block[..n].iter().enumerate() {
            let idx = old_len + i;
            let new = self.dither.quantize(s);
            let value = if idx < XFADE_SAMPLES {
                let t = (idx + 1) as f32 / XFADE_SAMPLES as f32;
                let old = f32::from(self.displaced[idx]);
                (f32::from(new) * t + old * (1.0 - t)).round() as i16
            } else {
                new
            };
            self.scratch.push(value);
        }
        tape.write_raw(self.track, pos, &self.scratch);
        n
    }

    /// Close the pass, applying the punch-out fade to the tail of the
    /// written region so it blends back into the displaced content.
    /// Skipped when the pass ran to the very end of the tape, where there
    /// is nothing to blend back into.
    pub fn finish(&mut self, tape: &mut Tape) {
        let len = self.len();
        if len == 0 {
            return;
        }
        let end = self.start + len;
        if end >= tape.len_samples() {
            return;
        }
        let fade = XFADE_SAMPLES.min(len);
        let tail_start = end - fade;
        self.scratch.resize(fade, 0);
        tape.read_raw(self.track, tail_start, &mut self.scratch[..fade]);
        for i in 0..fade {
            let t = (i + 1) as f32 / fade as f32;
            let new = f32::from(self.scratch[i]);
            let old = f32::from(self.displaced[len - fade + i]);
            self.scratch[i] = (new * (1.0 - t) + old * t).round() as i16;
        }
        let tail: Vec<i16> = self.scratch[..fade].to_vec();
        tape.write_raw(self.track, tail_start, &tail);
    }

    /// Restore the displaced audio, returning what was on tape in its
    /// place so the operation can be redone.
    pub fn undo(&self, tape: &mut Tape) -> Vec<i16> {
        let mut current = vec![0i16; self.displaced.len()];
        tape.read_raw(self.track, self.start, &mut current);
        tape.write_raw(self.track, self.start, &self.displaced);
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::Tape;
    use porta_testkit::meter::rms_dbfs;
    use porta_testkit::signal::sine;

    fn read_f32(tape: &Tape, track: usize, start: usize, len: usize) -> Vec<f32> {
        let mut out = vec![0f32; len];
        tape.read(track, start, &mut out);
        out
    }

    #[test]
    fn dither_is_deterministic_and_near_transparent() {
        let s = sine(1000.0, -6.0, 48_000);
        let mut a = Dither::new(7);
        let mut b = Dither::new(7);
        let qa: Vec<i16> = s.iter().map(|&x| a.quantize(x)).collect();
        let qb: Vec<i16> = s.iter().map(|&x| b.quantize(x)).collect();
        assert_eq!(qa, qb, "same seed must reproduce");

        let back: Vec<f32> = qa.iter().map(|&v| f32::from(v) / 32768.0).collect();
        let err: Vec<f32> = s.iter().zip(&back).map(|(a, b)| a - b).collect();
        assert!(rms_dbfs(&err) < -85.0, "dither noise {}", rms_dbfs(&err));
    }

    #[test]
    fn records_and_captures_displaced_audio() {
        let mut tape = Tape::new(48_000);
        let old = sine(200.0, -6.0, 10_000);
        let mut pass0 = RecordPass::new(0, 1000, 1);
        pass0.write_block(&mut tape, &old);
        pass0.finish(&mut tape);

        let new = sine(3000.0, -6.0, 5000);
        let mut pass1 = RecordPass::new(0, 2000, 2);
        assert_eq!(pass1.write_block(&mut tape, &new), 5000);
        assert_eq!(pass1.displaced.len(), 5000);
        pass1.finish(&mut tape);

        // Mid-region (clear of both crossfades) is the new signal.
        let mid = read_f32(&tape, 0, 3000, 1000);
        let err: Vec<f32> = mid
            .iter()
            .zip(&new[1000..2000])
            .map(|(a, b)| a - b)
            .collect();
        assert!(rms_dbfs(&err) < -80.0);
    }

    #[test]
    fn undo_restores_byte_exactly_and_redo_is_symmetric() {
        let mut tape = Tape::new(48_000);
        let mut p0 = RecordPass::new(1, 0, 1);
        p0.write_block(&mut tape, &sine(440.0, -6.0, 20_000));
        p0.finish(&mut tape);
        let mut before = vec![0i16; 20_000];
        tape.read_raw(1, 0, &mut before);

        let mut p1 = RecordPass::new(1, 5000, 2);
        p1.write_block(&mut tape, &sine(880.0, -3.0, 5000));
        p1.finish(&mut tape);

        let redo_payload = p1.undo(&mut tape);
        let mut after_undo = vec![0i16; 20_000];
        tape.read_raw(1, 0, &mut after_undo);
        assert_eq!(after_undo, before, "undo must restore byte-exactly");

        tape.write_raw(1, p1.start, &redo_payload);
        let mut after_redo = vec![0i16; 5000];
        tape.read_raw(1, 5000, &mut after_redo);
        assert_eq!(after_redo, redo_payload, "redo must be symmetric");
    }

    #[test]
    fn punch_boundaries_do_not_click() {
        let mut tape = Tape::new(48_000);
        let mut p0 = RecordPass::new(2, 0, 1);
        p0.write_block(&mut tape, &sine(220.0, -6.0, 40_000));
        p0.finish(&mut tape);

        // Punch a loud, phase-unrelated signal into the middle.
        let mut p1 = RecordPass::new(2, 10_000, 2);
        p1.write_block(&mut tape, &sine(1777.0, -3.0, 10_000));
        p1.finish(&mut tape);

        let audio = read_f32(&tape, 2, 0, 40_000);
        porta_testkit::assert_no_clicks!(&audio);
    }

    #[test]
    fn writing_stops_at_tape_end() {
        let mut tape = Tape::new(1000);
        let mut p = RecordPass::new(3, 900, 1);
        assert_eq!(p.write_block(&mut tape, &sine(440.0, -6.0, 500)), 100);
        assert_eq!(p.write_block(&mut tape, &sine(440.0, -6.0, 500)), 0);
        p.finish(&mut tape);
        assert_eq!(p.len(), 100);
    }

    #[test]
    fn other_tracks_are_untouched() {
        let mut tape = Tape::new(10_000);
        let mut p0 = RecordPass::new(0, 0, 1);
        p0.write_block(&mut tape, &sine(300.0, -6.0, 10_000));
        p0.finish(&mut tape);

        let mut others = [vec![0i16; 10_000], vec![0i16; 10_000], vec![0i16; 10_000]];
        for (i, buf) in others.iter_mut().enumerate() {
            tape.read_raw(i + 1, 0, buf);
        }

        let mut p1 = RecordPass::new(0, 2000, 2);
        p1.write_block(&mut tape, &sine(900.0, -6.0, 3000));
        p1.finish(&mut tape);

        for (i, buf) in others.iter().enumerate() {
            let mut now = vec![0i16; 10_000];
            tape.read_raw(i + 1, 0, &mut now);
            assert_eq!(&now, buf, "track {} changed", i + 1);
        }
    }
}
