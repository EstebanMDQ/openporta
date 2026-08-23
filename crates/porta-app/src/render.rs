//! Offline mixdown and WAV writing (REQ-803). Rendering runs the engine
//! exactly as playback does, so an export is what the machine sounds
//! like, not a second code path that might drift from it.

use porta_engine::engine::Engine;
use porta_engine::NUM_TRACKS;
use std::path::Path;

const BLOCK: usize = 512;

/// A middle-of-the-road bitrate for the "share" format: clearly lossy
/// but not obviously so, small enough to actually be convenient to
/// share. Not exposed as a flag/UI control - WAV is the format meant
/// to be tuned (`--bits`), MP3 exists to be a one-click convenience.
const MP3_BITRATE_KBPS: u32 = 192;

#[derive(Debug, thiserror::Error)]
pub enum Mp3Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Encode(#[from] shine_rs::EncoderError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitDepth {
    Sixteen,
    TwentyFour,
}

impl BitDepth {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "16" => Some(Self::Sixteen),
            "24" => Some(Self::TwentyFour),
            _ => None,
        }
    }

    fn bits(self) -> u16 {
        match self {
            Self::Sixteen => 16,
            Self::TwentyFour => 24,
        }
    }
}

/// Play `samples` of tape from the current playhead and return the
/// stereo mix. Leaves the transport stopped.
pub fn mixdown(engine: &mut Engine, samples: usize) -> (Vec<f32>, Vec<f32>) {
    engine.play();
    let quiet = vec![0.0; BLOCK];
    let inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    let mut out_l = Vec::with_capacity(samples);
    let mut out_r = Vec::with_capacity(samples);
    let mut done = 0;
    while done < samples {
        let want = BLOCK.min(samples - done);
        let n = engine.process_block(&inputs, &mut l[..want], &mut r[..want]);
        if n == 0 {
            break;
        }
        out_l.extend_from_slice(&l[..n]);
        out_r.extend_from_slice(&r[..n]);
        done += n;
    }
    engine.stop();
    (out_l, out_r)
}

pub fn write_wav(
    path: impl AsRef<Path>,
    l: &[f32],
    r: &[f32],
    depth: BitDepth,
) -> Result<(), hound::Error> {
    if let Some(parent) = path.as_ref().parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: porta_engine::SAMPLE_RATE,
        bits_per_sample: depth.bits(),
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    let peak = match depth {
        BitDepth::Sixteen => 32767.0,
        BitDepth::TwentyFour => 8_388_607.0,
    };
    for i in 0..l.len().min(r.len()) {
        for &s in &[l[i], r[i]] {
            w.write_sample((s.clamp(-1.0, 1.0) * peak).round() as i32)?;
        }
    }
    w.finalize()
}

/// Same mixdown as `write_wav`, encoded to MP3 instead - a convenience
/// format to share, not the master (that's WAV: lossless, tunable bit
/// depth). Quantizes the same way `write_wav` does (round to i16, no
/// dither - matching what WAV export already does, not a new gap).
pub fn write_mp3(path: impl AsRef<Path>, l: &[f32], r: &[f32]) -> Result<(), Mp3Error> {
    if let Some(parent) = path.as_ref().parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let n = l.len().min(r.len());
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        pcm.push((l[i].clamp(-1.0, 1.0) * 32767.0).round() as i16);
        pcm.push((r[i].clamp(-1.0, 1.0) * 32767.0).round() as i16);
    }
    let config = shine_rs::Mp3EncoderConfig::new()
        .sample_rate(porta_engine::SAMPLE_RATE)
        .bitrate(MP3_BITRATE_KBPS)
        .channels(2)
        .stereo_mode(shine_rs::StereoMode::JointStereo);
    let mp3_bytes = shine_rs::encode_pcm_to_mp3(config, &pcm)?;
    std::fs::write(path, mp3_bytes)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Wav(#[from] hound::Error),
    #[error(
        "ffmpeg not found on PATH - video export shells out to it rather than \
         bundling a video encoder; install it (e.g. `brew install ffmpeg` or \
         `apt install ffmpeg`) and try again"
    )]
    FfmpegMissing,
    #[error("ffmpeg exited with an error:\n{0}")]
    FfmpegFailed(String),
}

/// Combine a single still `image` with the already-mixed audio (`l`/`r`)
/// into an MP4 - the standard "static image + audio" recipe most video
/// platforms, including YouTube, accept directly. Not a new video
/// encoder written for this: shells out to `ffmpeg`, an explicit,
/// checked-for external dependency (not bundled into releases), the
/// same reasoning that already applies to MP3 being a convenience
/// format on top of the WAV master.
///
/// Writes the mix to a temporary WAV next to `path` (ffmpeg needs a
/// real audio *file*, not in-process samples) and removes it
/// afterward, success or failure - `path` itself is the only output
/// this function is meant to leave behind.
pub fn write_video(
    path: impl AsRef<Path>,
    image: impl AsRef<Path>,
    l: &[f32],
    r: &[f32],
) -> Result<(), VideoError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let temp_wav = path.with_extension("tmp.wav");
    write_wav(&temp_wav, l, r, BitDepth::Sixteen)?;

    let result = run_ffmpeg(&temp_wav, image.as_ref(), path);
    let _ = std::fs::remove_file(&temp_wav);
    result
}

fn run_ffmpeg(audio: &Path, image: &Path, out: &Path) -> Result<(), VideoError> {
    use std::process::Command;
    let output = Command::new("ffmpeg")
        .args(["-y", "-loop", "1", "-i"])
        .arg(image)
        .args(["-i"])
        .arg(audio)
        .args([
            "-c:v",
            "libx264",
            "-tune",
            "stillimage",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-shortest",
        ])
        .arg(out)
        .output();
    match output {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(VideoError::FfmpegFailed(
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(VideoError::FfmpegMissing),
        Err(e) => Err(VideoError::Io(e)),
    }
}

pub fn read_wav_mono(path: impl AsRef<Path>) -> Result<Vec<f32>, hound::Error> {
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
