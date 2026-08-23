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
use crate::record::{BouncePass, RecordPass};
use crate::tape::{BusChannel, Tape};
use crate::transport::{Transport, TransportState};
use crate::undo::{Journal, UndoError};
use crate::NUM_TRACKS;
use porta_dsp::character::TapeCharacter;
use porta_dsp::flutter::StereoFlutter;
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
    /// The bounce bus's arm-like flag (REQ-404) - separate from
    /// `armed`, and mutually exclusive with it (REQ-405). Session
    /// state, not persisted, exactly like the track arms.
    bus_armed: bool,
    /// Whether an armed track's live input is audible while stopped or
    /// playing, not just while actually recording - see
    /// `Command::Monitor`.
    monitor: [bool; NUM_TRACKS],
    passes: [Option<RecordPass>; NUM_TRACKS],
    chains: Vec<Chain>,
    pass_counter: u64,
    pass_buffer_fallbacks: u64,
    // Per-track scratch: processed input (record path) and tape playback.
    processed: Vec<Vec<f32>>,
    playback: Vec<Vec<f32>>,
    /// The bus's own playback slot (L/R), mirroring a track's
    /// `playback`: whatever the bus is currently contributing to the
    /// mix. Ordinary tape readback when no pass is open; during a
    /// bounce it holds the freshly printed signal (REQ-408). Allocated
    /// here, once, off the realtime thread.
    bus_playback: (Vec<f32>, Vec<f32>),
    /// Phase 1's ungated print sum, threaded from `sum_tracks` through
    /// the bus's own chain (REQ-406). Engine-owned and allocated once:
    /// `Mixer` has no reason to remember it between calls.
    print_buf: (Vec<f32>, Vec<f32>),
    /// The bus's smoothed gain, ticked once per sample up front and
    /// read back at BOTH use sites - folded into the print input
    /// pre-chain, and applied to the monitor output post-chain. Ticking
    /// at each site instead would advance the ramp twice per sample,
    /// doubling its rate and making the result depend on how often it
    /// happened to be read (REQ-203/602).
    bus_gain_scratch: Vec<f32>,
    /// One channel's chain halves either side of the shared flutter,
    /// built unconditionally at open/create (never lazily, never on the
    /// audio thread) and reset+reseeded per pass.
    bounce_chain_l: (Chain, Chain),
    bounce_chain_r: (Chain, Chain),
    bounce_flutter: StereoFlutter,
    bounce_pass: BouncePass,
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
        // The bounce reserve is sized to this cassette's tape length and
        // allocated here, off the realtime thread, alongside `Tape`
        // itself - a bounce engages from a non-blocking command, so it
        // can never allocate these for itself (REQ-902).
        let journal = Journal::load(project.undo_dir())?.with_bus_reserve(tape.len_samples());
        let mut mixer = Mixer::new();
        project.manifest.apply_to(&mut mixer);
        let mut transport = Transport::new(tape.len_samples());
        transport.seek(project.manifest.playhead);
        // Copied out before `project` moves into the struct literal below.
        let character = project.manifest.character;
        Ok(Self {
            tape,
            transport,
            mixer,
            journal,
            project,
            armed: [false; NUM_TRACKS],
            bus_armed: false,
            monitor: [false; NUM_TRACKS],
            passes: [const { None }; NUM_TRACKS],
            // Built for real here, off the realtime thread, so `record()`
            // (which runs on it) only ever has to reseed an existing
            // chain in place, not allocate a new one (REQ-902 - see
            // reseed_chain's doc comment). The seed itself doesn't matter
            // yet - every track's chain gets a real pass seed the first
            // time record() engages it, before anything's been written.
            chains: (0..NUM_TRACKS).map(|_| character.build_chain(0)).collect(),
            pass_counter: 0,
            pass_buffer_fallbacks: 0,
            processed: vec![vec![0.0; MAX_BLOCK]; NUM_TRACKS],
            playback: vec![vec![0.0; MAX_BLOCK]; NUM_TRACKS],
            bus_playback: (vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]),
            print_buf: (vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]),
            bus_gain_scratch: vec![0.0; MAX_BLOCK],
            // Unconditionally, like the track chains above - not lazily
            // on first arm, which would land on the audio thread
            // (arming is not a blocking command).
            bounce_chain_l: character.build_split_chain(0),
            bounce_chain_r: character.build_split_chain(0),
            bounce_flutter: character.build_stereo_flutter(0),
            bounce_pass: BouncePass::new(MAX_BLOCK),
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

    /// Arm a track. Arming any track disarms the bounce bus (REQ-405):
    /// a bounce pass and a live input pass never overlap, so there is
    /// no simultaneous case to reason about anywhere downstream.
    pub fn set_armed(&mut self, track: usize, armed: bool) {
        self.armed[track] = armed;
        if armed {
            self.bus_armed = false;
        }
    }

    pub fn is_armed(&self, track: usize) -> bool {
        self.armed[track]
    }

    /// Arm the bounce bus (REQ-404). Arming it clears all four track
    /// arms, the other half of REQ-405's mutual exclusion. A direct
    /// consequence, stated so it isn't a surprise: no track's live
    /// input can be monitored while the bus is armed, since
    /// input-monitor preview requires an armed track. Intended - a
    /// bounce is about the bus's printed signal, not a live source.
    pub fn set_bus_armed(&mut self, armed: bool) {
        self.bus_armed = armed;
        if armed {
            self.armed = [false; NUM_TRACKS];
        }
    }

    pub fn is_bus_armed(&self) -> bool {
        self.bus_armed
    }

    pub fn set_monitor(&mut self, track: usize, on: bool) {
        self.monitor[track] = on;
    }

    pub fn is_monitor(&self, track: usize) -> bool {
        self.monitor[track]
    }

    pub fn fader_db(&self, track: usize) -> f32 {
        self.mixer.fader_db(track)
    }

    pub fn is_muted(&self, track: usize) -> bool {
        self.mixer.is_muted(track)
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

    /// Engage recording on the armed tracks, opening a pass for each.
    pub fn record(&mut self) {
        if self.bus_armed {
            self.begin_bounce_pass();
            self.transport.record();
            return;
        }
        if self.armed.iter().all(|&a| !a) {
            return;
        }
        let start = self.transport.playhead();
        for t in 0..NUM_TRACKS {
            if self.armed[t] && self.passes[t].is_none() {
                let seed = seed_for(self.project.manifest.noise_seed, self.pass_counter);
                self.pass_counter += 1;
                // This track's whole chunk-buffer reserve, not a
                // reserve_exact sized to the whole remaining tape - this
                // runs on the realtime audio thread (REQ-902; see
                // record.rs's module doc comment).
                let spares = self.journal.take_spares(t);
                self.passes[t] = Some(RecordPass::with_spares(t, start, seed, MAX_BLOCK, spares));
                // Reseed the track's existing chain in place rather than
                // building a fresh one (that used to be a realtime-thread
                // allocation - see reseed_chain's doc comment; found in
                // review independent of the bounce proposal). Flutter and
                // hiss still get their own seed each pass so successive
                // generations decorrelate (REQ-304) - reseed_chain does
                // that the same way build_chain always did.
                self.project
                    .manifest
                    .character
                    .reseed_chain(&mut self.chains[t], seed);
            }
        }
        self.transport.record();
    }

    /// Engage a stereo bounce pass (REQ-301/401). Realtime-safe: the
    /// capture buffers come from the journal's pre-allocated reserve
    /// and the chains are reseeded in place, never rebuilt.
    fn begin_bounce_pass(&mut self) {
        if self.bounce_pass.is_open() {
            return;
        }
        let start = self.transport.playhead();
        let noise_seed = self.project.manifest.noise_seed;
        let pass = self.pass_counter;
        self.pass_counter += 1;
        let seed_l = seed_for_channel(noise_seed, pass, 0);
        let seed_r = seed_for_channel(noise_seed, pass, 1);

        let character = self.project.manifest.character;
        character.reseed_split_chain(
            &mut self.bounce_chain_l.0,
            &mut self.bounce_chain_l.1,
            seed_l,
        );
        character.reseed_split_chain(
            &mut self.bounce_chain_r.0,
            &mut self.bounce_chain_r.1,
            seed_r,
        );
        // One shared modulator, seeded at channel term 0 by convention.
        self.bounce_flutter.reseed(seed_l);

        let len = self.tape.len_samples();
        let (left, right, from_reserve) = match self.journal.take_bus_buffers() {
            Some((l, r)) => (l, r, true),
            // Documented, counted fallback: both reserve pairs are still
            // out (two bounces already pending a flush). Allocating here
            // is a real REQ-902 exception, so it is counted rather than
            // hidden - see pass_buffer_fallbacks().
            None => (vec![0i16; len], vec![0i16; len], false),
        };
        self.bounce_pass
            .begin(start, seed_l, seed_r, left, right, from_reserve);
        // Tracks 1-4 are already inside what the bus is printing, so
        // their own contribution to the audible mix goes silent for the
        // duration - metering deliberately unaffected (REQ-408).
        for t in 0..NUM_TRACKS {
            self.mixer.set_excluded_from_sum(t, true);
        }
    }

    /// Close any open passes, applying punch-out fades and journaling.
    /// Reachable from process_block itself (transport hitting the tape
    /// end while recording) as well as Stop/Play, all of which the
    /// realtime adapter runs directly on the audio callback - so this
    /// must never touch disk. Journal::push_pass only does in-memory
    /// bookkeeping; the actual write is deferred until save/undo/redo,
    /// which are always run off the realtime thread (REQ-902).
    fn close_passes(&mut self) {
        if self.bounce_pass.is_open() {
            let (len, left, right) = self.bounce_pass.finish(&mut self.tape);
            if self.bounce_pass.allocated_on_thread {
                self.pass_buffer_fallbacks += 1;
            }
            let start = self.bounce_pass.start;
            self.journal.push_bus_pass(start, len, left, right);
            for t in 0..NUM_TRACKS {
                self.mixer.set_excluded_from_sum(t, false);
            }
        }
        for t in 0..NUM_TRACKS {
            if let Some(mut pass) = self.passes[t].take() {
                pass.finish(&mut self.tape);
                if pass.allocated_on_thread {
                    self.pass_buffer_fallbacks += 1;
                }
                self.journal.push_pass(pass);
            }
        }
    }

    /// How many times a record pass has had to fall back to an ordinary
    /// allocation because the journal's pre-reserved chunk pool ran dry
    /// (see `record.rs`'s module doc comment) - should stay at 0 in
    /// ordinary use; a non-zero count is a real, honest signal, not a
    /// crash or silently-wrong undo data.
    pub fn pass_buffer_fallbacks(&self) -> u64 {
        self.pass_buffer_fallbacks
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
            } else if self.armed[t] && self.monitor[t] {
                // Input-monitor preview: hear what's coming in on an
                // armed track without actually recording it, to check
                // levels or mic placement before committing. Dry, not
                // run through the character chain - that's reseeded
                // fresh per record pass (REQ-303/REQ-304), and reusing
                // it here for a stateless preview would mutate state a
                // real pass doesn't expect to have been touched.
                let src = &inputs[t][..n.min(inputs[t].len())];
                self.playback[t][..src.len()].copy_from_slice(src);
                self.playback[t][src.len()..n].fill(0.0);
            } else {
                let (dst, _) = self.playback[t].split_at_mut(n);
                self.tape.read(t, pos, dst);
            }
        }

        let bouncing = recording && self.bounce_pass.is_open();

        // Phase 1. During a bounce the print sum is what feeds the bus's
        // chain; tracks 1-4 are in it at full weight even though they
        // are excluded from the audible monitor sum (REQ-406/408).
        let views: [&[f32]; NUM_TRACKS] = std::array::from_fn(|t| &self.playback[t][..n]);
        if bouncing {
            let (pl, pr) = &mut self.print_buf;
            self.mixer.sum_tracks(
                &views,
                &mut out_l[..n],
                &mut out_r[..n],
                Some((&mut pl[..n], &mut pr[..n])),
            );
        } else {
            self.mixer
                .sum_tracks(&views, &mut out_l[..n], &mut out_r[..n], None);
        }

        // The bus's gain for this block, ticked exactly once per sample
        // and read back at both use sites below (REQ-408's tick-once
        // rule) - the chain runs between them, so neither site can
        // safely tick for itself.
        self.mixer.tick_bus_gain(&mut self.bus_gain_scratch[..n]);

        if bouncing {
            // REQ-407: the bus's own prior content at this position,
            // read BEFORE the pass writes over it, is part of what gets
            // printed - which is what makes a second bounce fold the
            // first one forward with no special self-referential code.
            self.tape
                .read_bus(BusChannel::Left, pos, &mut self.bus_playback.0[..n]);
            self.tape
                .read_bus(BusChannel::Right, pos, &mut self.bus_playback.1[..n]);
            for i in 0..n {
                let g = self.bus_gain_scratch[i];
                self.print_buf.0[i] += self.bus_playback.0[i] * g;
                self.print_buf.1[i] += self.bus_playback.1[i] * g;
            }

            // The character chain, per channel, around one shared
            // flutter (REQ-402). This is the pre-master tap: no master
            // gain has touched any of it (REQ-406).
            self.bounce_chain_l.0.process(&mut self.print_buf.0[..n]);
            self.bounce_chain_r.0.process(&mut self.print_buf.1[..n]);
            self.bounce_flutter
                .process(&mut self.print_buf.0[..n], &mut self.print_buf.1[..n]);
            self.bounce_chain_l.1.process(&mut self.print_buf.0[..n]);
            self.bounce_chain_r.1.process(&mut self.print_buf.1[..n]);

            self.bounce_pass.write_block(
                &mut self.tape,
                &self.print_buf.0[..n],
                &self.print_buf.1[..n],
            );
            // Monitor the printed signal itself, through the bus's own
            // fader/mute like any other playback (REQ-408) - not the old
            // tape content, and not bypassing the fader.
            self.bus_playback.0[..n].copy_from_slice(&self.print_buf.0[..n]);
            self.bus_playback.1[..n].copy_from_slice(&self.print_buf.1[..n]);
        } else {
            // Ordinary playback: the bus contributes its stored content,
            // exactly as an idle track does (REQ-401).
            self.tape
                .read_bus(BusChannel::Left, pos, &mut self.bus_playback.0[..n]);
            self.tape
                .read_bus(BusChannel::Right, pos, &mut self.bus_playback.1[..n]);
        }

        // Phase 2, with the gain already ticked above.
        self.mixer.finish_mix(
            &mut out_l[..n],
            &mut out_r[..n],
            Some((&self.bus_playback.0[..n], &self.bus_playback.1[..n])),
            Some(&self.bus_gain_scratch[..n]),
        );
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

/// Per-pass seed: cassette seed mixed with the pass counter, so each pass
/// is decorrelated but the whole session stays reproducible (REQ-304).
/// Per-channel seed for a stereo pass (REQ-702): the ordinary pass
/// derivation with a channel term folded in, so L and R decorrelate.
/// Channel 0 is left, and is also what the single shared flutter
/// modulator seeds at - a fixed choice, not an implementation
/// coin-flip bit-reproducibility would silently depend on.
fn seed_for_channel(noise_seed: u64, pass: u64, channel: u64) -> u32 {
    seed_for(noise_seed, pass.wrapping_mul(2).wrapping_add(channel))
}

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
    fn arming_the_bus_and_a_track_are_mutually_exclusive() {
        // REQ-405, both directions.
        let dir = TempDir::new("bus-arm-exclusion");
        let mut e = Engine::create(&dir.0, 48_000, 1).unwrap();

        e.set_armed(0, true);
        e.set_armed(2, true);
        e.set_bus_armed(true);
        assert!(e.is_bus_armed());
        for t in 0..NUM_TRACKS {
            assert!(!e.is_armed(t), "arming the bus must clear track {t}");
        }

        e.set_armed(1, true);
        assert!(!e.is_bus_armed(), "arming a track must clear the bus's arm");
        assert!(e.is_armed(1));

        // Disarming is not exclusion: it clears only what it names.
        e.set_bus_armed(true);
        e.set_bus_armed(false);
        assert!(!e.is_bus_armed());
        assert!(!e.is_armed(1), "still cleared by the earlier bus arm");
    }

    #[test]
    fn bus_content_plays_back_at_its_own_fader_and_mute() {
        // REQ-401/409: the bus is part of ordinary playback, not only
        // audible while bouncing.
        let dir = TempDir::new("bus-playback");
        let mut e = Engine::create(&dir.0, 48_000, 1).unwrap();
        let tone = sine(440.0, -6.0, 24_000);
        let quantized: Vec<i16> = tone.iter().map(|&s| (s * 32767.0).round() as i16).collect();
        e.tape.write_bus_raw(BusChannel::Left, 0, &quantized);
        e.tape.write_bus_raw(BusChannel::Right, 0, &quantized);

        let measure = |e: &mut Engine| {
            e.seek(0);
            e.play();
            let out = run(e, 0, &silence(24_000), 512);
            e.stop();
            rms_dbfs(&out[4800..])
        };

        let unity = measure(&mut e);
        assert!(unity > -20.0, "bus must be audible in playback: {unity:.1}");

        e.mixer().set_bus_fader_db(-12.0);
        let cut = measure(&mut e);
        assert!(
            (unity - cut - 12.0).abs() < 1.0,
            "bus fader should cut ~12dB, got {:.1}",
            unity - cut
        );

        e.mixer().set_bus_muted(true);
        let muted = measure(&mut e);
        assert!(muted < -100.0, "muted bus must be silent: {muted:.1}");
    }

    #[test]
    fn bus_fader_and_mute_moves_do_not_click() {
        // REQ-409's smoothing clause, through the real engine path.
        let dir = TempDir::new("bus-ramp");
        let mut e = Engine::create(&dir.0, 48_000, 1).unwrap();
        let tone = sine(440.0, -6.0, 48_000);
        let quantized: Vec<i16> = tone.iter().map(|&s| (s * 32767.0).round() as i16).collect();
        e.tape.write_bus_raw(BusChannel::Left, 0, &quantized);
        e.tape.write_bus_raw(BusChannel::Right, 0, &quantized);

        e.seek(0);
        e.play();
        let quiet = silence(512);
        let inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
        let mut out = Vec::new();
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        for block in 0..80 {
            if block == 20 {
                e.mixer().set_bus_fader_db(-24.0);
            }
            if block == 50 {
                e.mixer().set_bus_muted(true);
            }
            let n = e.process_block(&inputs, &mut l, &mut r);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&l[..n]);
        }
        e.stop();
        porta_testkit::assert_no_clicks!(&out);
    }

    #[test]
    fn unused_spares_return_to_the_pool_so_short_takes_never_fall_back() {
        // Regression for a real bug found in review: an earlier version
        // of the chunk-pool fix only ever gave back the chunks a pass
        // *used*, never the ones it reserved and didn't - so a track's
        // reserve shrank by its whole per-pass share on every take,
        // regardless of length, and a handful of short takes (with no
        // Save/Undo in between - the only thing that would otherwise
        // replenish it) was enough to exhaust it. Ten short takes, well
        // under CHUNK_POOL_PER_TRACK each, must all stay allocation-free
        // on the strength of push_pass's immediate give-back alone.
        let dir = TempDir::new("spares-return");
        let mut e =
            Engine::create_with_character(&dir.0, 48_000 * 60 * 5, TapeCharacter::clean()).unwrap();
        for i in 0..10 {
            e.set_armed(0, true);
            e.record();
            run(&mut e, 0, &sine(440.0, -6.0, 4_800), 512); // 0.1s, far under one chunk
            e.stop();
            assert_eq!(
                e.pass_buffer_fallbacks(),
                0,
                "take {i} fell back to an on-thread allocation - the chunk reserve leaked"
            );
        }
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
    fn monitor_previews_live_input_while_armed_and_not_recording() {
        let dir = TempDir::new("monitor");
        let mut e = Engine::create_with_character(&dir.0, 96_000, TapeCharacter::clean()).unwrap();
        e.set_armed(0, true);
        e.play();
        let take = sine(1000.0, 0.0, 4800);

        // Armed but not recording, monitor off: plays the (blank) tape,
        // same as before this feature existed.
        run(&mut e, 0, &take, 512);
        assert!(
            e.track_level_db(0) < -100.0,
            "monitor off should stay silent, got {} dB",
            e.track_level_db(0)
        );

        // Monitor on: hears the live input without recording it.
        e.set_monitor(0, true);
        run(&mut e, 0, &take, 512);
        assert!(
            (e.track_level_db(0) - 0.0).abs() < 0.5,
            "monitor on should pass the live input through, got {} dB",
            e.track_level_db(0)
        );

        // A preview, not a take - nothing should have reached tape.
        e.stop();
        let mut on_tape = vec![0i16; 4800];
        e.tape().read_raw(0, 0, &mut on_tape);
        assert!(
            on_tape.iter().all(|&s| s == 0),
            "monitor preview must not write to tape"
        );
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

#[cfg(test)]
mod bounce_pass_tests {
    use super::*;
    use porta_dsp::character::TapeCharacter;
    use porta_testkit::meter::rms_dbfs;
    use porta_testkit::signal::{silence, sine};

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("porta-bounce-{name}"));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const LEN: usize = 48_000;

    /// A cassette with material on tracks 0/1 and a clean character, so
    /// assertions are about routing and summing rather than colour.
    fn seeded(dir: &std::path::Path, seed: u64) -> Engine {
        let character = TapeCharacter {
            noise_seed: seed,
            ..TapeCharacter::clean()
        };
        let mut e = Engine::create_with_character(dir, LEN, character).unwrap();
        for (t, freq) in [(0usize, 220.0f32), (1, 330.0)] {
            let tone = sine(freq, -9.0, 24_000);
            let q: Vec<i16> = tone.iter().map(|&s| (s * 32767.0).round() as i16).collect();
            e.tape.write_raw(t, 0, &q);
        }
        e
    }

    /// Roll a bounce over `samples` from position 0.
    fn bounce(e: &mut Engine, samples: usize) {
        e.seek(0);
        e.set_bus_armed(true);
        e.record();
        let quiet = silence(512);
        let inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        let mut done = 0;
        while done < samples {
            let want = 512.min(samples - done);
            let n = e.process_block(&inputs, &mut l[..want], &mut r[..want]);
            if n == 0 {
                break;
            }
            done += n;
        }
        e.stop();
    }

    fn bus_raw(e: &Engine, len: usize) -> (Vec<i16>, Vec<i16>) {
        let mut l = vec![0i16; len];
        let mut r = vec![0i16; len];
        e.tape.read_bus_raw(BusChannel::Left, 0, &mut l);
        e.tape.read_bus_raw(BusChannel::Right, 0, &mut r);
        (l, r)
    }

    #[test]
    fn a_bounce_prints_the_track_mix_and_is_reproducible() {
        let d1 = TempDir::new("print-a");
        let d2 = TempDir::new("print-b");
        let mut a = seeded(&d1.0, 7);
        let mut b = seeded(&d2.0, 7);
        // Same live fader/pan on both, different master positions: the
        // master must not reach tape (REQ-406).
        for e in [&mut a, &mut b] {
            e.mixer().set_fader_db(0, -3.0);
            e.mixer().set_pan(1, 0.6);
        }
        a.mixer().set_master_db(0.0);
        b.mixer().set_master_db(-9.0);

        bounce(&mut a, 12_000);
        bounce(&mut b, 12_000);

        let (al, ar) = bus_raw(&a, 12_000);
        let (bl, br) = bus_raw(&b, 12_000);
        assert!(
            al.iter().any(|&s| s != 0),
            "the bounce must actually print something"
        );
        assert_eq!(al, bl, "master position must not change what is printed");
        assert_eq!(ar, br, "master position must not change what is printed");
        // Panning track 1 right must make the channels differ - the
        // whole point of a stereo bus (REQ-401 vs the old mono sum).
        assert_ne!(al, ar, "a panned source must print a stereo image");
    }

    #[test]
    fn tracks_and_bus_never_disturb_each_other() {
        // REQ-306 in both directions.
        let dir = TempDir::new("isolation");
        let mut e = seeded(&dir.0, 3);
        let before: Vec<Vec<i16>> = (0..NUM_TRACKS)
            .map(|t| {
                let mut v = vec![0i16; 24_000];
                e.tape.read_raw(t, 0, &mut v);
                v
            })
            .collect();

        bounce(&mut e, 12_000);

        for (t, want) in before.iter().enumerate() {
            let mut got = vec![0i16; 24_000];
            e.tape.read_raw(t, 0, &mut got);
            assert_eq!(&got, want, "track {t} changed across a bounce");
        }

        // And an ordinary track pass leaves the bus alone.
        let bus_before = bus_raw(&e, 12_000);
        e.seek(0);
        e.set_armed(2, true);
        e.record();
        let tone = sine(440.0, -6.0, 4096);
        let quiet = silence(512);
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        for chunk in tone.chunks(512) {
            let mut inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
            inputs[2] = chunk;
            e.process_block(&inputs, &mut l[..chunk.len()], &mut r[..chunk.len()]);
        }
        e.stop();
        assert_eq!(
            bus_raw(&e, 12_000),
            bus_before,
            "the bus changed across an ordinary track pass"
        );
    }

    #[test]
    fn a_second_bounce_folds_the_first_one_forward() {
        // REQ-407: the bus's prior content is read before it is
        // overwritten, so bouncing again keeps what was already there
        // instead of replacing it.
        let dir = TempDir::new("fold-forward");
        let mut e = seeded(&dir.0, 11);
        bounce(&mut e, 12_000);
        let first = bus_raw(&e, 12_000);

        // Mute the tracks so the second pass can only print what the
        // bus already held - if prior content were ignored, this would
        // come back silent.
        for t in 0..NUM_TRACKS {
            e.mixer().set_muted(t, true);
        }
        bounce(&mut e, 12_000);
        let second = bus_raw(&e, 12_000);

        let energy = rms_dbfs(
            &second
                .0
                .iter()
                .map(|&s| f32::from(s) / 32768.0)
                .collect::<Vec<_>>(),
        );
        assert!(
            energy > -60.0,
            "the second bounce lost the first one's content: {energy:.1} dBFS"
        );
        assert_ne!(first, second, "a second generation must degrade, not clone");
    }

    #[test]
    fn one_undo_reverts_both_channels_of_a_bounce() {
        let dir = TempDir::new("bounce-undo");
        let mut e = seeded(&dir.0, 5);
        let before = bus_raw(&e, 12_000);
        bounce(&mut e, 12_000);
        let after = bus_raw(&e, 12_000);
        assert_ne!(after, before);

        e.undo().unwrap();
        assert_eq!(bus_raw(&e, 12_000), before, "one undo must revert both");
        e.redo().unwrap();
        assert_eq!(bus_raw(&e, 12_000), after, "one redo must restore both");
    }

    #[test]
    fn two_back_to_back_bounces_never_allocate_a_third_may() {
        // The double-buffered reserve's whole purpose, and the honest
        // boundary of the guarantee.
        let dir = TempDir::new("no-fallback");
        let mut e = seeded(&dir.0, 13);
        bounce(&mut e, 4096);
        bounce(&mut e, 4096);
        assert_eq!(
            e.pass_buffer_fallbacks(),
            0,
            "two bounces with nothing saved in between must come from the reserve"
        );
        bounce(&mut e, 4096);
        // A third is allowed to fall back - documented, not a bug.
        assert!(e.pass_buffer_fallbacks() <= 1);

        // After a save the reserve refills, so bouncing is clean again.
        e.save().unwrap();
        let baseline = e.pass_buffer_fallbacks();
        bounce(&mut e, 4096);
        assert_eq!(
            e.pass_buffer_fallbacks(),
            baseline,
            "a flush must return the reserve pairs"
        );
    }

    #[test]
    fn tracks_are_excluded_from_the_monitor_sum_but_still_metered() {
        // REQ-408. With the bus muted the audible output is exactly
        // silent while the meters stay live - the clean measurable form
        // of "excluded but metered".
        let dir = TempDir::new("monitor-exclusion");
        let mut e = seeded(&dir.0, 17);
        e.mixer().set_bus_muted(true);
        e.seek(0);
        e.set_bus_armed(true);
        e.record();

        let quiet = silence(512);
        let inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        let mut out = Vec::new();
        for _ in 0..8 {
            let n = e.process_block(&inputs, &mut l, &mut r);
            out.extend_from_slice(&l[..n]);
        }
        let level = rms_dbfs(&out);
        assert!(
            level < -100.0,
            "tracks must be excluded from the audible sum: {level:.1} dBFS"
        );
        assert!(
            e.track_level_db(0) > -40.0,
            "track 0's meter must stay live during a bounce, got {:.1}",
            e.track_level_db(0)
        );
        e.stop();
    }
}
