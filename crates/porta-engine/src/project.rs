//! Project persistence (REQ-801, REQ-802). A cassette is a directory:
//!
//! ```text
//! myproject.porta/
//!   manifest.json        tape length, character seed, mixer state
//!   tape/track{0..3}.raw raw i16 LE, written in 5-second chunks
//!   undo/                the journal (see undo.rs)
//! ```
//!
//! Saves rewrite only dirty chunks, seeking to each chunk's offset, so a
//! three-minute overdub costs a few tens of megabytes of writes rather
//! than the whole tape. Callers must only save while stopped.

use crate::mixer::Mixer;
use crate::tape::{BusChannel, Tape, CHUNK_SAMPLES};
use crate::NUM_TRACKS;
use porta_dsp::character::TapeCharacter;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("project io: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad manifest: {0}")]
    Manifest(#[from] serde_json::Error),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub len_samples: usize,
    pub noise_seed: u64,
    /// The cassette's formulation, fixed at creation (REQ-103).
    #[serde(default = "TapeCharacter::default")]
    pub character: TapeCharacter,
    pub playhead: usize,
    pub fader_db: [f32; NUM_TRACKS],
    pub pan: [f32; NUM_TRACKS],
    pub master_db: f32,
    /// A mix decision, persisted like fader/pan - unlike arm and
    /// monitor, which are session-transient "ready to record"/"preview"
    /// states and were never saved either. `#[serde(default)]` so an
    /// existing cassette's manifest.json (saved before this field
    /// existed) still loads, unmuted.
    #[serde(default)]
    pub muted: [bool; NUM_TRACKS],
    /// The bounce bus's own fader/mute (REQ-409). Mix decisions,
    /// persisted for the same reason the per-track ones are - and this
    /// one matters more than most: the bus's mute is load-bearing on
    /// the next bounce (a muted bus prints without folding its prior
    /// content forward), so silently forgetting it across a reopen
    /// would change what the next bounce does. `#[serde(default)]`, so
    /// a pre-bus cassette loads at unity and unmuted.
    #[serde(default)]
    pub bounce_fader_db: f32,
    #[serde(default)]
    pub bounce_muted: bool,
}

impl Manifest {
    pub fn new(len_samples: usize, noise_seed: u64) -> Self {
        Self::with_character(len_samples, TapeCharacter::new(noise_seed))
    }

    pub fn with_character(len_samples: usize, character: TapeCharacter) -> Self {
        Self {
            len_samples,
            noise_seed: character.noise_seed,
            character,
            playhead: 0,
            fader_db: [0.0; NUM_TRACKS],
            pan: [0.0; NUM_TRACKS],
            master_db: 0.0,
            muted: [false; NUM_TRACKS],
            bounce_fader_db: 0.0,
            bounce_muted: false,
        }
    }

    pub fn apply_to(&self, mixer: &mut Mixer) {
        for t in 0..NUM_TRACKS {
            mixer.set_fader_db(t, self.fader_db[t]);
            mixer.set_pan(t, self.pan[t]);
            mixer.set_muted(t, self.muted[t]);
        }
        mixer.set_master_db(self.master_db);
        mixer.set_bus_fader_db(self.bounce_fader_db);
        mixer.set_bus_muted(self.bounce_muted);
    }

    pub fn capture_from(&mut self, mixer: &Mixer) {
        for t in 0..NUM_TRACKS {
            self.fader_db[t] = mixer.fader_db(t);
            self.pan[t] = mixer.pan(t);
            self.muted[t] = mixer.is_muted(t);
        }
        self.master_db = mixer.master_db();
        self.bounce_fader_db = mixer.bus_fader_db();
        self.bounce_muted = mixer.is_bus_muted();
    }
}

pub struct Project {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

fn track_path(dir: &Path, track: usize) -> PathBuf {
    dir.join("tape").join(format!("track{track}.raw"))
}

fn bus_path(dir: &Path, channel: BusChannel) -> PathBuf {
    let name = match channel {
        BusChannel::Left => "bounce_l.raw",
        BusChannel::Right => "bounce_r.raw",
    };
    dir.join("tape").join(name)
}

impl Project {
    /// Create the directory structure and zero-fill the track files.
    pub fn create(
        dir: impl Into<PathBuf>,
        len_samples: usize,
        noise_seed: u64,
    ) -> Result<Self, ProjectError> {
        Self::create_with_character(dir, len_samples, TapeCharacter::new(noise_seed))
    }

    pub fn create_with_character(
        dir: impl Into<PathBuf>,
        len_samples: usize,
        character: TapeCharacter,
    ) -> Result<Self, ProjectError> {
        let dir = dir.into();
        fs::create_dir_all(dir.join("tape"))?;
        fs::create_dir_all(dir.join("undo"))?;
        for t in 0..NUM_TRACKS {
            let f = fs::File::create(track_path(&dir, t))?;
            f.set_len((len_samples * 2) as u64)?;
        }
        for channel in BusChannel::BOTH {
            let f = fs::File::create(bus_path(&dir, channel))?;
            f.set_len((len_samples * 2) as u64)?;
        }
        let project = Self {
            dir,
            manifest: Manifest::with_character(len_samples, character),
        };
        project.write_manifest()?;
        Ok(project)
    }

    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, ProjectError> {
        let dir = dir.into();
        let text = fs::read_to_string(dir.join("manifest.json"))?;
        let manifest: Manifest = serde_json::from_str(&text)?;
        Ok(Self { dir, manifest })
    }

    pub fn undo_dir(&self) -> PathBuf {
        self.dir.join("undo")
    }

    fn write_manifest(&self) -> Result<(), ProjectError> {
        let json = serde_json::to_string_pretty(&self.manifest)?;
        fs::write(self.dir.join("manifest.json"), json)?;
        Ok(())
    }

    /// Write dirty chunks only, then clear the dirty flags. Returns the
    /// number of chunks actually written, which the tests assert on.
    pub fn save_tape(&self, tape: &mut Tape) -> Result<usize, ProjectError> {
        let mut written = 0;
        for t in 0..NUM_TRACKS {
            let dirty = tape.dirty_chunks(t);
            if dirty.is_empty() {
                continue;
            }
            let mut f = fs::OpenOptions::new()
                .write(true)
                .open(track_path(&self.dir, t))?;
            for c in dirty {
                let data = tape.chunk(t, c);
                let mut bytes = Vec::with_capacity(data.len() * 2);
                for &s in data {
                    bytes.extend_from_slice(&s.to_le_bytes());
                }
                f.seek(SeekFrom::Start((c * CHUNK_SAMPLES * 2) as u64))?;
                f.write_all(&bytes)?;
                written += 1;
            }
            tape.clear_dirty(t);
        }
        for channel in BusChannel::BOTH {
            let dirty = tape.bus_dirty_chunks(channel);
            if dirty.is_empty() {
                continue;
            }
            // A cassette created before the bus existed has no file to
            // open - create it on first write rather than failing a
            // save (REQ-801's missing-file rule, write side).
            let path = bus_path(&self.dir, channel);
            if !path.exists() {
                let f = fs::File::create(&path)?;
                f.set_len((self.manifest.len_samples * 2) as u64)?;
            }
            let mut f = fs::OpenOptions::new().write(true).open(path)?;
            for c in dirty {
                let data = tape.bus_chunk(channel, c);
                let mut bytes = Vec::with_capacity(data.len() * 2);
                for &s in data {
                    bytes.extend_from_slice(&s.to_le_bytes());
                }
                f.seek(SeekFrom::Start((c * CHUNK_SAMPLES * 2) as u64))?;
                f.write_all(&bytes)?;
                written += 1;
            }
            tape.clear_bus_dirty(channel);
        }
        Ok(written)
    }

    pub fn save(&self, tape: &mut Tape) -> Result<usize, ProjectError> {
        self.write_manifest()?;
        self.save_tape(tape)
    }

    pub fn load_tape(&self) -> Result<Tape, ProjectError> {
        let mut tape = Tape::new(self.manifest.len_samples);
        for t in 0..NUM_TRACKS {
            let mut bytes = Vec::new();
            fs::File::open(track_path(&self.dir, t))?.read_to_end(&mut bytes)?;
            let samples: Vec<i16> = bytes
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            tape.write_raw(t, 0, &samples);
            tape.clear_dirty(t);
        }
        for channel in BusChannel::BOTH {
            // Missing bus files read as "never bounced yet" - all-zero,
            // exactly how a fresh cassette's tracks start - so every
            // cassette saved before this feature existed opens
            // unchanged rather than erroring (REQ-801).
            let Ok(mut f) = fs::File::open(bus_path(&self.dir, channel)) else {
                continue;
            };
            let mut bytes = Vec::new();
            f.read_to_end(&mut bytes)?;
            let samples: Vec<i16> = bytes
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            tape.write_bus_raw(channel, 0, &samples);
            tape.clear_bus_dirty(channel);
        }
        Ok(tape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::RecordPass;
    use porta_testkit::signal::sine;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("porta-project-{name}"));
            let _ = fs::remove_dir_all(&p);
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn record(tape: &mut Tape, track: usize, start: usize, freq: f32, len: usize) {
        let mut p = RecordPass::new(track, start, 1);
        p.write_block(tape, &sine(freq, -6.0, len));
        p.finish(tape);
    }

    #[test]
    fn tape_roundtrips_through_disk() {
        let dir = TempDir::new("roundtrip");
        let p = Project::create(&dir.0, CHUNK_SAMPLES * 4, 99).unwrap();
        let mut tape = Tape::new(CHUNK_SAMPLES * 4);
        record(&mut tape, 0, 1000, 440.0, 20_000);
        record(&mut tape, 3, CHUNK_SAMPLES * 3, 880.0, 5000);
        p.save(&mut tape).unwrap();

        let reopened = Project::open(&dir.0).unwrap();
        assert_eq!(reopened.manifest.noise_seed, 99);
        let loaded = reopened.load_tape().unwrap();
        for t in 0..NUM_TRACKS {
            let mut a = vec![0i16; CHUNK_SAMPLES * 4];
            let mut b = vec![0i16; CHUNK_SAMPLES * 4];
            tape.read_raw(t, 0, &mut a);
            loaded.read_raw(t, 0, &mut b);
            assert_eq!(a, b, "track {t} differs after reload");
        }
    }

    #[test]
    fn only_dirty_chunks_are_written() {
        let dir = TempDir::new("dirty");
        let p = Project::create(&dir.0, CHUNK_SAMPLES * 8, 1).unwrap();
        let mut tape = Tape::new(CHUNK_SAMPLES * 8);

        // One take covering roughly two chunks on one track.
        record(&mut tape, 1, 0, 440.0, CHUNK_SAMPLES + 100);
        assert_eq!(p.save(&mut tape).unwrap(), 2);

        // Nothing changed since: no chunk writes at all.
        assert_eq!(p.save(&mut tape).unwrap(), 0);

        // A short take inside one chunk costs exactly one chunk write.
        record(&mut tape, 2, CHUNK_SAMPLES * 5, 660.0, 500);
        assert_eq!(p.save(&mut tape).unwrap(), 1);
    }

    #[test]
    fn mixer_state_persists() {
        let dir = TempDir::new("mixer");
        let mut p = Project::create(&dir.0, 48_000, 5).unwrap();
        let mut mixer = Mixer::new();
        mixer.set_fader_db(2, -7.5);
        mixer.set_pan(2, -0.5);
        mixer.set_master_db(-2.0);
        p.manifest.capture_from(&mixer);
        p.manifest.playhead = 12_345;
        let mut tape = Tape::new(48_000);
        p.save(&mut tape).unwrap();

        let reopened = Project::open(&dir.0).unwrap();
        let mut restored = Mixer::new();
        reopened.manifest.apply_to(&mut restored);
        assert_eq!(restored.fader_db(2), -7.5);
        assert_eq!(restored.pan(2), -0.5);
        assert_eq!(restored.master_db(), -2.0);
        assert_eq!(reopened.manifest.playhead, 12_345);
    }
    #[test]
    fn bus_audio_and_controls_roundtrip_through_disk() {
        let dir = TempDir::new("bus-roundtrip");
        let p = Project::create(&dir.0, CHUNK_SAMPLES * 2, 42).unwrap();
        let mut tape = Tape::new(CHUNK_SAMPLES * 2);
        let left: Vec<i16> = (0..5000).map(|i| (i * 7) as i16).collect();
        let right: Vec<i16> = (0..5000).map(|i| -((i * 3) as i16)).collect();
        tape.write_bus_raw(BusChannel::Left, 100, &left);
        tape.write_bus_raw(BusChannel::Right, 100, &right);
        p.save(&mut tape).unwrap();

        let loaded = p.load_tape().unwrap();
        let mut got_l = vec![0i16; 5000];
        let mut got_r = vec![0i16; 5000];
        loaded.read_bus_raw(BusChannel::Left, 100, &mut got_l);
        loaded.read_bus_raw(BusChannel::Right, 100, &mut got_r);
        assert_eq!(got_l, left, "bus left must survive a save/reopen");
        assert_eq!(got_r, right, "bus right must survive a save/reopen");
        assert!(
            loaded.bus_dirty_chunks(BusChannel::Left).is_empty(),
            "a freshly loaded bus is not dirty"
        );
    }

    #[test]
    fn only_dirty_bus_chunks_are_written() {
        let dir = TempDir::new("bus-dirty");
        let p = Project::create(&dir.0, CHUNK_SAMPLES * 4, 1).unwrap();
        let mut tape = Tape::new(CHUNK_SAMPLES * 4);
        tape.write_bus_raw(BusChannel::Left, 0, &[5i16; 100]);
        // One chunk, one channel - not eight.
        assert_eq!(p.save_tape(&mut tape).unwrap(), 1);
        assert_eq!(p.save_tape(&mut tape).unwrap(), 0, "nothing left dirty");
    }

    #[test]
    fn bus_fader_and_mute_persist_and_default_for_old_cassettes() {
        let dir = TempDir::new("bus-controls");
        let mut p = Project::create(&dir.0, 48_000, 1).unwrap();
        let mut mixer = Mixer::new();
        mixer.set_bus_fader_db(-7.5);
        mixer.set_bus_muted(true);
        p.manifest.capture_from(&mixer);
        p.save(&mut Tape::new(48_000)).unwrap();

        let reopened = Project::open(&dir.0).unwrap();
        assert_eq!(reopened.manifest.bounce_fader_db, -7.5);
        assert!(reopened.manifest.bounce_muted);
        let mut fresh = Mixer::new();
        reopened.manifest.apply_to(&mut fresh);
        assert_eq!(fresh.bus_fader_db(), -7.5);
        assert!(fresh.is_bus_muted());
    }

    #[test]
    fn a_pre_bus_cassette_opens_with_a_silent_bus_at_unity() {
        // REQ-801: no bus files, no bus fields in the manifest - the
        // shape every cassette saved before this feature has.
        let dir = TempDir::new("pre-bus");
        let p = Project::create(&dir.0, 48_000, 1).unwrap();
        std::fs::remove_file(dir.0.join("tape").join("bounce_l.raw")).unwrap();
        std::fs::remove_file(dir.0.join("tape").join("bounce_r.raw")).unwrap();
        let old = r#"{"len_samples":48000,"noise_seed":1,"playhead":0,
            "fader_db":[0.0,0.0,0.0,0.0],"pan":[0.0,0.0,0.0,0.0],"master_db":0.0}"#;
        std::fs::write(dir.0.join("manifest.json"), old).unwrap();

        let reopened = Project::open(&dir.0).unwrap();
        assert_eq!(reopened.manifest.bounce_fader_db, 0.0, "unity");
        assert!(!reopened.manifest.bounce_muted, "unmuted");
        let tape = reopened
            .load_tape()
            .expect("missing bus files must not error");
        let mut got = vec![9i16; 100];
        tape.read_bus_raw(BusChannel::Left, 0, &mut got);
        assert_eq!(got, vec![0i16; 100], "never bounced reads as silence");

        // And saving such a cassette creates the files rather than failing.
        let mut tape = tape;
        tape.write_bus_raw(BusChannel::Left, 0, &[3i16; 50]);
        p.save_tape(&mut tape).unwrap();
        assert!(dir.0.join("tape").join("bounce_l.raw").exists());
    }
}
