//! Bounded undo journal (REQ-502). Each entry is one record pass's
//! displaced audio, spilled to disk so long sessions do not hold every
//! take in RAM. Destructive UX, recoverable underneath: undo and redo
//! buttons only, no history browser (REQ-505).

use crate::record::RecordPass;
use crate::tape::{Tape, CHUNK_SAMPLES};
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

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("undo journal io: {0}")]
    Io(#[from] std::io::Error),
    #[error("nothing to {0}")]
    Empty(&'static str),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub id: u64,
    pub track: usize,
    pub start: usize,
    pub len: usize,
}

impl Entry {
    fn bytes(&self) -> u64 {
        (self.len * 2) as u64
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
    pending_writes: Vec<(u64, usize, Vec<Vec<i16>>)>,
    pending_deletes: Vec<u64>,
    /// One pre-reserved reserve of chunk buffers per track (see
    /// `CHUNK_POOL_PER_TRACK`) - `take_spares` hands a track's whole
    /// reserve over in one move (genuinely zero-allocation: it takes
    /// ownership of an already-allocated `Vec`, leaving an empty one
    /// behind, rather than constructing a new container). `push_pass`
    /// gives back whatever a closed pass didn't use immediately (also a
    /// move); `flush_pending` gives back what it did use, cleared, once
    /// written to disk.
    chunk_pool: [Vec<Vec<i16>>; NUM_TRACKS],
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
            chunk_pool: std::array::from_fn(|_| {
                (0..CHUNK_POOL_PER_TRACK)
                    .map(|_| Vec::with_capacity(CHUNK_SAMPLES))
                    .collect()
            }),
        })
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
        for i in 0..self.redo.len() {
            let (id, track) = (self.redo[i].id, self.redo[i].track);
            self.release_entry_payload(id, track);
        }
        self.redo.clear();
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
        self.pending_writes.push((id, track, pass.into_chunks()));
        self.undo.push(Entry {
            id,
            track,
            start,
            len,
        });
        self.evict();
    }

    /// Oldest-first eviction once either cap is exceeded. Deferred like
    /// `push_pass` - no I/O here either.
    fn evict(&mut self) {
        let mut total: u64 = self.undo.iter().map(Entry::bytes).sum();
        while self.undo.len() > self.max_passes || (total > self.max_bytes && self.undo.len() > 1) {
            let e = self.undo.remove(0);
            total -= e.bytes();
            self.release_entry_payload(e.id, e.track);
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
    fn release_entry_payload(&mut self, id: u64, track: usize) {
        if let Some(pos) = self.pending_writes.iter().position(|(pid, ..)| *pid == id) {
            let (_, _, chunks) = self.pending_writes.remove(pos);
            self.reclaim_chunks(track, chunks);
        } else {
            self.pending_deletes.push(id);
        }
    }

    /// Clear and return each chunk to `track`'s reserve, up to its
    /// target size - shared by `flush_pending` (chunks whose bytes just
    /// made it to disk) and `release_entry_payload` (chunks that never
    /// needed to; either way, the reserve gets them back the same way).
    fn reclaim_chunks(&mut self, track: usize, chunks: Vec<Vec<i16>>) {
        for mut chunk in chunks {
            chunk.clear();
            if chunk.capacity() >= CHUNK_SAMPLES
                && self.chunk_pool[track].len() < CHUNK_POOL_PER_TRACK
            {
                self.chunk_pool[track].push(chunk);
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
        for (id, track, chunks) in std::mem::take(&mut self.pending_writes) {
            self.write_payload_chunks(id, &chunks)?;
            // Reclaim each chunk's already-reserved capacity back into
            // its track's reserve instead of dropping it - alongside
            // `release_entry_payload`'s immediate return of never-
            // flushed chunks, this is how `chunk_pool` stays
            // replenished across a whole session without ever touching
            // the realtime thread. See record.rs's module doc.
            self.reclaim_chunks(track, chunks);
        }
        Ok(())
    }

    pub fn undo(&mut self, tape: &mut Tape) -> Result<(), UndoError> {
        // Never called from the realtime thread (Undo is a blocking
        // command), so flushing here is safe and keeps the guarantee
        // that a payload is always readable regardless of whether
        // push_pass happened to flush it yet.
        self.flush_pending()?;
        let entry = self.undo.pop().ok_or(UndoError::Empty("undo"))?;
        let payload = self.read_payload(&entry)?;
        let mut current = vec![0i16; entry.len];
        tape.read_raw(entry.track, entry.start, &mut current);
        tape.write_raw(entry.track, entry.start, &payload);
        self.write_payload(entry.id, &current)?;
        self.redo.push(entry);
        Ok(())
    }

    pub fn redo(&mut self, tape: &mut Tape) -> Result<(), UndoError> {
        self.flush_pending()?;
        let entry = self.redo.pop().ok_or(UndoError::Empty("redo"))?;
        let payload = self.read_payload(&entry)?;
        let mut current = vec![0i16; entry.len];
        tape.read_raw(entry.track, entry.start, &mut current);
        tape.write_raw(entry.track, entry.start, &payload);
        self.write_payload(entry.id, &current)?;
        self.undo.push(entry);
        Ok(())
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
