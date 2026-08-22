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
//!
//! # Realtime-safe capture (REQ-902)
//!
//! Displaced audio is captured in fixed-size chunks (`tape::CHUNK_SAMPLES`,
//! matching REQ-802's own tape-save granularity) instead of one buffer
//! reserved for the whole remaining tape - `Command::Record` is not a
//! blocking command, so `Engine::record()` and every `write_block` call
//! run directly on the realtime audio thread, and a `reserve_exact` sized
//! to up to 30 minutes of tape there is a genuine REQ-902 violation
//! (found while investigating the stereo-bounce proposal's REQ-902
//! question).
//!
//! `Journal` hands a new pass its whole track's reserve of pre-allocated
//! chunk buffers (`spares`, see `with_spares`) up front, at pass-start -
//! one `mem::take` moving an already-allocated `Vec` wholesale, not a
//! per-pass allocation of its own; as `current` fills, a fresh chunk
//! comes from `spares` with no allocation either. There is deliberately
//! no attempt to refill `spares` *during* a live pass - `Engine` is
//! exclusively owned by the realtime thread while a session is
//! connected (the same reason blocking commands like Save/Undo fully
//! disconnect first), so nothing can hand more buffers over mid-pass
//! without its own background thread and wait-free queues. Instead each
//! track keeps its own dedicated reserve (`CHUNK_POOL_PER_TRACK` chunks,
//! ~2 minutes of continuous recording), given back in full by
//! `Journal::push_pass` the instant a pass closes (whatever it didn't
//! use) and replenished further, chunk by chunk, at the existing
//! off-thread touchpoint (`Journal::flush_pending`, run by Save/Undo/
//! Redo) as passes are written out - both plain moves, not allocations,
//! which is what keeps a track's reserve from draining to nothing after
//! a handful of ordinary takes (an earlier version of this fix got that
//! wrong: it only ever gave back the chunks a pass *used*, never the
//! ones it reserved and didn't, so the reserve shrank on every pass
//! regardless of length - caught in review, fixed here). A single pass
//! longer than its own reserve, with nothing flushing in between, falls
//! back to an ordinary allocation for the overflow rather than
//! corrupting undo data or refusing to record - rare in practice (most
//! takes are far shorter than 2 minutes), and counted via
//! `allocated_on_thread` rather than silently swallowed.

use crate::tape::{Tape, CHUNK_SAMPLES};
use porta_dsp::SAMPLE_RATE;
use std::collections::VecDeque;

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

/// An open or closed record pass. Displaced tape content (what the pass
/// overwrote) is captured in fixed-size chunks - `chunks` holds closed
/// ones in order, `current` the one still being filled. See the module
/// doc comment for why.
pub struct RecordPass {
    pub track: usize,
    pub start: usize,
    chunks: Vec<Vec<i16>>,
    current: Vec<i16>,
    total_len: usize,
    /// Pre-reserved chunk buffers handed to this pass at construction
    /// (see `with_spares`) - consumed as `current` fills. Empty for
    /// `new()`, which simply allocates a fresh chunk on every rollover
    /// instead; fine off the realtime thread (tests, the session-script
    /// runner, offline rendering).
    spares: Vec<Vec<i16>>,
    /// Set once this pass had to allocate a chunk itself because
    /// `spares` ran dry - see the module doc comment.
    pub allocated_on_thread: bool,
    /// The last (up to) `XFADE_SAMPLES` displaced samples, in order,
    /// independent of chunk boundaries - so the punch-out fade in
    /// `finish` never needs to reach back into an already-closed chunk.
    tail: VecDeque<i16>,
    dither: Dither,
    scratch: Vec<i16>,
}

impl RecordPass {
    pub fn new(track: usize, start: usize, seed: u32) -> Self {
        Self {
            track,
            start,
            chunks: Vec::new(),
            current: Vec::new(),
            total_len: 0,
            spares: Vec::new(),
            allocated_on_thread: false,
            tail: VecDeque::with_capacity(XFADE_SAMPLES),
            dither: Dither::new(seed),
            scratch: Vec::new(),
        }
    }

    /// Same as `new`, but starts with a reserve of pre-allocated chunk
    /// buffers (from `Journal::take_spares`) instead of allocating its
    /// first chunk fresh - see the module doc comment. `max_block` still
    /// sizes the small per-block `scratch` working buffer, same as
    /// before this change.
    pub fn with_spares(
        track: usize,
        start: usize,
        seed: u32,
        max_block: usize,
        spares: Vec<Vec<i16>>,
    ) -> Self {
        let mut p = Self::new(track, start, seed);
        // Enough capacity for every spare handed in plus the one
        // `current` will hold once `chunks` starts closing them out, so
        // `chunks.push` in `write_block` never reallocates within a
        // pass that stays inside its reserve.
        p.chunks.reserve_exact(spares.len() + 1);
        p.spares = spares;
        p.scratch.reserve_exact(max_block.max(XFADE_SAMPLES));
        p
    }

    /// Give back whatever chunk buffers this pass reserved but never
    /// wrote into - called by `Journal::push_pass` so a track's reserve
    /// doesn't shrink every time it records less than its full share.
    /// A plain move, not an allocation.
    pub fn take_unused_spares(&mut self) -> Vec<Vec<i16>> {
        std::mem::take(&mut self.spares)
    }

    /// Samples written so far.
    pub fn len(&self) -> usize {
        self.total_len
    }

    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }

    /// Write one block of already-processed audio to tape, capturing what
    /// it displaces and applying the punch-in fade over the first
    /// `XFADE_SAMPLES` of the pass. Returns samples written.
    pub fn write_block(&mut self, tape: &mut Tape, block: &[f32]) -> usize {
        let pos = self.start + self.total_len;
        let room = tape.len_samples().saturating_sub(pos);
        let n = block.len().min(room);
        if n == 0 {
            return 0;
        }

        let mut done = 0usize;
        while done < n {
            // Roll to a fresh chunk when the current one is full (or
            // this is the very first write, where `current` starts
            // empty with zero capacity - the same check covers both).
            if self.current.len() == self.current.capacity() {
                if !self.current.is_empty() {
                    let filled = std::mem::take(&mut self.current);
                    self.chunks.push(filled);
                }
                match self.spares.pop() {
                    Some(buf) => self.current = buf,
                    None => {
                        self.allocated_on_thread = true;
                        self.current = Vec::with_capacity(CHUNK_SAMPLES);
                    }
                }
            }

            let space = self.current.capacity() - self.current.len();
            // `take` never exceeds the caller's block size (bounded by
            // MAX_BLOCK, far smaller than CHUNK_SAMPLES), so the
            // `scratch.reserve` below is always a no-op.
            let take = (n - done).min(space);

            let old_start = self.current.len();
            self.current.resize(old_start + take, 0);
            let abs_pos = pos + done;
            tape.read_raw(
                self.track,
                abs_pos,
                &mut self.current[old_start..old_start + take],
            );

            self.scratch.clear();
            self.scratch.reserve(take);
            for i in 0..take {
                let pass_idx = self.total_len + done + i;
                let old_sample = self.current[old_start + i];
                let new = self.dither.quantize(block[done + i]);
                let value = if pass_idx < XFADE_SAMPLES {
                    let t = (pass_idx + 1) as f32 / XFADE_SAMPLES as f32;
                    (f32::from(new) * t + f32::from(old_sample) * (1.0 - t)).round() as i16
                } else {
                    new
                };
                self.scratch.push(value);
                if self.tail.len() == XFADE_SAMPLES {
                    self.tail.pop_front();
                }
                self.tail.push_back(old_sample);
            }
            tape.write_raw(self.track, abs_pos, &self.scratch);
            done += take;
        }
        self.total_len += done;
        done
    }

    /// Close the pass, applying the punch-out fade to the tail of the
    /// written region so it blends back into the displaced content.
    /// Skipped when the pass ran to the very end of the tape, where there
    /// is nothing to blend back into.
    pub fn finish(&mut self, tape: &mut Tape) {
        let len = self.total_len;
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
        // `self.tail` always holds at least the last `fade` displaced
        // samples (fade <= XFADE_SAMPLES, tail's own cap), regardless of
        // which chunk they originally landed in.
        let tail_len = self.tail.len();
        for i in 0..fade {
            let t = (i + 1) as f32 / fade as f32;
            let new = f32::from(self.scratch[i]);
            let old = f32::from(self.tail[tail_len - fade + i]);
            self.scratch[i] = (new * (1.0 - t) + old * t).round() as i16;
        }
        tape.write_raw(self.track, tail_start, &self.scratch[..fade]);
    }

    /// Consume the pass, returning its displaced-audio chunks in order
    /// (closed ones followed by whatever `current` holds, full or not) -
    /// what `Journal::push_pass` persists. Concatenation happens only as
    /// each chunk's bytes are written to disk (`Journal::write_payload`,
    /// always off the realtime thread), never in memory here.
    pub fn into_chunks(mut self) -> Vec<Vec<i16>> {
        self.chunks.push(self.current);
        self.chunks
    }

    /// Restore the displaced audio, returning what was on tape in its
    /// place so the operation can be redone. Not on any realtime path
    /// (only `Journal::undo`/`redo`, always off-thread, do this for
    /// real) - used directly only by this module's own test below.
    pub fn undo(&self, tape: &mut Tape) -> Vec<i16> {
        let mut current = vec![0i16; self.total_len];
        tape.read_raw(self.track, self.start, &mut current);
        let mut offset = 0;
        for chunk in &self.chunks {
            tape.write_raw(self.track, self.start + offset, chunk);
            offset += chunk.len();
        }
        tape.write_raw(self.track, self.start + offset, &self.current);
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
        assert_eq!(pass1.len(), 5000);
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

    #[test]
    fn a_pass_spanning_several_chunks_round_trips_through_undo() {
        // Longer than one CHUNK_SAMPLES worth, so this exercises the
        // chunk-rollover path in write_block, using no spares (the
        // plain `new()` path always falls back to fresh chunks).
        let len = CHUNK_SAMPLES * 2 + 777;
        let mut tape = Tape::new(len + 1000);
        let signal = sine(440.0, -6.0, len);
        let mut p = RecordPass::new(0, 500, 5);
        assert_eq!(p.write_block(&mut tape, &signal), len);
        assert!(p.allocated_on_thread, "new() has no spares, must fall back");
        p.finish(&mut tape);

        let mut before = vec![0i16; len];
        tape.read_raw(0, 500, &mut before);
        let after_write = before.clone();

        let restored = p.undo(&mut tape);
        assert_eq!(
            restored, after_write,
            "redo payload matches what was on tape"
        );
        let mut now = vec![0i16; len];
        tape.read_raw(0, 500, &mut now);
        assert_eq!(now, vec![0i16; len], "undo restores the original silence");
    }

    #[test]
    fn spares_avoid_the_fallback_allocation_within_their_budget() {
        let len = CHUNK_SAMPLES + 100;
        let mut tape = Tape::new(len + 100);
        let spares = vec![
            Vec::with_capacity(CHUNK_SAMPLES),
            Vec::with_capacity(CHUNK_SAMPLES),
        ];
        let mut p = RecordPass::with_spares(0, 0, 1, 4096, spares);
        p.write_block(&mut tape, &sine(440.0, -6.0, len));
        assert!(
            !p.allocated_on_thread,
            "two spares cover a pass just over one chunk boundary"
        );
    }
}
