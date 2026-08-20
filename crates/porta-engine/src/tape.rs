//! The tape: four fixed-length mono i16 tracks held in RAM, with per-chunk
//! dirty tracking so saves rewrite only what changed.

use crate::NUM_TRACKS;
use porta_dsp::SAMPLE_RATE;

/// Granularity of dirty tracking and on-disk writes: 5 seconds.
pub const CHUNK_SAMPLES: usize = (SAMPLE_RATE as usize) * 5;

/// Default cassette length: 15 minutes.
pub const DEFAULT_TAPE_SAMPLES: usize = (SAMPLE_RATE as usize) * 60 * 15;
/// Hard cap: 30 minutes (REQ-101).
pub const MAX_TAPE_SAMPLES: usize = (SAMPLE_RATE as usize) * 60 * 30;

pub struct Track {
    samples: Vec<i16>,
    dirty: Vec<bool>,
}

impl Track {
    fn new(len_samples: usize) -> Self {
        let chunks = len_samples.div_ceil(CHUNK_SAMPLES);
        Self {
            samples: vec![0; len_samples],
            dirty: vec![false; chunks],
        }
    }
}

pub struct Tape {
    tracks: Vec<Track>,
    len_samples: usize,
}

impl Tape {
    /// Panics if `len_samples` is zero or beyond the 30-minute cap.
    pub fn new(len_samples: usize) -> Self {
        assert!(len_samples > 0, "tape length must be non-zero");
        assert!(
            len_samples <= MAX_TAPE_SAMPLES,
            "tape length beyond 30-minute cap"
        );
        Self {
            tracks: (0..NUM_TRACKS).map(|_| Track::new(len_samples)).collect(),
            len_samples,
        }
    }

    pub fn len_samples(&self) -> usize {
        self.len_samples
    }

    pub fn num_chunks(&self) -> usize {
        self.len_samples.div_ceil(CHUNK_SAMPLES)
    }

    /// Read as f32 in [-1, 1). Reads past the tape end are zero-filled.
    /// Returns the number of real tape samples read.
    pub fn read(&self, track: usize, start: usize, out: &mut [f32]) -> usize {
        let src = &self.tracks[track].samples;
        let end = (start + out.len()).min(self.len_samples);
        let n = end.saturating_sub(start);
        for (dst, &s) in out[..n].iter_mut().zip(&src[start..end]) {
            *dst = f32::from(s) / 32768.0;
        }
        out[n..].fill(0.0);
        n
    }

    /// Raw i16 read, zero-filled past the end. Returns real samples read.
    pub fn read_raw(&self, track: usize, start: usize, out: &mut [i16]) -> usize {
        let src = &self.tracks[track].samples;
        let end = (start + out.len()).min(self.len_samples);
        let n = end.saturating_sub(start);
        out[..n].copy_from_slice(&src[start..end]);
        out[n..].fill(0);
        n
    }

    /// Write already-quantized samples, truncating at the tape end. Marks
    /// touched chunks dirty. Returns the number of samples written.
    pub fn write_raw(&mut self, track: usize, start: usize, data: &[i16]) -> usize {
        if start >= self.len_samples {
            return 0;
        }
        let end = (start + data.len()).min(self.len_samples);
        let n = end - start;
        self.tracks[track].samples[start..end].copy_from_slice(&data[..n]);
        let first_chunk = start / CHUNK_SAMPLES;
        let last_chunk = (end - 1) / CHUNK_SAMPLES;
        for c in first_chunk..=last_chunk {
            self.tracks[track].dirty[c] = true;
        }
        n
    }

    /// Direct chunk access for persistence (chunk may be short at tape end).
    pub fn chunk(&self, track: usize, chunk: usize) -> &[i16] {
        let start = chunk * CHUNK_SAMPLES;
        let end = (start + CHUNK_SAMPLES).min(self.len_samples);
        &self.tracks[track].samples[start..end]
    }

    pub fn dirty_chunks(&self, track: usize) -> Vec<usize> {
        self.tracks[track]
            .dirty
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| d.then_some(i))
            .collect()
    }

    pub fn clear_dirty(&mut self, track: usize) {
        self.tracks[track].dirty.fill(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_raw_and_f32() {
        let mut tape = Tape::new(CHUNK_SAMPLES * 2);
        let data: Vec<i16> = (0..1000).map(|i| (i * 30) as i16).collect();
        assert_eq!(tape.write_raw(1, 100, &data), 1000);

        let mut raw = vec![0i16; 1000];
        assert_eq!(tape.read_raw(1, 100, &mut raw), 1000);
        assert_eq!(raw, data);

        let mut f = vec![0f32; 4];
        tape.read(1, 100, &mut f);
        assert_eq!(f[0], 0.0);
        assert!((f[1] - 30.0 / 32768.0).abs() < 1e-9);
    }

    #[test]
    fn writes_truncate_at_tape_end() {
        let mut tape = Tape::new(1000);
        let data = vec![7i16; 100];
        assert_eq!(tape.write_raw(0, 950, &data), 50);
        assert_eq!(tape.write_raw(0, 1000, &data), 0);
        assert_eq!(tape.write_raw(0, 2000, &data), 0);
    }

    #[test]
    fn reads_zero_fill_past_end() {
        let mut tape = Tape::new(1000);
        tape.write_raw(2, 990, &[100i16; 10]);
        let mut out = vec![1f32; 20];
        assert_eq!(tape.read(2, 990, &mut out), 10);
        assert!(out[..10].iter().all(|&x| x > 0.0));
        assert!(out[10..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn dirty_chunks_track_writes() {
        let mut tape = Tape::new(CHUNK_SAMPLES * 4);
        assert!(tape.dirty_chunks(0).is_empty());

        // A write spanning the chunk 1 / chunk 2 boundary.
        let data = vec![1i16; 100];
        tape.write_raw(0, CHUNK_SAMPLES * 2 - 50, &data);
        assert_eq!(tape.dirty_chunks(0), vec![1, 2]);
        assert!(tape.dirty_chunks(1).is_empty(), "other tracks untouched");

        tape.clear_dirty(0);
        assert!(tape.dirty_chunks(0).is_empty());
    }

    #[test]
    fn chunk_access_handles_short_tail() {
        let tape = Tape::new(CHUNK_SAMPLES + 100);
        assert_eq!(tape.num_chunks(), 2);
        assert_eq!(tape.chunk(0, 0).len(), CHUNK_SAMPLES);
        assert_eq!(tape.chunk(0, 1).len(), 100);
    }

    #[test]
    #[should_panic(expected = "30-minute cap")]
    fn rejects_over_long_tape() {
        Tape::new(MAX_TAPE_SAMPLES + 1);
    }
}
