//! The engine facade: tape, transport, mixer, record passes, and undo
//! behind one hardware-agnostic block-processing API. Callers push input
//! blocks and receive stereo output; nothing here knows about audio
//! devices.
//!
//! Each record pass builds a fresh character chain from the cassette's
//! `TapeCharacter` with a per-pass seed, so degradation is baked onto
//! tape at record time and compounds across generations (REQ-303).

use crate::mixer::Mixer;
use crate::project::{Manifest, Project, ProjectError};
use crate::record::RecordPass;
use crate::tape::Tape;
use crate::transport::{Transport, TransportState};
use crate::undo::{Journal, UndoError};
use crate::NUM_TRACKS;
use porta_dsp::character::TapeCharacter;
use porta_dsp::{AudioProcessor, Chain, MAX_BLOCK};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    Undo(#[from] UndoError),
    #[error("{0} is only allowed while stopped")]
    NotStopped(&'static str),
}

pub struct Engine {
    tape: Tape,
    transport: Transport,
    mixer: Mixer,
    journal: Journal,
    project: Project,
    armed: [bool; NUM_TRACKS],
    passes: [Option<RecordPass>; NUM_TRACKS],
    chains: Vec<Chain>,
    pass_counter: u64,
    // Per-track scratch: processed input (record path) and tape playback.
    processed: Vec<Vec<f32>>,
    playback: Vec<Vec<f32>>,
}

impl Engine {
    pub fn create(
        dir: impl Into<std::path::PathBuf>,
        len_samples: usize,
        noise_seed: u64,
    ) -> Result<Self, EngineError> {
        Self::create_with_character(dir, len_samples, TapeCharacter::new(noise_seed))
    }

    /// Create a cassette with an explicit formulation. Mostly for tests
    /// that want the tape mechanics without the colour.
    pub fn create_with_character(
        dir: impl Into<std::path::PathBuf>,
        len_samples: usize,
        character: TapeCharacter,
    ) -> Result<Self, EngineError> {
        let project = Project::create_with_character(dir, len_samples, character)?;
        let tape = Tape::new(len_samples);
        Self::assemble(project, tape)
    }

    pub fn open(dir: impl Into<std::path::PathBuf>) -> Result<Self, EngineError> {
        let project = Project::open(dir)?;
        let tape = project.load_tape()?;
        Self::assemble(project, tape)
    }

    fn assemble(project: Project, tape: Tape) -> Result<Self, EngineError> {
        let journal = Journal::load(project.undo_dir())?;
        let mut mixer = Mixer::new();
        project.manifest.apply_to(&mut mixer);
        let mut transport = Transport::new(tape.len_samples());
        transport.seek(project.manifest.playhead);
        Ok(Self {
            tape,
            transport,
            mixer,
            journal,
            project,
            armed: [false; NUM_TRACKS],
            passes: [const { None }; NUM_TRACKS],
            // Replaced per pass in `record()`; a passthrough placeholder
            // keeps the array populated while stopped.
            chains: (0..NUM_TRACKS).map(|_| Chain::passthrough()).collect(),
            pass_counter: 0,
            processed: vec![vec![0.0; MAX_BLOCK]; NUM_TRACKS],
            playback: vec![vec![0.0; MAX_BLOCK]; NUM_TRACKS],
        })
    }

    pub fn mixer(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    pub fn tape(&self) -> &Tape {
        &self.tape
    }

    pub fn state(&self) -> TransportState {
        self.transport.state()
    }

    pub fn playhead(&self) -> usize {
        self.transport.playhead()
    }

    pub fn manifest(&self) -> &Manifest {
        &self.project.manifest
    }

    pub fn set_armed(&mut self, track: usize, armed: bool) {
        self.armed[track] = armed;
    }

    pub fn is_armed(&self, track: usize) -> bool {
        self.armed[track]
    }

    pub fn fader_db(&self, track: usize) -> f32 {
        self.mixer.fader_db(track)
    }

    pub fn pan(&self, track: usize) -> f32 {
        self.mixer.pan(track)
    }

    pub fn master_db(&self) -> f32 {
        self.mixer.master_db()
    }

    /// Post-fader peak of `track` from the most recently mixed block,
    /// in dBFS. For a UI meter; see Mixer::track_level_db.
    pub fn track_level_db(&self, track: usize) -> f32 {
        self.mixer.track_level_db(track)
    }

    /// Peak of the summed stereo output from the most recently mixed
    /// block, in dBFS.
    pub fn master_level_db(&self) -> (f32, f32) {
        self.mixer.master_level_db()
    }

    pub fn seek(&mut self, pos: usize) -> bool {
        self.transport.seek(pos)
    }

    pub fn play(&mut self) {
        self.close_passes();
        self.transport.play();
    }

    pub fn stop(&mut self) {
        self.close_passes();
        self.transport.stop();
    }

    /// Bounce tracks 1-3 down to track 4 (REQ-401). The input is the
    /// engine's own post-fader mono sum of the source tracks; pans are
    /// ignored, matching the reference hardware's bus (REQ-603). The sum
    /// is printed through a record pass, so generation loss and undo
    /// come for free.
    ///
    /// Rewinds to the start, records the whole tape, and leaves the
    /// transport stopped. Refused while rolling.
    pub fn bounce(&mut self) -> Result<(), EngineError> {
        if !self.transport.is_stopped() {
            return Err(EngineError::NotStopped("bounce"));
        }
        const DEST: usize = NUM_TRACKS - 1;
        let sources: Vec<usize> = (0..DEST).collect();
        let gains: Vec<f32> = sources
            .iter()
            .map(|&t| db_to_amp(self.mixer.fader_db(t)))
            .collect();

        let armed_before = self.armed;
        self.armed = [false; NUM_TRACKS];
        self.armed[DEST] = true;
        self.seek(0);
        self.record();

        let len = self.tape.len_samples();
        let mut sum = vec![0.0f32; MAX_BLOCK];
        let mut scratch = vec![0.0f32; MAX_BLOCK];
        let mut sink_l = vec![0.0f32; MAX_BLOCK];
        let mut sink_r = vec![0.0f32; MAX_BLOCK];
        let mut pos = 0;
        while pos < len {
            let n = MAX_BLOCK.min(len - pos);
            sum[..n].fill(0.0);
            for (i, &t) in sources.iter().enumerate() {
                self.tape.read(t, pos, &mut scratch[..n]);
                for (dst, &s) in sum[..n].iter_mut().zip(&scratch[..n]) {
                    *dst += s * gains[i];
                }
            }
            let quiet = &scratch[..0];
            let inputs: [&[f32]; NUM_TRACKS] =
                std::array::from_fn(|t| if t == DEST { &sum[..n] } else { quiet });
            let done = self.process_block(&inputs, &mut sink_l[..n], &mut sink_r[..n]);
            if done == 0 {
                break;
            }
            pos += done;
        }
        self.stop();
        self.armed = armed_before;
        Ok(())
    }

    /// Engage recording on the armed tracks, opening a pass for each.
    pub fn record(&mut self) {
        if self.armed.iter().all(|&a| !a) {
            return;
        }
        let start = self.transport.playhead();
        let capacity = self.tape.len_samples().saturating_sub(start);
        for t in 0..NUM_TRACKS {
            if self.armed[t] && self.passes[t].is_none() {
                let seed = seed_for(self.project.manifest.noise_seed, self.pass_counter);
                self.pass_counter += 1;
                self.passes[t] = Some(RecordPass::with_capacity(
                    t, start, seed, capacity, MAX_BLOCK,
                ));
                // A fresh chain per pass: flutter and hiss get their own
                // seed so successive generations decorrelate (REQ-304).
                self.chains[t] = self.project.manifest.character.build_chain(seed);
            }
        }
        self.transport.record();
    }

    /// Close any open passes, applying punch-out fades and journaling.
    /// Reachable from process_block itself (transport hitting the tape
    /// end while recording) as well as Stop/Play, all of which the
    /// realtime adapter runs directly on the audio callback - so this
    /// must never touch disk. Journal::push_pass only does in-memory
    /// bookkeeping; the actual write is deferred until save/undo/redo,
    /// which are always run off the realtime thread (REQ-902).
    fn close_passes(&mut self) {
        for t in 0..NUM_TRACKS {
            if let Some(mut pass) = self.passes[t].take() {
                pass.finish(&mut self.tape);
                self.journal.push_pass(pass);
            }
        }
    }

    /// Process one block. `inputs` supplies live input per track (only
    /// armed tracks while recording consume it). Output is stereo.
    /// Returns samples actually produced, which is short at the tape end.
    pub fn process_block(
        &mut self,
        inputs: &[&[f32]; NUM_TRACKS],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) -> usize {
        let want = out_l.len().min(out_r.len()).min(MAX_BLOCK);
        let pos = self.transport.playhead();
        let remaining = self.tape.len_samples().saturating_sub(pos);
        let n = if self.transport.is_stopped() {
            0
        } else {
            want.min(remaining)
        };

        out_l[n..].fill(0.0);
        out_r[n..].fill(0.0);
        if n == 0 {
            out_l[..].fill(0.0);
            out_r[..].fill(0.0);
            return 0;
        }

        let recording = self.transport.state() == TransportState::Recording;
        for t in 0..NUM_TRACKS {
            if recording && self.armed[t] {
                let src = &inputs[t][..n.min(inputs[t].len())];
                self.processed[t][..src.len()].copy_from_slice(src);
                self.processed[t][src.len()..n].fill(0.0);
                self.chains[t].process(&mut self.processed[t][..n]);
                if let Some(pass) = self.passes[t].as_mut() {
                    pass.write_block(&mut self.tape, &self.processed[t][..n]);
                }
                // Monitoring is post-chain (REQ-305): the player hears
                // what the tape is receiving, not the old tape content.
                self.playback[t][..n].copy_from_slice(&self.processed[t][..n]);
            } else {
                let (dst, _) = self.playback[t].split_at_mut(n);
                self.tape.read(t, pos, dst);
            }
        }

        let views: [&[f32]; NUM_TRACKS] = std::array::from_fn(|t| &self.playback[t][..n]);
        self.mixer
            .mix_block(&views, &mut out_l[..n], &mut out_r[..n]);
        self.transport.advance(n);
        if self.transport.is_stopped() {
            self.close_passes();
        }
        n
    }

    pub fn undo(&mut self) -> Result<(), EngineError> {
        if !self.transport.is_stopped() {
            return Err(EngineError::NotStopped("undo"));
        }
        self.journal.undo(&mut self.tape)?;
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), EngineError> {
        if !self.transport.is_stopped() {
            return Err(EngineError::NotStopped("redo"));
        }
        self.journal.redo(&mut self.tape)?;
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        self.journal.can_undo()
    }

    pub fn save(&mut self) -> Result<(), EngineError> {
        if !self.transport.is_stopped() {
            return Err(EngineError::NotStopped("save"));
        }
        self.project.manifest.capture_from(&self.mixer);
        self.project.manifest.playhead = self.transport.playhead();
        self.project.save(&mut self.tape)?;
        self.journal.save()?;
        Ok(())
    }
}

fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Per-pass seed: cassette seed mixed with the pass counter, so each pass
/// is decorrelated but the whole session stays reproducible (REQ-304).
fn seed_for(noise_seed: u64, pass: u64) -> u32 {
    let mixed = noise_seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(pass.wrapping_mul(1_442_695_040_888_963_407));
    ((mixed >> 32) as u32) | 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use porta_testkit::meter::rms_dbfs;
    use porta_testkit::signal::{silence, sine};

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("porta-engine-{name}"));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Feed `signal` through the engine in `block` sized chunks, returning
    /// the stereo left channel.
    fn run(engine: &mut Engine, track: usize, signal: &[f32], block: usize) -> Vec<f32> {
        let quiet = silence(block);
        let mut left = Vec::new();
        let mut l = vec![0.0; block];
        let mut r = vec![0.0; block];
        let mut fed = 0;
        while fed < signal.len() {
            let end = (fed + block).min(signal.len());
            let chunk = &signal[fed..end];
            let mut inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
            inputs[track] = chunk;
            let n = engine.process_block(&inputs, &mut l[..chunk.len()], &mut r[..chunk.len()]);
            left.extend_from_slice(&l[..n]);
            if n == 0 {
                break;
            }
            fed = end;
        }
        left
    }

    #[test]
    fn record_then_play_returns_the_take() {
        let dir = TempDir::new("roundtrip");
        let mut e = Engine::create_with_character(&dir.0, 96_000, TapeCharacter::clean()).unwrap();
        e.set_armed(0, true);
        e.record();
        let take = sine(1000.0, -6.0, 24_000);
        run(&mut e, 0, &take, 512);
        e.stop();

        e.seek(0);
        e.play();
        let played = run(&mut e, 0, &silence(24_000), 512);
        // Center pan costs 3 dB; the take was -6 dBFS peak (-9 dB RMS).
        assert!(
            (rms_dbfs(&played) - (-12.0)).abs() < 0.5,
            "played {} dBFS",
            rms_dbfs(&played)
        );
    }

    #[test]
    fn levels_reflect_the_current_block_during_playback() {
        let dir = TempDir::new("levels");
        let mut e = Engine::create_with_character(&dir.0, 96_000, TapeCharacter::clean()).unwrap();
        e.set_armed(0, true);
        e.record();
        let take = sine(1000.0, 0.0, 24_000); // 0 dBFS peak
        run(&mut e, 0, &take, 512);
        e.stop();
        // (The meter was already live during that recording pass, per
        // REQ-305 - monitoring goes through the same mix_block call.)

        e.seek(0);
        e.play();
        run(&mut e, 0, &silence(24_000), 512);

        // Track 0's fader is unity, so its meter reads the take's own
        // peak; the other tracks never played anything.
        assert!(
            (e.track_level_db(0) - 0.0).abs() < 0.5,
            "track 0 got {} dB",
            e.track_level_db(0)
        );
        assert!(e.track_level_db(1) < -100.0);

        // Center pan costs 3.01 dB per side.
        let (ml, mr) = e.master_level_db();
        assert!((ml - (-3.01)).abs() < 0.5, "master L got {ml} dB");
        assert!((mr - (-3.01)).abs() < 0.5, "master R got {mr} dB");
    }

    #[test]
    fn unarmed_tracks_stay_silent_and_untouched() {
        let dir = TempDir::new("unarmed");
        let mut e = Engine::create_with_character(&dir.0, 96_000, TapeCharacter::clean()).unwrap();
        let mut before = [vec![0i16; 96_000], vec![0i16; 96_000], vec![0i16; 96_000]];
        for (i, b) in before.iter_mut().enumerate() {
            e.tape().read_raw(i + 1, 0, b);
        }
        e.set_armed(0, true);
        e.record();
        run(&mut e, 0, &sine(440.0, -3.0, 20_000), 256);
        e.stop();
        for (i, b) in before.iter().enumerate() {
            let mut now = vec![0i16; 96_000];
            e.tape().read_raw(i + 1, 0, &mut now);
            assert_eq!(&now, b, "track {} changed", i + 1);
        }
    }

    #[test]
    fn undo_removes_the_take_and_is_stop_gated() {
        let dir = TempDir::new("undo");
        let mut e = Engine::create_with_character(&dir.0, 96_000, TapeCharacter::clean()).unwrap();
        e.set_armed(1, true);
        e.record();
        run(&mut e, 1, &sine(440.0, -3.0, 20_000), 480);
        e.stop();
        assert!(e.can_undo());

        e.seek(0);
        e.play();
        assert!(matches!(e.undo(), Err(EngineError::NotStopped(_))));
        e.stop();

        e.undo().unwrap();
        e.seek(0);
        e.play();
        let played = run(&mut e, 1, &silence(20_000), 480);
        assert!(rms_dbfs(&played) < -80.0, "tape should be blank again");
    }

    #[test]
    fn block_size_does_not_change_the_result() {
        let take = sine(777.0, -6.0, 30_000);
        let mut renders = Vec::new();
        for (i, block) in [64usize, 480, 1024].iter().enumerate() {
            let dir = TempDir::new(&format!("blocksize{i}"));
            let mut e =
                Engine::create_with_character(&dir.0, 96_000, TapeCharacter::clean()).unwrap();
            e.set_armed(2, true);
            e.record();
            run(&mut e, 2, &take, *block);
            e.stop();
            let mut raw = vec![0i16; 30_000];
            e.tape().read_raw(2, 0, &mut raw);
            renders.push(raw);
        }
        assert_eq!(renders[0], renders[1], "64 vs 480 differ");
        assert_eq!(renders[1], renders[2], "480 vs 1024 differ");
    }

    #[test]
    fn recording_stops_at_tape_end() {
        let dir = TempDir::new("tapeend");
        let mut e = Engine::create_with_character(&dir.0, 10_000, TapeCharacter::clean()).unwrap();
        e.set_armed(0, true);
        e.seek(9_000);
        e.record();
        run(&mut e, 0, &sine(440.0, -6.0, 5_000), 512);
        assert_eq!(e.state(), TransportState::Stopped);
        assert_eq!(e.playhead(), 10_000);
        assert!(e.can_undo(), "the partial pass was still journaled");
    }

    #[test]
    fn session_survives_save_and_reopen() {
        let dir = TempDir::new("persist");
        {
            let mut e =
                Engine::create_with_character(&dir.0, 96_000, TapeCharacter::clean()).unwrap();
            e.set_armed(3, true);
            e.record();
            run(&mut e, 3, &sine(600.0, -6.0, 20_000), 512);
            e.stop();
            e.mixer().set_fader_db(3, -4.0);
            e.save().unwrap();
        }
        let mut e = Engine::open(&dir.0).unwrap();

        e.seek(0);
        e.play();
        let played = run(&mut e, 3, &silence(20_000), 512);
        // -6 peak sine = -9 RMS, -3 pan, -4 fader.
        assert!(
            (rms_dbfs(&played) - (-16.0)).abs() < 0.5,
            "played {} dBFS",
            rms_dbfs(&played)
        );
        assert!(e.can_undo(), "undo journal survived reload");
    }

    /// REQ-902 regression (M4.4): Stop, and process_block's own
    /// auto-stop at the tape end, run on the realtime audio thread and
    /// must never touch disk. The pass payload should stay in memory
    /// until something that always runs off that thread - save, here -
    /// flushes it.
    #[test]
    fn stop_does_not_write_the_journal_payload_until_save() {
        let dir = TempDir::new("deferred-journal");
        let mut e = Engine::create_with_character(&dir.0, 96_000, TapeCharacter::clean()).unwrap();
        e.set_armed(0, true);
        e.record();
        run(&mut e, 0, &sine(440.0, -6.0, 10_000), 512);
        e.stop();

        assert!(e.can_undo(), "entry recorded in memory");
        let undo_dir = dir.0.join("undo");
        assert!(
            !has_pass_payload(&undo_dir),
            "stop must not write the pass payload to disk"
        );

        e.save().unwrap();
        assert!(
            has_pass_payload(&undo_dir),
            "save must flush the deferred pass payload"
        );
    }

    fn has_pass_payload(undo_dir: &std::path::Path) -> bool {
        std::fs::read_dir(undo_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| e.file_name().to_string_lossy().starts_with("pass-"))
            })
            .unwrap_or(false)
    }
}
