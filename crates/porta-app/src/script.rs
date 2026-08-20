//! Session scripts (REQ-804): a JSON op list that drives the engine
//! headlessly. This is how integration tests exercise the machine and how
//! audition renders are produced without touching audio hardware.

use porta_engine::engine::{Engine, EngineError};
use porta_engine::NUM_TRACKS;
use std::path::{Path, PathBuf};

/// Block size the runner feeds the engine. Results are block-size
/// invariant (REQ-203), so this is a throughput choice, not a sonic one.
const BLOCK: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("script io: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad script: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("wav: {0}")]
    Wav(#[from] hound::Error),
    #[error("track {0} is out of range")]
    BadTrack(usize),
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Create a new cassette. `minutes` defaults to 15.
    New {
        dir: String,
        #[serde(default)]
        minutes: Option<f32>,
        #[serde(default)]
        seed: Option<u64>,
    },
    Open {
        dir: String,
    },
    Seek {
        seconds: f32,
    },
    Arm {
        track: usize,
        #[serde(default = "yes")]
        on: bool,
    },
    Fader {
        track: usize,
        db: f32,
    },
    Pan {
        track: usize,
        value: f32,
    },
    Master {
        db: f32,
    },
    /// Record `input_wav` onto the armed tracks from the playhead.
    Record {
        input_wav: String,
    },
    /// Play `seconds` of tape (or to the tape end).
    Play {
        seconds: f32,
    },
    Undo,
    Redo,
    Save,
    /// Write the stereo mixdown captured since the last export to a WAV.
    Export {
        out: String,
    },
}

fn yes() -> bool {
    true
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Script {
    pub ops: Vec<Op>,
}

pub struct Runner {
    engine: Option<Engine>,
    base: PathBuf,
    /// Stereo output captured since the last export.
    captured: (Vec<f32>, Vec<f32>),
}

impl Runner {
    /// `base` is the directory relative paths in the script resolve against.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self {
            engine: None,
            base: base.into(),
            captured: (Vec::new(), Vec::new()),
        }
    }

    pub fn run_file(&mut self, path: impl AsRef<Path>) -> Result<(), ScriptError> {
        let text = std::fs::read_to_string(path)?;
        let script: Script = serde_json::from_str(&text)?;
        self.run(&script)
    }

    pub fn run(&mut self, script: &Script) -> Result<(), ScriptError> {
        for op in &script.ops {
            self.exec(op)?;
        }
        Ok(())
    }

    fn path(&self, p: &str) -> PathBuf {
        self.base.join(p)
    }

    fn engine(&mut self) -> Result<&mut Engine, ScriptError> {
        self.engine
            .as_mut()
            .ok_or(ScriptError::Engine(EngineError::NotStopped("no cassette")))
    }

    fn check_track(track: usize) -> Result<(), ScriptError> {
        (track < NUM_TRACKS)
            .then_some(())
            .ok_or(ScriptError::BadTrack(track))
    }

    fn exec(&mut self, op: &Op) -> Result<(), ScriptError> {
        match op {
            Op::New { dir, minutes, seed } => {
                let len =
                    (porta_engine::SAMPLE_RATE as f32 * 60.0 * minutes.unwrap_or(15.0)) as usize;
                self.engine = Some(Engine::create(self.path(dir), len, seed.unwrap_or(0))?);
            }
            Op::Open { dir } => {
                self.engine = Some(Engine::open(self.path(dir))?);
            }
            Op::Seek { seconds } => {
                let pos = (porta_engine::SAMPLE_RATE as f32 * seconds) as usize;
                self.engine()?.seek(pos);
            }
            Op::Arm { track, on } => {
                Self::check_track(*track)?;
                self.engine()?.set_armed(*track, *on);
            }
            Op::Fader { track, db } => {
                Self::check_track(*track)?;
                self.engine()?.mixer().set_fader_db(*track, *db);
            }
            Op::Pan { track, value } => {
                Self::check_track(*track)?;
                self.engine()?.mixer().set_pan(*track, *value);
            }
            Op::Master { db } => {
                self.engine()?.mixer().set_master_db(*db);
            }
            Op::Record { input_wav } => {
                let input = read_wav_mono(self.path(input_wav))?;
                self.record(&input)?;
            }
            Op::Play { seconds } => {
                let n = (porta_engine::SAMPLE_RATE as f32 * seconds) as usize;
                self.play(n)?;
            }
            Op::Undo => self.engine()?.undo()?,
            Op::Redo => self.engine()?.redo()?,
            Op::Save => self.engine()?.save()?,
            Op::Export { out } => {
                let path = self.path(out);
                write_wav_stereo(&path, &self.captured.0, &self.captured.1)?;
                self.captured.0.clear();
                self.captured.1.clear();
            }
        }
        Ok(())
    }

    fn record(&mut self, input: &[f32]) -> Result<(), ScriptError> {
        let engine = self
            .engine
            .as_mut()
            .ok_or(ScriptError::Engine(EngineError::NotStopped("no cassette")))?;
        engine.record();
        let silence = vec![0.0; BLOCK];
        let mut l = vec![0.0; BLOCK];
        let mut r = vec![0.0; BLOCK];
        let mut fed = 0;
        while fed < input.len() {
            let end = (fed + BLOCK).min(input.len());
            let chunk = &input[fed..end];
            let inputs: [&[f32]; NUM_TRACKS] = std::array::from_fn(|t| {
                if engine.is_armed(t) {
                    chunk
                } else {
                    &silence[..chunk.len()]
                }
            });
            let n = engine.process_block(&inputs, &mut l[..chunk.len()], &mut r[..chunk.len()]);
            if n == 0 {
                break;
            }
            self.captured.0.extend_from_slice(&l[..n]);
            self.captured.1.extend_from_slice(&r[..n]);
            fed = end;
        }
        engine.stop();
        Ok(())
    }

    fn play(&mut self, samples: usize) -> Result<(), ScriptError> {
        let engine = self
            .engine
            .as_mut()
            .ok_or(ScriptError::Engine(EngineError::NotStopped("no cassette")))?;
        engine.play();
        let silence = vec![0.0; BLOCK];
        let inputs: [&[f32]; NUM_TRACKS] = [&silence, &silence, &silence, &silence];
        let mut l = vec![0.0; BLOCK];
        let mut r = vec![0.0; BLOCK];
        let mut done = 0;
        while done < samples {
            let want = BLOCK.min(samples - done);
            let n = engine.process_block(&inputs, &mut l[..want], &mut r[..want]);
            if n == 0 {
                break;
            }
            self.captured.0.extend_from_slice(&l[..n]);
            self.captured.1.extend_from_slice(&r[..n]);
            done += n;
        }
        engine.stop();
        Ok(())
    }
}

pub fn read_wav_mono(path: impl AsRef<Path>) -> Result<Vec<f32>, ScriptError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()?
        }
    };
    Ok(raw
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect())
}

pub fn write_wav_stereo(path: impl AsRef<Path>, l: &[f32], r: &[f32]) -> Result<(), ScriptError> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: porta_engine::SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for i in 0..l.len().min(r.len()) {
        for &s in &[l[i], r[i]] {
            w.write_sample((s.clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
        }
    }
    w.finalize()?;
    Ok(())
}
