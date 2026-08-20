//! Realtime audio adapter (cpal). Feature-gated: default builds and CI
//! never see it, because it needs an audio device to do anything.
//!
//! The division of labour is deliberate. Everything that decides what
//! the machine sounds like lives in `porta-engine` and is tested
//! headlessly; this file only moves bytes, and obeys three rules:
//!
//! 1. Neither callback allocates, locks, or touches the filesystem
//!    (REQ-902). Buffers are sized once at startup, commands arrive
//!    through wait-free queues, and blocking commands are bounced back
//!    to the control thread.
//! 2. The output callback splits its buffer at command boundaries, so a
//!    command takes effect on the sample it was scheduled for whatever
//!    period the device hands us. See `realtime_sim.rs` for why: apply
//!    commands only at period boundaries and the same session renders
//!    differently on different hardware.
//! 3. Blocking work (bounce, undo, save) happens on the control thread
//!    with the transport stopped, never in a callback.
//!
//! Input and output are separate cpal streams (there is no duplex API),
//! joined by a wait-free ring. The two clocks are not sample-locked, so
//! the ring absorbs the drift: an empty ring reads as silence and a full
//! one drops, both counted as xruns rather than papered over.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, StreamConfig};
use porta_engine::command::{Command, EngineEvent};
use porta_engine::engine::Engine;
use porta_engine::NUM_TRACKS;
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Command/event queue depth. Generous: these are human-scale events.
const QUEUE: usize = 256;
/// Largest period we size buffers for.
const MAX_PERIOD: usize = 4096;
/// Input ring depth, in samples. About 85ms at 48kHz: enough to ride
/// out clock drift between the two devices without adding real latency.
const INPUT_RING: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum RealtimeError {
    #[error("no output device available")]
    NoOutputDevice,
    #[error("device '{0}' does not support 48kHz")]
    UnsupportedRate(String),
    #[error(transparent)]
    Cpal(#[from] cpal::Error),
}

/// List devices, so a support problem is one command away from being
/// diagnosed rather than a mystery.
pub fn list_devices() -> Result<Vec<String>, RealtimeError> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    for d in host.output_devices()? {
        let name = d.to_string();
        let rates: Vec<String> = d
            .supported_output_configs()?
            .map(|c| {
                format!(
                    "{}-{}Hz/{}ch",
                    c.min_sample_rate(),
                    c.max_sample_rate(),
                    c.channels()
                )
            })
            .collect();
        out.push(format!("output  {name} [{}]", rates.join(", ")));
    }
    for d in host.input_devices()? {
        out.push(format!("input   {d}"));
    }
    Ok(out)
}

fn pick(devices: impl Iterator<Item = cpal::Device>, wanted: Option<&str>) -> Option<cpal::Device> {
    let want = wanted?.to_lowercase();
    devices
        .into_iter()
        .find(|d| d.to_string().to_lowercase().contains(&want))
}

fn supports_48k(device: &cpal::Device) -> Result<cpal::SupportedStreamConfigRange, RealtimeError> {
    device
        .supported_output_configs()?
        .find(|c| {
            c.min_sample_rate() <= porta_engine::SAMPLE_RATE
                && c.max_sample_rate() >= porta_engine::SAMPLE_RATE
        })
        .ok_or_else(|| RealtimeError::UnsupportedRate(device.to_string()))
}

/// Counters the control thread can read without disturbing audio.
#[derive(Default)]
pub struct Xruns {
    /// Output callback could not be served (oversized period).
    pub output: AtomicU64,
    /// Input ring ran dry: the engine recorded silence it should not
    /// have.
    pub starved: AtomicU64,
    /// Input ring overflowed: captured audio was dropped.
    pub dropped: AtomicU64,
}

/// A running session. Dropping this stops the audio.
pub struct RealtimeSession {
    _input: Option<cpal::Stream>,
    _output: cpal::Stream,
    commands: Producer<Command>,
    events: Consumer<EngineEvent>,
    pub xruns: Arc<Xruns>,
    pub period: usize,
    pub input_device: Option<String>,
    pub output_device: String,
}

impl RealtimeSession {
    /// Queue a non-blocking command for the audio thread. Blocking
    /// commands are refused: run those on the control thread with the
    /// transport stopped.
    pub fn send(&mut self, command: Command) -> Result<(), Command> {
        if command.is_blocking() {
            return Err(command);
        }
        self.commands
            .push(command)
            .map_err(|rtrb::PushError::Full(c)| c)
    }

    /// Drain what the audio thread has reported since last time.
    pub fn poll(&mut self) -> Vec<EngineEvent> {
        let mut out = Vec::new();
        while let Ok(e) = self.events.pop() {
            out.push(e);
        }
        out
    }

    pub fn xrun_summary(&self) -> String {
        format!(
            "output {}, starved {}, dropped {}",
            self.xruns.output.load(Ordering::Relaxed),
            self.xruns.starved.load(Ordering::Relaxed),
            self.xruns.dropped.load(Ordering::Relaxed),
        )
    }
}

/// Start playback (and capture, if an input device is available).
/// `input_name`/`output_name` are substring matches against device
/// names; `None` means the system default. `period` is a hint - the
/// device decides, and some hosts ignore it entirely.
pub fn start(
    mut engine: Engine,
    input_name: Option<&str>,
    output_name: Option<&str>,
    period: Option<usize>,
) -> Result<RealtimeSession, RealtimeError> {
    let host = cpal::default_host();
    let output = pick(host.output_devices()?, output_name)
        .or_else(|| host.default_output_device())
        .ok_or(RealtimeError::NoOutputDevice)?;
    let supported = supports_48k(&output)?;
    let out_channels = supported.channels() as usize;
    let output_device = output.to_string();

    let period = period.unwrap_or(256).clamp(16, MAX_PERIOD);
    let out_config = StreamConfig {
        channels: supported.channels(),
        sample_rate: porta_engine::SAMPLE_RATE,
        buffer_size: BufferSize::Fixed(period as u32),
    };

    let (command_tx, mut command_rx) = RingBuffer::<Command>::new(QUEUE);
    let (mut event_tx, event_rx) = RingBuffer::<EngineEvent>::new(QUEUE);
    let (mut capture_tx, mut capture_rx) = RingBuffer::<f32>::new(INPUT_RING);
    let xruns = Arc::new(Xruns::default());

    // Optional capture stream. Its only job is to hand mono samples to
    // the output callback.
    let input_device =
        pick(host.input_devices()?, input_name).or_else(|| host.default_input_device());
    let (input_stream, input_name_used) = match input_device {
        None => (None, None),
        Some(device) => {
            let name = device.to_string();
            let in_config = StreamConfig {
                channels: 1,
                sample_rate: porta_engine::SAMPLE_RATE,
                buffer_size: BufferSize::Fixed(period as u32),
            };
            let counters = Arc::clone(&xruns);
            let stream = device.build_input_stream(
                in_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    for &s in data {
                        if capture_tx.push(s).is_err() {
                            counters.dropped.fetch_add(1, Ordering::Relaxed);
                            break;
                        }
                    }
                },
                move |err| eprintln!("audio input error: {err}"),
                None,
            )?;
            stream.play()?;
            (Some(stream), Some(name))
        }
    };

    // Everything the output callback touches is allocated here, before
    // the stream starts (REQ-902).
    let mut captured = vec![0.0f32; MAX_PERIOD];
    let mut mix_l = vec![0.0f32; MAX_PERIOD];
    let mut mix_r = vec![0.0f32; MAX_PERIOD];
    let mut last_state = engine.state();
    let counters = Arc::clone(&xruns);

    let stream = output.build_output_stream(
        out_config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            data.fill(0.0);
            let frames = data.len() / out_channels;
            if frames > MAX_PERIOD {
                // Bigger than we sized for; stay silent rather than
                // allocate in the callback.
                counters.output.fetch_add(1, Ordering::Relaxed);
                let _ = event_tx.push(EngineEvent::Xrun {
                    total: counters.output.load(Ordering::Relaxed),
                });
                return;
            }

            let mut starved = 0u64;
            for slot in captured[..frames].iter_mut() {
                match capture_rx.pop() {
                    Ok(s) => *slot = s,
                    Err(_) => {
                        *slot = 0.0;
                        starved += 1;
                    }
                }
            }
            if starved > 0 {
                counters.starved.fetch_add(starved, Ordering::Relaxed);
            }

            let mut done = 0usize;
            while done < frames {
                // Rule 2: one command, then process only up to the next.
                if let Ok(cmd) = command_rx.pop() {
                    if cmd.is_blocking() {
                        let _ = event_tx.push(EngineEvent::Rejected(cmd));
                    } else {
                        let _ = porta_engine::command::apply(&mut engine, cmd);
                    }
                }

                let want = frames - done;
                let slice = &captured[done..done + want];
                let inputs: [&[f32]; NUM_TRACKS] = [slice; NUM_TRACKS];
                let n = engine.process_block(&inputs, &mut mix_l[..want], &mut mix_r[..want]);
                if n == 0 {
                    // Transport parked: the rest of the buffer stays
                    // silent. Drain any remaining commands so a burst
                    // does not trickle out one per callback.
                    while let Ok(cmd) = command_rx.pop() {
                        if cmd.is_blocking() {
                            let _ = event_tx.push(EngineEvent::Rejected(cmd));
                        } else {
                            let _ = porta_engine::command::apply(&mut engine, cmd);
                        }
                    }
                    break;
                }
                for f in 0..n {
                    let frame = &mut data[(done + f) * out_channels..][..out_channels];
                    frame[0] = mix_l[f];
                    if out_channels > 1 {
                        frame[1] = mix_r[f];
                    }
                }
                done += n;
            }

            if engine.state() != last_state {
                last_state = engine.state();
                let _ = event_tx.push(EngineEvent::State(last_state));
            }
            let _ = event_tx.push(EngineEvent::Playhead {
                sample: engine.playhead(),
            });
        },
        move |err| eprintln!("audio output error: {err}"),
        None,
    )?;
    stream.play()?;

    Ok(RealtimeSession {
        _input: input_stream,
        _output: stream,
        commands: command_tx,
        events: event_rx,
        xruns,
        period,
        input_device: input_name_used,
        output_device,
    })
}
