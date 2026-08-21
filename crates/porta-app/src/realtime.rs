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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    #[error("no input device available")]
    NoInputDevice,
    #[error("device '{0}' does not support 48kHz")]
    UnsupportedRate(String),
    #[error(transparent)]
    Cpal(#[from] cpal::Error),
    #[error("audio thread did not hand the engine back in time")]
    ShutdownTimeout,
}

/// `start`'s failure. Negotiation failures (bad device name, no
/// device, unsupported rate, ...) hand `engine` back so a failed
/// connect attempt never silently loses whatever it held. The rare
/// failure after engine has already moved into the output callback (an
/// actual stream-build error, not a naming mistake) cannot recover it
/// - cpal owns and drops the closure along with everything it captured.
pub enum StartError {
    // The recovered engine is only read back out by ui.rs's connect
    // flow (M5.5) - cmd_live just wants the message. Unused, not dead,
    // when built with realtime alone.
    #[cfg_attr(not(feature = "ui"), allow(dead_code))]
    Negotiation(Box<Engine>, RealtimeError),
    StreamBuild(RealtimeError),
}

impl std::fmt::Debug for StartError {
    // Engine has no Debug impl (and printing its contents wouldn't be
    // useful here anyway) - show the reason only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StartError({})", self.reason())
    }
}

impl StartError {
    pub fn reason(&self) -> &RealtimeError {
        match self {
            StartError::Negotiation(_, e) | StartError::StreamBuild(e) => e,
        }
    }

    /// The engine back, if this failure happened early enough to still
    /// have it.
    #[cfg_attr(not(feature = "ui"), allow(dead_code))]
    pub fn into_engine(self) -> Option<Engine> {
        match self {
            StartError::Negotiation(engine, _) => Some(*engine),
            StartError::StreamBuild(_) => None,
        }
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.reason())
    }
}

impl std::error::Error for StartError {}

/// One-shot exchange used only at shutdown: the audio thread owns the
/// engine outright for the life of the stream (REQ-902 forbids sharing
/// it under a lock), so the only way to run a blocking command like
/// Save is to stop the stream and hand the engine back to whoever asked
/// for it. `quit` is a plain atomic so the audio side can check it every
/// callback for free; the engine itself crosses on the wait-free ring
/// already used for commands and events.
///
/// Split from `RealtimeSession`/the cpal closure so the handoff protocol
/// itself is unit-testable without real hardware.
struct HandoffAudioSide<T> {
    quit: Arc<AtomicBool>,
    tx: Producer<T>,
}

impl<T> HandoffAudioSide<T> {
    /// Call once per callback. If a handoff was requested, moves the
    /// payload out of `slot` and returns true: the caller must not touch
    /// it again and should fill silence from now on.
    fn maybe_handoff(&mut self, slot: &mut Option<T>) -> bool {
        if !self.quit.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(payload) = slot.take() {
            let _ = self.tx.push(payload);
        }
        true
    }
}

struct HandoffControlSide<T> {
    quit: Arc<AtomicBool>,
    rx: Consumer<T>,
}

impl<T> HandoffControlSide<T> {
    /// Signal the audio side and block (briefly - this runs on the
    /// control thread, never the callback) until it hands the payload
    /// back or `timeout` elapses.
    fn request(&mut self, timeout: Duration) -> Result<T, RealtimeError> {
        self.quit.store(true, Ordering::Relaxed);
        let start = Instant::now();
        loop {
            if let Ok(payload) = self.rx.pop() {
                return Ok(payload);
            }
            if start.elapsed() > timeout {
                return Err(RealtimeError::ShutdownTimeout);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod handoff_tests {
    use super::*;

    #[test]
    fn shutdown_hands_the_payload_back() {
        let quit = Arc::new(AtomicBool::new(false));
        let (tx, rx) = RingBuffer::<u64>::new(1);
        let mut audio = HandoffAudioSide {
            quit: Arc::clone(&quit),
            tx,
        };
        let mut control = HandoffControlSide { quit, rx };

        let handle = std::thread::spawn(move || {
            let mut slot = Some(42u64);
            loop {
                if audio.maybe_handoff(&mut slot) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });

        let got = control.request(Duration::from_secs(2)).expect("handoff");
        assert_eq!(got, 42);
        handle.join().unwrap();
    }

    #[test]
    fn no_handoff_without_a_request() {
        let quit = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = RingBuffer::<u64>::new(1);
        let mut audio = HandoffAudioSide { quit, tx };
        let mut slot = Some(7u64);
        assert!(!audio.maybe_handoff(&mut slot));
        assert_eq!(slot, Some(7));
        assert!(rx.pop().is_err());
    }

    #[test]
    fn timeout_when_the_audio_side_never_answers() {
        let quit = Arc::new(AtomicBool::new(false));
        let (_tx, rx) = RingBuffer::<u64>::new(1);
        let mut control = HandoffControlSide { quit, rx };
        assert!(matches!(
            control.request(Duration::from_millis(20)),
            Err(RealtimeError::ShutdownTimeout)
        ));
    }
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

/// Print a live peak-level meter per input channel until the user hits
/// enter. For working out which physical jack lands on which channel
/// index on interfaces that don't order them the way you'd guess (the
/// L6, for one) - play into one input at a time and watch which column
/// moves, instead of a slow record/render round trip per guess.
pub fn probe_input(input_name: Option<&str>) -> Result<(), RealtimeError> {
    let host = cpal::default_host();
    let device = pick(host.input_devices()?, input_name)
        .or_else(|| host.default_input_device())
        .ok_or(RealtimeError::NoInputDevice)?;
    let name = device.to_string();
    let channels = max_input_channels(&device)?;
    let config = StreamConfig {
        channels,
        sample_rate: porta_engine::SAMPLE_RATE,
        buffer_size: BufferSize::Fixed(256),
    };

    // Bit-pattern max on the abs value; f32's bit ordering matches
    // numeric ordering for finite non-negative values.
    let peaks: Arc<Vec<AtomicU32>> = Arc::new((0..channels).map(|_| AtomicU32::new(0)).collect());
    let peaks_cb = Arc::clone(&peaks);
    let stride = channels as usize;
    let stream = device.build_input_stream(
        config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for frame in data.chunks_exact(stride) {
                for (i, &s) in frame.iter().enumerate() {
                    peaks_cb[i].fetch_max(s.abs().to_bits(), Ordering::Relaxed);
                }
            }
        },
        move |err| eprintln!("audio input error: {err}"),
        None,
    )?;
    stream.play()?;

    println!("probing {name} ({channels} channels) - play into one input at a time.");
    println!("press enter to quit.");

    let stop = Arc::new(AtomicBool::new(false));
    let stop_printer = Arc::clone(&stop);
    let printer = std::thread::spawn(move || {
        while !stop_printer.load(Ordering::Relaxed) {
            let bars: Vec<String> = peaks
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let peak = f32::from_bits(p.swap(0, Ordering::Relaxed));
                    let db = if peak > 0.0 {
                        (20.0 * peak.log10()).max(-60.0)
                    } else {
                        -60.0
                    };
                    format!("ch{}: {db:>5.0}dB", i + 1)
                })
                .collect();
            print!("\r{}   ", bars.join("  "));
            let _ = std::io::Write::flush(&mut std::io::stdout());
            std::thread::sleep(Duration::from_millis(150));
        }
    });

    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    stop.store(true, Ordering::Relaxed);
    let _ = printer.join();
    println!();
    Ok(())
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

/// Widest channel count the device offers at 48kHz. Interfaces like the
/// Zoom L6 expose their inputs as one multi-channel device (main mix on
/// channels 1-2, per-track sends on 3-6), so we need to know how many
/// channels we can actually ask cpal for before deciding how many of
/// them we can route to tracks.
fn max_input_channels(device: &cpal::Device) -> Result<u16, RealtimeError> {
    device
        .supported_input_configs()?
        .filter(|c| {
            c.min_sample_rate() <= porta_engine::SAMPLE_RATE
                && c.max_sample_rate() >= porta_engine::SAMPLE_RATE
        })
        .map(|c| c.channels())
        .max()
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

/// A running session. Dropping this stops the audio without saving -
/// call `shutdown` to get the engine back first.
pub struct RealtimeSession {
    _input: Option<cpal::Stream>,
    _output: cpal::Stream,
    commands: Producer<Command>,
    events: Consumer<EngineEvent>,
    engine_handoff: HandoffControlSide<Engine>,
    pub xruns: Arc<Xruns>,
    pub period: usize,
    pub input_device: Option<String>,
    pub output_device: String,
    /// How many tracks actually have a distinct input channel wired up
    /// (0 with no input device, otherwise up to NUM_TRACKS - fewer if
    /// the device doesn't have channel_offset + NUM_TRACKS channels).
    /// Tracks beyond this record silence rather than a duplicate signal.
    pub input_tracks: usize,
    pub input_channel_offset: usize,
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

    /// Stop the audio thread and take the engine back so the caller can
    /// run blocking commands (Save, Bounce, Undo, Redo) on it safely.
    /// `self` is consumed: the streams tear down once this returns,
    /// which only happens after the audio side has confirmed the
    /// handoff, so the callback never gets called again once it's given
    /// up the engine.
    pub fn shutdown(mut self) -> Result<Engine, RealtimeError> {
        self.engine_handoff.request(Duration::from_secs(2))
    }
}

/// Everything `start` needs to set up before it's safe to touch the
/// engine at all - device negotiation, config, queues, the optional
/// input stream. Split out so a failure here (bad device name, no
/// device found, unsupported rate, ...) never has to consume the
/// engine to report it: `start` only moves `engine` in after this
/// succeeds, so a failed connect attempt hands it straight back
/// instead of silently dropping whatever it held.
struct Negotiated {
    output: cpal::Device,
    out_config: StreamConfig,
    out_channels: usize,
    output_device: String,
    period: usize,
    command_tx: Producer<Command>,
    command_rx: Consumer<Command>,
    event_tx: Producer<EngineEvent>,
    event_rx: Consumer<EngineEvent>,
    xruns: Arc<Xruns>,
    engine_handoff_audio: HandoffAudioSide<Engine>,
    engine_handoff_control: HandoffControlSide<Engine>,
    input_stream: Option<cpal::Stream>,
    input_name_used: Option<String>,
    track_capture_rx: Vec<Consumer<f32>>,
}

fn negotiate(
    input_name: Option<&str>,
    output_name: Option<&str>,
    period: Option<usize>,
    channel_offset: usize,
) -> Result<Negotiated, RealtimeError> {
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

    let (command_tx, command_rx) = RingBuffer::<Command>::new(QUEUE);
    let (event_tx, event_rx) = RingBuffer::<EngineEvent>::new(QUEUE);
    let xruns = Arc::new(Xruns::default());

    // Shutdown-only handoff (see HandoffAudioSide doc comment): the only
    // way anything outside the callback ever touches the engine again.
    let quit = Arc::new(AtomicBool::new(false));
    let (engine_tx, engine_rx) = RingBuffer::<Engine>::new(1);
    let engine_handoff_audio = HandoffAudioSide {
        quit: Arc::clone(&quit),
        tx: engine_tx,
    };
    let engine_handoff_control = HandoffControlSide {
        quit,
        rx: engine_rx,
    };

    // Optional capture stream: one ring per track, fed from a distinct
    // device channel starting at channel_offset. Tracks beyond however
    // many channels the device actually has get no ring at all and
    // record silence rather than a duplicate of another track's input.
    let input_device =
        pick(host.input_devices()?, input_name).or_else(|| host.default_input_device());
    let (input_stream, input_name_used, track_capture_rx) = match input_device {
        None => (None, None, Vec::new()),
        Some(device) => {
            let name = device.to_string();
            let max_channels = max_input_channels(&device)?;
            let wanted = (channel_offset as u16).saturating_add(NUM_TRACKS as u16);
            let total_channels = max_channels.min(wanted).max(1);
            let active_tracks = (total_channels as usize)
                .saturating_sub(channel_offset)
                .min(NUM_TRACKS);
            let in_config = StreamConfig {
                channels: total_channels,
                sample_rate: porta_engine::SAMPLE_RATE,
                buffer_size: BufferSize::Fixed(period as u32),
            };
            let mut capture_tx = Vec::with_capacity(active_tracks);
            let mut capture_rx = Vec::with_capacity(active_tracks);
            for _ in 0..active_tracks {
                let (tx, rx) = RingBuffer::<f32>::new(INPUT_RING);
                capture_tx.push(tx);
                capture_rx.push(rx);
            }
            let counters = Arc::clone(&xruns);
            let frame_stride = total_channels as usize;
            let stream = device.build_input_stream(
                in_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    for frame in data.chunks_exact(frame_stride) {
                        for (t, tx) in capture_tx.iter_mut().enumerate() {
                            if tx.push(frame[channel_offset + t]).is_err() {
                                counters.dropped.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                },
                move |err| eprintln!("audio input error: {err}"),
                None,
            )?;
            stream.play()?;
            (Some(stream), Some(name), capture_rx)
        }
    };

    Ok(Negotiated {
        output,
        out_config,
        out_channels,
        output_device,
        period,
        command_tx,
        command_rx,
        event_tx,
        event_rx,
        xruns,
        engine_handoff_audio,
        engine_handoff_control,
        input_stream,
        input_name_used,
        track_capture_rx,
    })
}

/// Start playback (and capture, if an input device is available).
/// `input_name`/`output_name` are substring matches against device
/// names; `None` means the system default. `period` is a hint - the
/// device decides, and some hosts ignore it entirely. `channel_offset`
/// skips that many leading input channels before assigning the rest to
/// tracks 1..NUM_TRACKS in order - e.g. 2 on a Zoom L6, whose channels 1
/// and 2 carry its own main mix rather than a per-track send.
///
/// On failure `engine` comes back with the error rather than being
/// dropped, so a failed connect attempt from a UI never silently loses
/// whatever the engine held. One narrow gap remains: if the platform
/// call inside `build_output_stream` itself fails (not device
/// negotiation - an actual stream-build error), the engine has already
/// moved into that closure and cpal drops it with the closure. Rare in
/// practice; negotiation failures (bad name, no device, unsupported
/// rate) are what a mistyped setting actually produces, and those are
/// all caught before engine moves anywhere.
pub fn start(
    engine: Engine,
    input_name: Option<&str>,
    output_name: Option<&str>,
    period: Option<usize>,
    channel_offset: usize,
) -> Result<RealtimeSession, StartError> {
    let Negotiated {
        output,
        out_config,
        out_channels,
        output_device,
        period,
        command_tx,
        mut command_rx,
        mut event_tx,
        event_rx,
        xruns,
        mut engine_handoff_audio,
        engine_handoff_control,
        input_stream,
        input_name_used,
        mut track_capture_rx,
    } = match negotiate(input_name, output_name, period, channel_offset) {
        Ok(n) => n,
        Err(e) => return Err(StartError::Negotiation(Box::new(engine), e)),
    };
    let input_tracks = track_capture_rx.len();

    // Everything the output callback touches is allocated here, before
    // the stream starts (REQ-902).
    let mut captured: Vec<Vec<f32>> = vec![vec![0.0f32; MAX_PERIOD]; NUM_TRACKS];
    let mut mix_l = vec![0.0f32; MAX_PERIOD];
    let mut mix_r = vec![0.0f32; MAX_PERIOD];
    let mut last_state = engine.state();
    let mut engine_slot = Some(engine);
    let counters = Arc::clone(&xruns);

    let stream = output
        .build_output_stream(
            out_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                data.fill(0.0);
                if engine_handoff_audio.maybe_handoff(&mut engine_slot) {
                    return;
                }
                let engine = engine_slot.as_mut().expect("handed off, not silenced");
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
                for (t, rx) in track_capture_rx.iter_mut().enumerate() {
                    for slot in captured[t][..frames].iter_mut() {
                        match rx.pop() {
                            Ok(s) => *slot = s,
                            Err(_) => {
                                *slot = 0.0;
                                starved += 1;
                            }
                        }
                    }
                }
                for row in captured[track_capture_rx.len()..].iter_mut() {
                    // No device channel wired to this track: silence, not a
                    // duplicate of another track's input.
                    row[..frames].fill(0.0);
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
                            let _ = porta_engine::command::apply(&mut *engine, cmd);
                        }
                    }

                    let want = frames - done;
                    let inputs: [&[f32]; NUM_TRACKS] =
                        std::array::from_fn(|t| &captured[t][done..done + want]);
                    let n = engine.process_block(&inputs, &mut mix_l[..want], &mut mix_r[..want]);
                    if n == 0 {
                        // Transport parked: the rest of the buffer stays
                        // silent. Drain any remaining commands so a burst
                        // does not trickle out one per callback.
                        while let Ok(cmd) = command_rx.pop() {
                            if cmd.is_blocking() {
                                let _ = event_tx.push(EngineEvent::Rejected(cmd));
                            } else {
                                let _ = porta_engine::command::apply(&mut *engine, cmd);
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
                let _ = event_tx.push(EngineEvent::Levels {
                    tracks: std::array::from_fn(|t| engine.track_level_db(t)),
                    master: engine.master_level_db(),
                });
            },
            move |err| eprintln!("audio output error: {err}"),
            None,
        )
        // engine already moved into the closure above - can't hand it back
        // from here on, whichever of these two calls fails.
        .map_err(|e| StartError::StreamBuild(RealtimeError::from(e)))?;
    stream
        .play()
        .map_err(|e| StartError::StreamBuild(RealtimeError::from(e)))?;

    Ok(RealtimeSession {
        _input: input_stream,
        _output: stream,
        commands: command_tx,
        events: event_rx,
        engine_handoff: engine_handoff_control,
        xruns,
        period,
        input_device: input_name_used,
        output_device,
        input_tracks,
        input_channel_offset: channel_offset,
    })
}
