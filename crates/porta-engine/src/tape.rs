//! The tape: four fixed-length mono i16 tracks held in RAM, plus the
//! stereo bounce bus (change 001), with per-chunk dirty tracking so
//! saves rewrite only what changed.
//!
//! The bus is deliberately its own field, not a fifth and sixth element
//! appended to `tracks`: most of this codebase walks tracks by
//! `0..NUM_TRACKS` (the constant, not `tracks.len()`), so appended
//! elements would be written correctly and then silently never read -
//! a far worse failure than a loud one. Track-indexed methods take a
//! `usize` and can only reach tracks; bus methods take a `BusChannel`
//! and can only reach the bus. Neither can address the other, by
//! construction rather than by convention.

use crate::NUM_TRACKS;
use porta_dsp::SAMPLE_RATE;

/// Granularity of dirty tracking and on-disk writes: 5 seconds.
pub const CHUNK_SAMPLES: usize = (SAMPLE_RATE as usize) * 5;

/// Default cassette length: 15 minutes.
pub const DEFAULT_TAPE_SAMPLES: usize = (SAMPLE_RATE as usize) * 60 * 15;
/// Hard cap: 30 minutes (REQ-101).
pub const MAX_TAPE_SAMPLES: usize = (SAMPLE_RATE as usize) * 60 * 30;

/// One channel of storage: samples plus its own dirty bitmap. Used for
/// both an ordinary track and one channel of the bounce bus - the
/// storage mechanics are identical, only the addressing differs.
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

    /// Read as f32 in [-1, 1), zero-filled past `len_samples`. Returns
    /// real samples read.
    fn read(&self, start: usize, out: &mut [f32], len_samples: usize) -> usize {
        let end = (start + out.len()).min(len_samples);
        let n = end.saturating_sub(start);
        for (dst, &s) in out[..n].iter_mut().zip(&self.samples[start..end]) {
            *dst = f32::from(s) / 32768.0;
        }
        out[n..].fill(0.0);
        n
    }

    fn read_raw(&self, start: usize, out: &mut [i16], len_samples: usize) -> usize {
        let end = (start + out.len()).min(len_samples);
        let n = end.saturating_sub(start);
        out[..n].copy_from_slice(&self.samples[start..end]);
        out[n..].fill(0);
        n
    }

    fn write_raw(&mut self, start: usize, data: &[i16], len_samples: usize) -> usize {
        if start >= len_samples {
            return 0;
        }
        let end = (start + data.len()).min(len_samples);
        let n = end - start;
        self.samples[start..end].copy_from_slice(&data[..n]);
        let first_chunk = start / CHUNK_SAMPLES;
        let last_chunk = (end - 1) / CHUNK_SAMPLES;
        for c in first_chunk..=last_chunk {
            self.dirty[c] = true;
        }
        n
    }

    fn chunk(&self, chunk: usize, len_samples: usize) -> &[i16] {
        let start = chunk * CHUNK_SAMPLES;
        let end = (start + CHUNK_SAMPLES).min(len_samples);
        &self.samples[start..end]
    }

    fn dirty_chunks(&self) -> Vec<usize> {
        self.dirty
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| d.then_some(i))
            .collect()
    }

    fn clear_dirty(&mut self) {
        self.dirty.fill(false);
    }
}

/// Which channel of the stereo bounce bus (change 001, REQ-101). An
/// enum, not an index: a track number can never be silently accepted
/// as a bus channel, and no `0..NUM_TRACKS` loop can reach the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusChannel {
    Left,
    Right,
}

impl BusChannel {
    /// Both channels in a fixed order - for persistence and for the
    /// stereo pass, neither of which should hardcode the pair.
    pub const BOTH: [BusChannel; 2] = [BusChannel::Left, BusChannel::Right];

    fn index(self) -> usize {
        match self {
            BusChannel::Left => 0,
            BusChannel::Right => 1,
        }
    }
}

pub struct Tape {
    tracks: Vec<Track>,
    /// The stereo bounce bus: same fixed length as the tracks, its own
    /// dirty tracking, reachable only through the `*_bus*` methods.
    /// See the module doc for why this is a field and not two more
    /// elements of `tracks`.
    bus: [Track; 2],
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
            bus: [Track::new(len_samples), Track::new(len_samples)],
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
        self.tracks[track].read(start, out, self.len_samples)
    }

    /// Raw i16 read, zero-filled past the end. Returns real samples read.
    pub fn read_raw(&self, track: usize, start: usize, out: &mut [i16]) -> usize {
        self.tracks[track].read_raw(start, out, self.len_samples)
    }

    /// Write already-quantized samples, truncating at the tape end. Marks
    /// touched chunks dirty. Returns the number of samples written.
    pub fn write_raw(&mut self, track: usize, start: usize, data: &[i16]) -> usize {
        let len = self.len_samples;
        self.tracks[track].write_raw(start, data, len)
    }

    /// Direct chunk access for persistence (chunk may be short at tape end).
    pub fn chunk(&self, track: usize, chunk: usize) -> &[i16] {
        self.tracks[track].chunk(chunk, self.len_samples)
    }

    pub fn dirty_chunks(&self, track: usize) -> Vec<usize> {
        self.tracks[track].dirty_chunks()
    }

    pub fn clear_dirty(&mut self, track: usize) {
        self.tracks[track].clear_dirty();
    }

    // --- Bounce bus (change 001) -------------------------------------
    // Same storage mechanics as a track, addressed by BusChannel so the
    // two can never be confused. A bus write leaves every track
    // byte-identical and vice versa (REQ-306's symmetry clause).

    /// Read one bus channel as f32, zero-filled past the tape end.
    pub fn read_bus(&self, channel: BusChannel, start: usize, out: &mut [f32]) -> usize {
        self.bus[channel.index()].read(start, out, self.len_samples)
    }

    /// Raw i16 read of one bus channel, zero-filled past the end.
    pub fn read_bus_raw(&self, channel: BusChannel, start: usize, out: &mut [i16]) -> usize {
        self.bus[channel.index()].read_raw(start, out, self.len_samples)
    }

    /// Write already-quantized samples to one bus channel, truncating at
    /// the tape end and marking touched chunks dirty.
    pub fn write_bus_raw(&mut self, channel: BusChannel, start: usize, data: &[i16]) -> usize {
        let len = self.len_samples;
        self.bus[channel.index()].write_raw(start, data, len)
    }

    /// Direct chunk access for persistence (may be short at tape end).
    pub fn bus_chunk(&self, channel: BusChannel, chunk: usize) -> &[i16] {
        self.bus[channel.index()].chunk(chunk, self.len_samples)
    }

    pub fn bus_dirty_chunks(&self, channel: BusChannel) -> Vec<usize> {
        self.bus[channel.index()].dirty_chunks()
    }

    pub fn clear_bus_dirty(&mut self, channel: BusChannel) {
        self.bus[channel.index()].clear_dirty();
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

    #[test]
    fn bus_roundtrips_per_channel_independently() {
        let mut tape = Tape::new(CHUNK_SAMPLES * 2);
        let left: Vec<i16> = (0..500).map(|i| (i * 11) as i16).collect();
        let right: Vec<i16> = (0..500).map(|i| -((i * 7) as i16)).collect();

        assert_eq!(tape.write_bus_raw(BusChannel::Left, 100, &left), 500);
        assert_eq!(tape.write_bus_raw(BusChannel::Right, 100, &right), 500);

        let mut got = vec![0i16; 500];
        tape.read_bus_raw(BusChannel::Left, 100, &mut got);
        assert_eq!(got, left);
        tape.read_bus_raw(BusChannel::Right, 100, &mut got);
        assert_eq!(got, right, "channels must not share storage");

        let mut f = vec![0f32; 2];
        tape.read_bus(BusChannel::Left, 101, &mut f);
        assert!((f[0] - 11.0 / 32768.0).abs() < 1e-9);
    }

    #[test]
    fn bus_writes_truncate_and_reads_zero_fill() {
        let mut tape = Tape::new(1000);
        let data = vec![9i16; 100];
        assert_eq!(tape.write_bus_raw(BusChannel::Left, 950, &data), 50);
        assert_eq!(tape.write_bus_raw(BusChannel::Left, 1000, &data), 0);
        assert_eq!(tape.write_bus_raw(BusChannel::Left, 5000, &data), 0);

        let mut out = vec![1f32; 20];
        assert_eq!(tape.read_bus(BusChannel::Left, 990, &mut out), 10);
        assert!(out[..10].iter().all(|&x| x != 0.0));
        assert!(out[10..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn bus_dirty_tracking_is_per_channel_and_separate_from_tracks() {
        let mut tape = Tape::new(CHUNK_SAMPLES * 4);
        assert!(tape.bus_dirty_chunks(BusChannel::Left).is_empty());

        // A write spanning the chunk 1 / chunk 2 boundary.
        tape.write_bus_raw(BusChannel::Left, CHUNK_SAMPLES * 2 - 50, &[1i16; 100]);
        assert_eq!(tape.bus_dirty_chunks(BusChannel::Left), vec![1, 2]);
        assert!(
            tape.bus_dirty_chunks(BusChannel::Right).is_empty(),
            "the other bus channel must be untouched"
        );
        for t in 0..NUM_TRACKS {
            assert!(
                tape.dirty_chunks(t).is_empty(),
                "a bus write must not dirty track {t}"
            );
        }

        tape.clear_bus_dirty(BusChannel::Left);
        assert!(tape.bus_dirty_chunks(BusChannel::Left).is_empty());
    }

    #[test]
    fn bus_and_tracks_never_touch_each_others_audio() {
        // REQ-306's symmetry clause at the storage level. The stronger
        // structural guarantee - that no track-indexed API can address
        // the bus at all - is compile-time: read/write_raw take a
        // usize track index into `tracks`, the bus methods take a
        // BusChannel, and neither can reach the other's storage.
        let mut tape = Tape::new(CHUNK_SAMPLES);
        for t in 0..NUM_TRACKS {
            tape.write_raw(t, 0, &[(t as i16 + 1) * 100; 200]);
        }
        let snapshot: Vec<Vec<i16>> = (0..NUM_TRACKS)
            .map(|t| {
                let mut v = vec![0i16; 200];
                tape.read_raw(t, 0, &mut v);
                v
            })
            .collect();

        tape.write_bus_raw(BusChannel::Left, 0, &[-1i16; 200]);
        tape.write_bus_raw(BusChannel::Right, 0, &[-2i16; 200]);

        for (t, expected) in snapshot.iter().enumerate() {
            let mut v = vec![0i16; 200];
            tape.read_raw(t, 0, &mut v);
            assert_eq!(&v, expected, "track {t} changed across a bus write");
        }

        // And the reverse direction.
        let mut bus_l = vec![0i16; 200];
        tape.read_bus_raw(BusChannel::Left, 0, &mut bus_l);
        tape.write_raw(0, 0, &[42i16; 200]);
        let mut after = vec![0i16; 200];
        tape.read_bus_raw(BusChannel::Left, 0, &mut after);
        assert_eq!(after, bus_l, "bus changed across a track write");
    }

    #[test]
    fn bus_chunk_access_handles_short_tail() {
        let tape = Tape::new(CHUNK_SAMPLES + 100);
        assert_eq!(tape.bus_chunk(BusChannel::Left, 0).len(), CHUNK_SAMPLES);
        assert_eq!(tape.bus_chunk(BusChannel::Right, 1).len(), 100);
    }
}
