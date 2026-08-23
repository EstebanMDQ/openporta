//! Bounded undo journal (REQ-502). Each entry is one record pass's
//! displaced audio, spilled to disk so long sessions do not hold every
//! take in RAM. Destructive UX, recoverable underneath: undo and redo
//! buttons only, no history browser (REQ-505).

use crate::record::RecordPass;
use crate::tape::{BusChannel, Tape, CHUNK_SAMPLES};
use crate::NUM_TRACKS;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Default journal caps. Both bound the stack; whichever binds first wins.
pub const DEFAULT_MAX_PASSES: usize = 32;
pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Chunk buffers (`tape::CHUNK_SAMPLES` samples each) the pool starts
/// with per track: ~2 minutes of continuous recording (24 chunks x 5s).
/// See `record.rs`'s module doc comment for the realtime-safety
/// reasoning - this is a fixed, generous reserve, not a live-refilled
/// pool, so a pass beyond its share falls back to an ordinary
/// allocation for the overflow rather than corrupting undo data.
pub const CHUNK_POOL_PER_TRACK: usize = 24;

/// Full-tape buffer pairs held in reserve for bounce passes. Two, so a
/// second bounce with nothing saved in between still finds a free pair
/// - see `Journal.bus_reserve`.
pub const BUS_RESERVE_PAIRS: usize = 2;

/// A payload waiting to be written to disk. The track-vs-bus
/// distinction is an explicit tag, never an overloaded index: the two
/// have genuinely different storage shapes (a track's capture is many
/// small chunks, a bus pass's is one full-tape buffer per channel) and
/// their give-backs go to different reserves.
enum Pending {
    Track {
        id: u64,
        track: usize,
        chunks: Vec<Vec<i16>>,
    },
    Bus {
        id: u64,
        /// Valid samples per channel. The buffers themselves stay full
        /// tape length so they can go straight back to the reserve -
        /// truncating them would hand the next bounce a short buffer.
        len: usize,
        left: Vec<i16>,
        right: Vec<i16>,
    },
}

impl Pending {
    fn id(&self) -> u64 {
        match self {
            Pending::Track { id, .. } | Pending::Bus { id, .. } => *id,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("undo journal io: {0}")]
    Io(#[from] std::io::Error),
    #[error("nothing to {0}")]
    Empty(&'static str),
}

/// What a journaled pass wrote to: one ordinary track, or the stereo
/// bounce bus (both channels, one atomic entry - REQ-502/505).
///
/// This is what code matches on. `Entry`'s own `track`/`right_track`
/// fields are the *serialized* form and nothing should read them
/// directly: for a bus entry they hold indices past the real tracks,
/// which would be a silent out-of-bounds if anyone used them to index
/// `Tape`. Going through `Entry::target()` makes that impossible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassTarget {
    Track(usize),
    Bus,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub id: u64,
    /// Serialized form only - use `target()`. For a track pass this is
    /// the track index; for a bus pass it is a placeholder past the
    /// real tracks, paired with `right_track`.
    pub track: usize,
    pub start: usize,
    /// Samples **per channel**. A stereo entry's total payload is
    /// `len * 2 channels * 2 bytes`; see `bytes()`.
    pub len: usize,
    /// `None` for every ordinary single-channel entry, which is also
    /// what a pre-change-001 journal deserializes to (REQ-503: the
    /// format stays backward compatible by addition only). `Some` marks
    /// a stereo bus entry.
    #[serde(default)]
    pub right_track: Option<usize>,
}

/// Placeholder indices for a bus entry's serialized `track`/
/// `right_track`. Deliberately past `NUM_TRACKS` so that if anything
/// ever does index a track array with one, it panics loudly on the
/// first bounce instead of silently aliasing track 0.
const BUS_LEFT_SLOT: usize = NUM_TRACKS;
const BUS_RIGHT_SLOT: usize = NUM_TRACKS + 1;

impl Entry {
    fn for_track(id: u64, track: usize, start: usize, len: usize) -> Self {
        Self {
            id,
            track,
            start,
            len,
            right_track: None,
        }
    }

    fn for_bus(id: u64, start: usize, len: usize) -> Self {
        Self {
            id,
            track: BUS_LEFT_SLOT,
            start,
            len,
            right_track: Some(BUS_RIGHT_SLOT),
        }
    }

    pub fn target(&self) -> PassTarget {
        if self.right_track.is_some() {
            PassTarget::Bus
        } else {
            PassTarget::Track(self.track)
        }
    }

    /// Resident/on-disk size. A stereo entry carries two channels of
    /// `len` samples, so it counts double against the journal's byte
    /// cap - without this, eviction would let a bounce occupy twice the
    /// budget it was accounted for (REQ-502).
    fn bytes(&self) -> u64 {
        let channels = if self.right_track.is_some() { 2 } else { 1 };
        (self.len * 2 * channels) as u64
    }
}

pub struct Journal {
    dir: PathBuf,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_id: u64,
    max_passes: usize,
    max_bytes: u64,
    /// Payloads queued by `push_pass`/eviction but not yet on disk (see
    /// its doc comment). `flush_pending` writes them out; `save`,
    /// `undo`, and `redo` all call it first, since none of those ever
    /// run on the realtime audio thread.
    pending_writes: Vec<Pending>,
    pending_deletes: Vec<u64>,
    /// Chunk buffers `reclaim_chunks` couldn't return to a track's
    /// reserve (wrong capacity, or the reserve's already at
    /// `CHUNK_POOL_PER_TRACK` - reachable from a pass whose length
    /// exceeded the reserve and fell back to an ordinary allocation for
    /// the overflow, see `CHUNK_POOL_PER_TRACK`'s own doc comment).
    /// `reclaim_chunks` can run on the realtime thread (via
    /// `release_entry_payload`, reachable from `evict`/`push_pass`), so
    /// it must never just drop these in place - that would deallocate
    /// right there. Parking them here instead defers the actual
    /// deallocation to `flush_pending`, which is never called from the
    /// realtime thread (found in review: an earlier version of
    /// `reclaim_chunks` dropped overflow chunks immediately, a real
    /// REQ-902 gap this closes).
    pending_frees: Vec<Vec<i16>>,
    /// One pre-reserved reserve of chunk buffers per track (see
    /// `CHUNK_POOL_PER_TRACK`) - `take_spares` hands a track's whole
    /// reserve over in one move (genuinely zero-allocation: it takes
    /// ownership of an already-allocated `Vec`, leaving an empty one
    /// behind, rather than constructing a new container). `push_pass`
    /// gives back whatever a closed pass didn't use immediately (also a
    /// move); `flush_pending` gives back what it did use, cleared, once
    /// written to disk.
    chunk_pool: [Vec<Vec<i16>>; NUM_TRACKS],
    /// Pre-allocated full-tape buffer pairs for bounce passes, taken
    /// and given back by `mem::take`-style moves exactly like
    /// `chunk_pool` (REQ-902 - a bounce engages from a non-blocking
    /// command, so nothing here may allocate).
    ///
    /// **Double**-buffered, not single, and that is the whole point: a
    /// closed bus pass's buffer moves into `pending_writes` and does
    /// not come back until the next flush (Save/Undo/Redo - Stop
    /// deliberately does not flush). Bounce twice with nothing saved in
    /// between - which is this feature's own motivation, not an edge
    /// case - and a single buffer would already be gone. Pass 1 takes
    /// pair A, pass 2 takes pair B. A third in the same circumstance
    /// falls back to allocating, counted and documented rather than
    /// silently wrong.
    bus_reserve: Vec<(Vec<i16>, Vec<i16>)>,
}

impl Journal {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, UndoError> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 0,
            max_passes: DEFAULT_MAX_PASSES,
            max_bytes: DEFAULT_MAX_BYTES,
            pending_writes: Vec::new(),
            pending_deletes: Vec::new(),
            pending_frees: Vec::new(),
            chunk_pool: std::array::from_fn(|_| {
                (0..CHUNK_POOL_PER_TRACK)
                    .map(|_| Vec::with_capacity(CHUNK_SAMPLES))
                    .collect()
            }),
            // Empty until `with_bus_reserve` sizes it: only the caller
            // knows the cassette's tape length.
            bus_reserve: Vec::with_capacity(BUS_RESERVE_PAIRS),
        })
    }

    /// Pre-allocate the bounce reserve for a `len_samples` cassette,
    /// off the realtime thread, at open/create - the same moment `Tape`
    /// itself is allocated. Without this a bounce falls back to
    /// allocating its buffers on the audio thread every time.
    pub fn with_bus_reserve(mut self, len_samples: usize) -> Self {
        self.bus_reserve.clear();
        for _ in 0..BUS_RESERVE_PAIRS {
            self.bus_reserve
                .push((vec![0i16; len_samples], vec![0i16; len_samples]));
        }
        self
    }

    /// Hand a bounce pass a full-tape buffer pair. `None` means the
    /// reserve is out (both pairs still pending a flush) and the caller
    /// must allocate its own - the documented, counted fallback, not a
    /// silent failure. Realtime-safe: a `pop`, no allocation.
    pub fn take_bus_buffers(&mut self) -> Option<(Vec<i16>, Vec<i16>)> {
        self.bus_reserve.pop()
    }

    /// Give a pair back. Deliberately **not** cleared, unlike
    /// `reclaim_chunks`: a track's chunks are filled by `push`, so they
    /// have to come back empty, but a bus buffer is written by index
    /// and must keep its full length - clearing would force the next
    /// pass to re-`resize` (a 170MB memset on a 30-minute cassette,
    /// on the audio thread). Stale samples past the new pass's own
    /// length are never read: the entry's `len` bounds every read.
    ///
    /// Over the cap the buffers are parked in `pending_frees` rather
    /// than dropped in place, for the same reason `reclaim_chunks`
    /// parks its overflow: this can run on the realtime thread.
    fn reclaim_bus_buffers(&mut self, left: Vec<i16>, right: Vec<i16>) {
        if self.bus_reserve.len() < BUS_RESERVE_PAIRS {
            self.bus_reserve.push((left, right));
        } else {
            self.pending_frees.push(left);
            self.pending_frees.push(right);
        }
    }

    /// Hand `track`'s whole chunk-buffer reserve over to a new pass -
    /// realtime-safe: `mem::take` moves the already-allocated `Vec` out
    /// wholesale (no allocation, no partial-take container to build),
    /// leaving an empty reserve behind until this pass closes and
    /// returns what it didn't use (`push_pass`) or `flush_pending`
    /// returns what it did. Empty if the reserve is already out (e.g.
    /// re-arming and recording again before the previous pass on this
    /// track has closed - can't happen through `Engine::record()`
    /// today, but `take_spares` itself doesn't assume it).
    pub fn take_spares(&mut self, track: usize) -> Vec<Vec<i16>> {
        std::mem::take(&mut self.chunk_pool[track])
    }

    pub fn with_caps(mut self, max_passes: usize, max_bytes: u64) -> Self {
        self.max_passes = max_passes;
        self.max_bytes = max_bytes;
        self
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    fn path_for(&self, id: u64) -> PathBuf {
        self.dir.join(format!("pass-{id:04}.bin"))
    }

    fn write_payload(&self, id: u64, data: &[i16]) -> Result<(), UndoError> {
        let mut f = fs::File::create(self.path_for(id))?;
        let mut bytes = Vec::with_capacity(data.len() * 2);
        for &s in data {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        f.write_all(&bytes)?;
        Ok(())
    }

    /// Same as `write_payload`, but for a pass's chunked capture
    /// (`RecordPass::into_chunks`) - writes each chunk's bytes in order
    /// without concatenating them into one buffer first. Both produce
    /// byte-identical files; `read_payload` doesn't need to know or
    /// care which wrote a given one.
    fn write_payload_chunks(&self, id: u64, chunks: &[Vec<i16>]) -> Result<(), UndoError> {
        let mut f = fs::File::create(self.path_for(id))?;
        for chunk in chunks {
            let mut bytes = Vec::with_capacity(chunk.len() * 2);
            for &s in chunk {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
            f.write_all(&bytes)?;
        }
        Ok(())
    }

    /// A stereo payload: left channel's bytes then right's, in one file
    /// per entry id (same `path_for` model as every other payload -
    /// only the caller needs to know an entry is stereo).
    fn write_payload_stereo(&self, id: u64, left: &[i16], right: &[i16]) -> Result<(), UndoError> {
        let mut f = fs::File::create(self.path_for(id))?;
        for channel in [left, right] {
            let mut bytes = Vec::with_capacity(channel.len() * 2);
            for &s in channel {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
            f.write_all(&bytes)?;
        }
        Ok(())
    }

    fn read_payload(&self, entry: &Entry) -> Result<Vec<i16>, UndoError> {
        let mut f = fs::File::open(self.path_for(entry.id))?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect())
    }

    /// Record a completed pass. Does no I/O and cannot fail - the
    /// payload is kept in memory and its disk write deferred to
    /// `flush_pending`, so this is safe to call from the realtime audio
    /// thread (REQ-902). Also returns whatever chunk buffers the pass
    /// reserved but never wrote into - a plain move back into
    /// `chunk_pool`, not an allocation - so a track's reserve doesn't
    /// shrink every time it records less than its full share; without
    /// this, ordinary use drains the pool to nothing within a handful of
    /// takes (found in review, not by design). Clears the redo stack, as
    /// any new take invalidates the branch that was undone; those
    /// payloads are dropped from `pending_writes` if never flushed, or
    /// queued for deletion if they already made it to disk.
    pub fn push_pass(&mut self, mut pass: RecordPass) {
        // Indexed, not `drain`, so `release_entry_payload` (which needs
        // its own `&mut self`) isn't fighting an active borrow of
        // `self.redo` - and no allocation for a temporary id/track list.
        self.invalidate_redo();
        let track = pass.track;
        // `chunk_pool[track]` is empty when nothing above just returned
        // chunks to it (`take_spares` emptied it when this pass opened,
        // and only one pass per track can be open at a time) - `extend`
        // rather than a plain assignment so a same-track redo
        // invalidation just above doesn't get overwritten.
        self.chunk_pool[track].extend(pass.take_unused_spares());
        if pass.is_empty() {
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        let start = pass.start;
        let len = pass.len();
        // `into_chunks` just moves already-allocated chunk buffers into
        // `pending_writes` - no allocation of the sample data itself.
        self.pending_writes.push(Pending::Track {
            id,
            track,
            chunks: pass.into_chunks(),
        });
        self.undo.push(Entry::for_track(id, track, start, len));
        self.evict();
    }

    /// Record a completed bounce pass: both channels of displaced bus
    /// content as ONE entry, so a single undo reverts the whole thing
    /// and no intermediate state has one channel reverted and the other
    /// not (REQ-505). `left`/`right` must be the same length - the
    /// pass writes both channels in lockstep - and are moved, not
    /// copied, so this allocates nothing (REQ-902).
    ///
    /// Buffers come back to the reserve when the payload is written
    /// (`flush_pending`) or dropped (`release_entry_payload`), routed by
    /// the `Pending::Bus` tag rather than inferred from an index.
    pub fn push_bus_pass(&mut self, start: usize, len: usize, left: Vec<i16>, right: Vec<i16>) {
        assert!(
            left.len() >= len && right.len() >= len,
            "a bounce pass writes both channels in lockstep"
        );
        self.invalidate_redo();
        if len == 0 {
            self.reclaim_bus_buffers(left, right);
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.pending_writes.push(Pending::Bus {
            id,
            len,
            left,
            right,
        });
        self.undo.push(Entry::for_bus(id, start, len));
        self.evict();
    }

    /// Drop the redo branch - any new take invalidates it. Indexed, not
    /// `drain`, so `release_entry_payload` (which needs its own
    /// `&mut self`) isn't fighting an active borrow of `self.redo`, and
    /// no temporary list is allocated.
    fn invalidate_redo(&mut self) {
        for i in 0..self.redo.len() {
            let (id, target) = (self.redo[i].id, self.redo[i].target());
            self.release_entry_payload(id, target);
        }
        self.redo.clear();
    }

    /// Oldest-first eviction once either cap is exceeded. Deferred like
    /// `push_pass` - no I/O here either.
    fn evict(&mut self) {
        let mut total: u64 = self.undo.iter().map(Entry::bytes).sum();
        while self.undo.len() > self.max_passes || (total > self.max_bytes && self.undo.len() > 1) {
            let e = self.undo.remove(0);
            total -= e.bytes();
            self.release_entry_payload(e.id, e.target());
        }
    }

    /// A payload stops being reachable - either evicted or invalidated
    /// by a new take overwriting the redo branch. If it's still
    /// pending (never flushed), its chunk buffers go straight back to
    /// `track`'s reserve, the same plain move `flush_pending` uses once
    /// a payload's been written - not dropped, which would both
    /// deallocate on the realtime thread and permanently shrink the
    /// reserve (found in review: an earlier version of this function
    /// did exactly that via `Vec::retain`, silently undoing the give-
    /// back `push_pass`/`flush_pending` otherwise provide). If it
    /// already made it to disk, its file is queued for deletion instead
    /// - real I/O, deferred to `flush_pending`, off the realtime thread.
    fn release_entry_payload(&mut self, id: u64, target: PassTarget) {
        let Some(pos) = self.pending_writes.iter().position(|p| p.id() == id) else {
            self.pending_deletes.push(id);
            return;
        };
        // Routed by the payload's own tag, not by `target` - they
        // always agree, and trusting the tag means a mismatch can never
        // send a bus buffer into a track's chunk reserve.
        debug_assert_eq!(
            matches!(self.pending_writes[pos], Pending::Bus { .. }),
            target == PassTarget::Bus,
            "entry target and payload tag disagree"
        );
        match self.pending_writes.remove(pos) {
            Pending::Track { track, chunks, .. } => self.reclaim_chunks(track, chunks),
            Pending::Bus { left, right, .. } => self.reclaim_bus_buffers(left, right),
        }
    }

    /// Clear and return each chunk to `track`'s reserve, up to its
    /// target size - shared by `flush_pending` (chunks whose bytes just
    /// made it to disk) and `release_entry_payload` (chunks that never
    /// needed to; either way, the reserve gets them back the same way).
    /// Whatever doesn't fit back (see `pending_frees`'s doc comment)
    /// is parked there instead of dropped in place - this function must
    /// stay realtime-safe itself, since one of its callers is.
    fn reclaim_chunks(&mut self, track: usize, chunks: Vec<Vec<i16>>) {
        for mut chunk in chunks {
            chunk.clear();
            if chunk.capacity() >= CHUNK_SAMPLES
                && self.chunk_pool[track].len() < CHUNK_POOL_PER_TRACK
            {
                self.chunk_pool[track].push(chunk);
            } else {
                self.pending_frees.push(chunk);
            }
        }
    }

    /// Write out everything queued by `push_pass`/eviction since the
    /// last flush. Real disk I/O - never call this from the realtime
    /// callback. `save`, `undo`, and `redo` call it first so a payload
    /// is always readable regardless of whether it happened to be
    /// flushed yet.
    pub fn flush_pending(&mut self) -> Result<(), UndoError> {
        for id in std::mem::take(&mut self.pending_deletes) {
            let _ = fs::remove_file(self.path_for(id));
        }
        for pending in std::mem::take(&mut self.pending_writes) {
            match pending {
                Pending::Track { id, track, chunks } => {
                    self.write_payload_chunks(id, &chunks)?;
                    // Reclaim each chunk's already-reserved capacity back
                    // into its track's reserve instead of dropping it -
                    // alongside `release_entry_payload`'s immediate return
                    // of never-flushed chunks, this is how `chunk_pool`
                    // stays replenished across a whole session without
                    // ever touching the realtime thread. See record.rs's
                    // module doc.
                    self.reclaim_chunks(track, chunks);
                }
                Pending::Bus {
                    id,
                    len,
                    left,
                    right,
                } => {
                    // One file per entry: the left channel's bytes, then
                    // the right's, back to back. `Entry::len` is
                    // per-channel, so the reader knows where to split.
                    self.write_payload_stereo(id, &left[..len], &right[..len])?;
                    self.reclaim_bus_buffers(left, right);
                }
            }
        }
        // Whatever `reclaim_chunks` couldn't return to a reserve, on
        // this call or an earlier realtime-thread one, actually drops
        // here - the one place that's always off the realtime thread.
        self.pending_frees.clear();
        Ok(())
    }

    pub fn undo(&mut self, tape: &mut Tape) -> Result<(), UndoError> {
        // Never called from the realtime thread (Undo is a blocking
        // command), so flushing here is safe and keeps the guarantee
        // that a payload is always readable regardless of whether
        // push_pass happened to flush it yet.
        self.flush_pending()?;
        let entry = self.undo.pop().ok_or(UndoError::Empty("undo"))?;
        self.swap_with_tape(&entry, tape)?;
        self.redo.push(entry);
        Ok(())
    }

    pub fn redo(&mut self, tape: &mut Tape) -> Result<(), UndoError> {
        self.flush_pending()?;
        let entry = self.redo.pop().ok_or(UndoError::Empty("redo"))?;
        self.swap_with_tape(&entry, tape)?;
        self.undo.push(entry);
        Ok(())
    }

    /// Put an entry's payload back on tape and keep what was there as
    /// the payload for the opposite direction - the one operation undo
    /// and redo both are, in either direction.
    ///
    /// Ordered so a stereo entry is atomic against failure (REQ-505):
    /// everything fallible (reading the payload) happens first, then
    /// **both** channels are written with no fallible step between
    /// them, so there is no reachable state with one channel reverted
    /// and the other not. `Tape` writes cannot fail; only the trailing
    /// payload write can, and by then the tape is already consistent.
    fn swap_with_tape(&mut self, entry: &Entry, tape: &mut Tape) -> Result<(), UndoError> {
        let payload = self.read_payload(entry)?;
        match entry.target() {
            PassTarget::Track(track) => {
                let mut current = vec![0i16; entry.len];
                tape.read_raw(track, entry.start, &mut current);
                tape.write_raw(track, entry.start, &payload);
                self.write_payload(entry.id, &current)
            }
            PassTarget::Bus => {
                if payload.len() < entry.len * 2 {
                    return Err(UndoError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "stereo undo payload is short",
                    )));
                }
                let (left, right) = payload.split_at(entry.len);
                let mut cur_l = vec![0i16; entry.len];
                let mut cur_r = vec![0i16; entry.len];
                tape.read_bus_raw(BusChannel::Left, entry.start, &mut cur_l);
                tape.read_bus_raw(BusChannel::Right, entry.start, &mut cur_r);
                // Both writes, back to back, nothing fallible between.
                tape.write_bus_raw(BusChannel::Left, entry.start, left);
                tape.write_bus_raw(BusChannel::Right, entry.start, right);
                self.write_payload_stereo(entry.id, &cur_l, &cur_r)
            }
        }
    }

    /// Persist stack metadata so undo survives restart (REQ-503). Also
    /// flushes any pending payloads first, so "saved" really means
    /// everything is on disk.
    pub fn save(&mut self) -> Result<(), UndoError> {
        self.flush_pending()?;
        let state = JournalState {
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            next_id: self.next_id,
        };
        let json = serde_json::to_string_pretty(&state).expect("serialize journal");
        fs::write(self.dir.join("journal.json"), json)?;
        Ok(())
    }

    pub fn load(dir: impl AsRef<Path>) -> Result<Self, UndoError> {
        let dir = dir.as_ref().to_path_buf();
        let mut j = Journal::new(&dir)?;
        let path = dir.join("journal.json");
        if path.exists() {
            let text = fs::read_to_string(path)?;
            if let Ok(state) = serde_json::from_str::<JournalState>(&text) {
                j.undo = state.undo;
                j.redo = state.redo;
                j.next_id = state.next_id;
            }
        }
        Ok(j)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JournalState {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::RecordPass;
    use porta_testkit::signal::sine;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("porta-undo-{name}"));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn record(tape: &mut Tape, track: usize, start: usize, freq: f32, len: usize) -> RecordPass {
        let mut p = RecordPass::new(track, start, (freq as u32).max(1));
        p.write_block(tape, &sine(freq, -6.0, len));
        p.finish(tape);
        p
    }

    fn snapshot(tape: &Tape, track: usize, len: usize) -> Vec<i16> {
        let mut v = vec![0i16; len];
        tape.read_raw(track, 0, &mut v);
        v
    }

    #[test]
    fn undo_and_redo_roundtrip() {
        let dir = TempDir::new("roundtrip");
        let mut tape = Tape::new(48_000);
        let mut j = Journal::new(&dir.0).unwrap();

        let p0 = record(&mut tape, 0, 0, 220.0, 30_000);
        j.push_pass(p0);
        let after_first = snapshot(&tape, 0, 48_000);

        let p1 = record(&mut tape, 0, 5000, 1100.0, 10_000);
        j.push_pass(p1);
        let after_second = snapshot(&tape, 0, 48_000);
        assert_ne!(after_first, after_second);

        j.undo(&mut tape).unwrap();
        assert_eq!(snapshot(&tape, 0, 48_000), after_first, "undo byte-exact");

        j.redo(&mut tape).unwrap();
        assert_eq!(snapshot(&tape, 0, 48_000), after_second, "redo byte-exact");

        j.undo(&mut tape).unwrap();
        j.undo(&mut tape).unwrap();
        assert_eq!(
            snapshot(&tape, 0, 48_000),
            vec![0i16; 48_000],
            "back to blank"
        );
        assert!(!j.can_undo());
        assert!(matches!(j.undo(&mut tape), Err(UndoError::Empty(_))));
    }

    #[test]
    fn new_take_clears_redo() {
        let dir = TempDir::new("clears-redo");
        let mut tape = Tape::new(48_000);
        let mut j = Journal::new(&dir.0).unwrap();

        let p0 = record(&mut tape, 1, 0, 220.0, 10_000);
        j.push_pass(p0);
        j.undo(&mut tape).unwrap();
        assert!(j.can_redo());

        let p1 = record(&mut tape, 1, 0, 660.0, 10_000);
        j.push_pass(p1);
        assert!(!j.can_redo(), "new take must invalidate the redo branch");
    }

    #[test]
    fn evicting_a_still_pending_entry_returns_its_chunks_to_the_pool() {
        // Regression for a real bug found in review: evict() used to
        // drop a still-unflushed entry's chunk buffers via
        // Vec::retain (deallocating them, on the realtime thread, and
        // permanently shrinking the reserve) instead of returning them
        // the same way flush_pending does once they're written.
        let dir = TempDir::new("evict-pending-give-back");
        let mut tape = Tape::new(CHUNK_SAMPLES * 3);
        let mut j = Journal::new(&dir.0).unwrap().with_caps(1, u64::MAX);

        let spares = j.take_spares(0);
        let reserve_len = spares.len();
        assert!(
            reserve_len > 1,
            "need spares to prove some come back unused"
        );
        let mut p0 = RecordPass::with_spares(0, 0, 1, 4096, spares);
        // Short write: one chunk consumed, the rest of the reserve
        // stays unused - so give-back has to recover both a used chunk
        // (via eviction, below) and unused ones (via push_pass itself).
        p0.write_block(&mut tape, &sine(220.0, -6.0, 100));
        p0.finish(&mut tape);
        j.push_pass(p0); // pending: id 0, never flushed

        // A second, non-empty pass on the same track: push_pass reaches
        // evict() (an empty pass would return early first), and the
        // cap of 1 forces id 0 out while its payload is still pending.
        let mut p1 = RecordPass::new(0, 200, 2);
        p1.write_block(&mut tape, &sine(440.0, -6.0, 100));
        p1.finish(&mut tape);
        j.push_pass(p1);

        assert_eq!(j.depth(), 1, "cap enforced");
        let recovered = j.take_spares(0);
        assert_eq!(
            recovered.len(),
            reserve_len,
            "the evicted entry's chunk must come back, not be dropped"
        );
    }

    fn bus_snapshot(tape: &Tape, len: usize) -> (Vec<i16>, Vec<i16>) {
        let mut l = vec![0i16; len];
        let mut r = vec![0i16; len];
        tape.read_bus_raw(BusChannel::Left, 0, &mut l);
        tape.read_bus_raw(BusChannel::Right, 0, &mut r);
        (l, r)
    }

    #[test]
    fn stereo_entry_undo_and_redo_restore_both_channels() {
        let dir = TempDir::new("stereo-undo");
        let mut tape = Tape::new(48_000);
        let mut j = Journal::new(&dir.0).unwrap().with_bus_reserve(48_000);

        // Pretend a bounce ran: the bus had content, the pass displaced
        // it, and the displaced audio is what gets journaled.
        let before_l: Vec<i16> = (0..1000).map(|i| (i * 13) as i16).collect();
        let before_r: Vec<i16> = (0..1000).map(|i| -((i * 5) as i16)).collect();
        tape.write_bus_raw(BusChannel::Left, 0, &before_l);
        tape.write_bus_raw(BusChannel::Right, 0, &before_r);
        let original = bus_snapshot(&tape, 1000);

        let (mut buf_l, mut buf_r) = j.take_bus_buffers().expect("reserve pair");
        buf_l[..1000].copy_from_slice(&before_l);
        buf_r[..1000].copy_from_slice(&before_r);

        // The new printed content replaces it on tape.
        tape.write_bus_raw(BusChannel::Left, 0, &vec![7i16; 1000]);
        tape.write_bus_raw(BusChannel::Right, 0, &vec![9i16; 1000]);
        let after_bounce = bus_snapshot(&tape, 1000);
        j.push_bus_pass(0, 1000, buf_l, buf_r);

        assert_eq!(j.depth(), 1, "one atomic entry, not two");

        j.undo(&mut tape).unwrap();
        assert_eq!(
            bus_snapshot(&tape, 1000),
            original,
            "undo must restore BOTH channels"
        );

        j.redo(&mut tape).unwrap();
        assert_eq!(
            bus_snapshot(&tape, 1000),
            after_bounce,
            "redo must restore both channels"
        );

        // And an ordinary track is untouched throughout (REQ-306).
        assert_eq!(snapshot(&tape, 0, 1000), vec![0i16; 1000]);
    }

    #[test]
    fn a_stereo_entry_counts_double_against_the_byte_cap() {
        // Two channels of `len` samples, so eviction must charge it 2x
        // or a bounce would quietly occupy twice its budget.
        let mono = Entry::for_track(0, 0, 0, 1000);
        let stereo = Entry::for_bus(1, 0, 1000);
        assert_eq!(mono.bytes(), 2000);
        assert_eq!(stereo.bytes(), 4000);
        assert_eq!(stereo.target(), PassTarget::Bus);
        assert_eq!(mono.target(), PassTarget::Track(0));
    }

    #[test]
    fn a_pre_change_journal_still_loads() {
        // REQ-503: the old on-disk shape has no `right_track` at all.
        // It must deserialize as an ordinary single-channel entry.
        let dir = TempDir::new("old-journal");
        let old = r#"{"undo":[{"id":0,"track":2,"start":10,"len":500}],"redo":[],"next_id":1}"#;
        fs::write(dir.0.join("journal.json"), old).unwrap();
        let j = Journal::load(&dir.0).unwrap();
        assert_eq!(j.depth(), 1);
        let entry = &j.undo[0];
        assert_eq!(entry.right_track, None);
        assert_eq!(entry.target(), PassTarget::Track(2));
        assert_eq!(entry.bytes(), 1000, "still counted as one channel");
    }

    #[test]
    fn the_bus_reserve_hands_out_two_pairs_then_falls_back() {
        let dir = TempDir::new("bus-reserve");
        let mut tape = Tape::new(4000);
        let mut j = Journal::new(&dir.0).unwrap().with_bus_reserve(4000);

        // Two bounces back to back with nothing saved in between - the
        // exact case double-buffering exists for.
        let a = j.take_bus_buffers().expect("first pair");
        assert_eq!(a.0.len(), 4000, "reserve buffers are full tape length");
        let b = j.take_bus_buffers().expect("second pair, with A still out");
        assert!(
            j.take_bus_buffers().is_none(),
            "a third take must report the reserve is out rather than \
             silently handing back a buffer already in use"
        );

        // Both pending payloads land, then a flush gives the pairs back.
        j.push_bus_pass(0, 100, a.0, a.1);
        j.push_bus_pass(1000, 100, b.0, b.1);
        j.flush_pending().unwrap();
        assert!(j.take_bus_buffers().is_some(), "flush returns pair");
        assert!(j.take_bus_buffers().is_some(), "flush returns both pairs");

        // Give-back also works without a flush, via eviction.
        let c = j.take_bus_buffers();
        assert!(c.is_none(), "both out again");
        let _ = &mut tape;
    }

    #[test]
    fn evicting_a_pending_bus_entry_returns_its_buffers_not_a_track_reserve() {
        // The give-back has to be routed by the payload's own tag: a
        // full-tape bus buffer must never end up in a track's chunk
        // reserve (nor be dropped on the realtime thread).
        let dir = TempDir::new("bus-evict");
        let mut j = Journal::new(&dir.0)
            .unwrap()
            .with_bus_reserve(4000)
            .with_caps(1, u64::MAX);

        let (l, r) = j.take_bus_buffers().unwrap();
        j.push_bus_pass(0, 100, l, r); // pending, id 0
        let (l2, r2) = j.take_bus_buffers().unwrap();
        j.push_bus_pass(0, 100, l2, r2); // forces eviction of id 0 while pending

        assert_eq!(j.depth(), 1, "cap enforced");
        assert!(
            j.take_bus_buffers().is_some(),
            "the evicted pending entry's buffers must come back to the \
             bus reserve, not be dropped or misrouted"
        );
        for t in 0..NUM_TRACKS {
            assert_eq!(
                j.chunk_pool[t].len(),
                CHUNK_POOL_PER_TRACK,
                "track {t}'s chunk reserve must be untouched by bus traffic"
            );
        }
    }

    #[test]
    fn reclaiming_more_chunks_than_the_reserve_holds_defers_the_extra_frees() {
        // Regression for a real bug an eighth review found in the fix
        // above: reclaim_chunks dropped whatever didn't fit back into
        // chunk_pool right there, in place - fine when called from
        // flush_pending (already off the realtime thread), a real
        // REQ-902 violation when called from release_entry_payload,
        // reachable from evict()/push_pass on the realtime thread. An
        // overflow entry (more chunks than CHUNK_POOL_PER_TRACK, e.g. a
        // take that ran past the reserve and fell back to ordinary
        // allocation for the rest) evicted while still pending used to
        // deallocate the extra chunks on that thread.
        let dir = TempDir::new("reclaim-overflow");
        let mut j = Journal::new(&dir.0).unwrap();
        // The reserve starts pre-filled (Journal::new) - drain it first
        // so reclaim_chunks below has room to actually refill up to the
        // cap, rather than every incoming chunk overflowing trivially.
        j.take_spares(0);

        let extra = 3;
        let chunks: Vec<Vec<i16>> = (0..CHUNK_POOL_PER_TRACK + extra)
            .map(|_| Vec::with_capacity(CHUNK_SAMPLES))
            .collect();
        j.reclaim_chunks(0, chunks);

        assert_eq!(
            j.chunk_pool[0].len(),
            CHUNK_POOL_PER_TRACK,
            "reserve fills to its cap, not past it"
        );
        assert_eq!(
            j.pending_frees.len(),
            extra,
            "the overflow must be parked, not dropped in place"
        );

        j.flush_pending().unwrap();
        assert!(
            j.pending_frees.is_empty(),
            "flush_pending is the off-thread checkpoint that actually drops them"
        );
    }

    #[test]
    fn eviction_respects_caps() {
        let dir = TempDir::new("eviction");
        let mut tape = Tape::new(48_000);
        let mut j = Journal::new(&dir.0).unwrap().with_caps(3, u64::MAX);
        for i in 0..6 {
            let p = record(&mut tape, 2, i * 1000, 300.0 + i as f32 * 50.0, 2000);
            j.push_pass(p);
        }
        assert_eq!(j.depth(), 3, "oldest passes evicted");

        // Byte cap binds before the pass cap.
        let mut j2 = Journal::new(&dir.0).unwrap().with_caps(100, 8000);
        for i in 0..5 {
            let p = record(&mut tape, 3, i * 1000, 400.0, 2000);
            j2.push_pass(p);
        }
        assert!(
            j2.depth() <= 2,
            "byte cap should bind, depth {}",
            j2.depth()
        );
    }

    #[test]
    fn empty_pass_is_not_journaled() {
        let dir = TempDir::new("empty");
        let mut j = Journal::new(&dir.0).unwrap();
        let p = RecordPass::new(0, 0, 1);
        j.push_pass(p);
        assert_eq!(j.depth(), 0);
    }

    #[test]
    fn journal_survives_reload() {
        let dir = TempDir::new("reload");
        let mut tape = Tape::new(48_000);
        let baseline;
        {
            let mut j = Journal::new(&dir.0).unwrap();
            let p0 = record(&mut tape, 0, 0, 220.0, 20_000);
            j.push_pass(p0);
            baseline = snapshot(&tape, 0, 48_000);
            let p1 = record(&mut tape, 0, 4000, 990.0, 6000);
            j.push_pass(p1);
            j.save().unwrap();
        }
        let mut reloaded = Journal::load(&dir.0).unwrap();
        assert_eq!(reloaded.depth(), 2);
        reloaded.undo(&mut tape).unwrap();
        assert_eq!(snapshot(&tape, 0, 48_000), baseline);
    }
}
