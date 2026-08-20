//! Bounded undo journal (REQ-502). Each entry is one record pass's
//! displaced audio, spilled to disk so long sessions do not hold every
//! take in RAM. Destructive UX, recoverable underneath: undo and redo
//! buttons only, no history browser (REQ-505).

use crate::record::RecordPass;
use crate::tape::Tape;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Default journal caps. Both bound the stack; whichever binds first wins.
pub const DEFAULT_MAX_PASSES: usize = 32;
pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

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
    pub file: String,
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
        })
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

    fn write_payload(&self, id: u64, data: &[i16]) -> Result<String, UndoError> {
        let path = self.path_for(id);
        let mut f = fs::File::create(&path)?;
        let mut bytes = Vec::with_capacity(data.len() * 2);
        for &s in data {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        f.write_all(&bytes)?;
        Ok(path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string())
    }

    fn read_payload(&self, entry: &Entry) -> Result<Vec<i16>, UndoError> {
        let mut f = fs::File::open(self.dir.join(&entry.file))?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect())
    }

    /// Record a completed pass. Clears the redo stack, as any new take
    /// invalidates the branch that was undone.
    pub fn push_pass(&mut self, pass: &RecordPass) -> Result<(), UndoError> {
        for e in self.redo.drain(..) {
            let _ = fs::remove_file(self.dir.join(&e.file));
        }
        if pass.is_empty() {
            return Ok(());
        }
        let id = self.next_id;
        self.next_id += 1;
        let file = self.write_payload(id, &pass.displaced)?;
        self.undo.push(Entry {
            id,
            track: pass.track,
            start: pass.start,
            len: pass.displaced.len(),
            file,
        });
        self.evict()?;
        Ok(())
    }

    /// Oldest-first eviction once either cap is exceeded.
    fn evict(&mut self) -> Result<(), UndoError> {
        let mut total: u64 = self.undo.iter().map(Entry::bytes).sum();
        while self.undo.len() > self.max_passes || (total > self.max_bytes && self.undo.len() > 1) {
            let e = self.undo.remove(0);
            total -= e.bytes();
            let _ = fs::remove_file(self.dir.join(&e.file));
        }
        Ok(())
    }

    pub fn undo(&mut self, tape: &mut Tape) -> Result<(), UndoError> {
        let entry = self.undo.pop().ok_or(UndoError::Empty("undo"))?;
        let payload = self.read_payload(&entry)?;
        let mut current = vec![0i16; entry.len];
        tape.read_raw(entry.track, entry.start, &mut current);
        tape.write_raw(entry.track, entry.start, &payload);
        let file = self.write_payload(entry.id, &current)?;
        self.redo.push(Entry { file, ..entry });
        Ok(())
    }

    pub fn redo(&mut self, tape: &mut Tape) -> Result<(), UndoError> {
        let entry = self.redo.pop().ok_or(UndoError::Empty("redo"))?;
        let payload = self.read_payload(&entry)?;
        let mut current = vec![0i16; entry.len];
        tape.read_raw(entry.track, entry.start, &mut current);
        tape.write_raw(entry.track, entry.start, &payload);
        let file = self.write_payload(entry.id, &current)?;
        self.undo.push(Entry { file, ..entry });
        Ok(())
    }

    /// Persist stack metadata so undo survives restart (REQ-503).
    pub fn save(&self) -> Result<(), UndoError> {
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
        j.push_pass(&p0).unwrap();
        let after_first = snapshot(&tape, 0, 48_000);

        let p1 = record(&mut tape, 0, 5000, 1100.0, 10_000);
        j.push_pass(&p1).unwrap();
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
        j.push_pass(&p0).unwrap();
        j.undo(&mut tape).unwrap();
        assert!(j.can_redo());

        let p1 = record(&mut tape, 1, 0, 660.0, 10_000);
        j.push_pass(&p1).unwrap();
        assert!(!j.can_redo(), "new take must invalidate the redo branch");
    }

    #[test]
    fn eviction_respects_caps() {
        let dir = TempDir::new("eviction");
        let mut tape = Tape::new(48_000);
        let mut j = Journal::new(&dir.0).unwrap().with_caps(3, u64::MAX);
        for i in 0..6 {
            let p = record(&mut tape, 2, i * 1000, 300.0 + i as f32 * 50.0, 2000);
            j.push_pass(&p).unwrap();
        }
        assert_eq!(j.depth(), 3, "oldest passes evicted");

        // Byte cap binds before the pass cap.
        let mut j2 = Journal::new(&dir.0).unwrap().with_caps(100, 8000);
        for i in 0..5 {
            let p = record(&mut tape, 3, i * 1000, 400.0, 2000);
            j2.push_pass(&p).unwrap();
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
        j.push_pass(&p).unwrap();
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
            j.push_pass(&p0).unwrap();
            baseline = snapshot(&tape, 0, 48_000);
            let p1 = record(&mut tape, 0, 4000, 990.0, 6000);
            j.push_pass(&p1).unwrap();
            j.save().unwrap();
        }
        let mut reloaded = Journal::load(&dir.0).unwrap();
        assert_eq!(reloaded.depth(), 2);
        reloaded.undo(&mut tape).unwrap();
        assert_eq!(snapshot(&tape, 0, 48_000), baseline);
    }
}
