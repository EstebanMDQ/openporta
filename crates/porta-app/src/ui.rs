//! Slint UI (behind the `ui` feature): transport, tape counter, track
//! strips (arm/fader/pan), master, meters (M5.1/M5.2), save/undo/
//! export, cassette new/load (M5.3/M5.4), and - when built with
//! `realtime` too - real audio through the same cpal adapter `live`
//! uses (M5.5).
//!
//! Per the M5 acceptance gate, this drives the engine through the
//! command queue only - every handler below calls `Backend::send` (in
//! turn `porta_engine::command::apply` or `RealtimeSession::send`) to
//! mutate, and reads back only a `Snapshot` built from public
//! accessors or mirrored `EngineEvent`s, never Engine's internal
//! fields directly.
//!
//! # Silent vs. Live
//!
//! Without the `realtime` feature (or before the user presses
//! Connect), the UI owns `Engine` directly - a repeating Slint timer
//! stands in for the audio thread, feeding silence through
//! `process_block` so the transport, counter, and meters behave the
//! way they will once real input is connected. This needs no
//! cross-thread queue: Slint's model is single-threaded and reactive.
//!
//! With `realtime` on and Connect pressed, a real `RealtimeSession`
//! runs on cpal's thread instead, and this file only mirrors its
//! state - refreshed from `session.poll()` each tick, since nothing
//! else can safely reach into Engine while it's over there. One real
//! consequence: blocking commands (Save/Undo/Redo, and export, which
//! isn't a Command but touches the engine the same way) cannot reach a
//! running session (REQ-902 - the audio thread can't be asked to do
//! disk I/O). `with_engine` below is how they run anyway: disconnect
//! (hand the engine back), run the operation directly on it, then
//! reconnect with the same settings - a brief, deliberate audio
//! interruption, not a bug.

slint::include_modules!();

use porta_dsp::character::TapeCharacter;
#[cfg(not(feature = "realtime"))]
use porta_engine::command::apply;
use porta_engine::command::Command;
#[cfg(feature = "realtime")]
use porta_engine::command::{apply, EngineEvent};
use porta_engine::engine::Engine;
use porta_engine::transport::TransportState;
use porta_engine::NUM_TRACKS;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// UI refresh rate, not an audio period.
const TICK: Duration = Duration::from_millis(20);
const TICK_SAMPLES: usize = (porta_engine::SAMPLE_RATE as usize * 20) / 1000;

/// How far a single rewind/fast-forward button press moves the
/// playhead. Matches the `[`/`]` keys in `cmd_live`.
const SEEK_STEP_SAMPLES: usize = porta_engine::SAMPLE_RATE as usize;

/// Meter window: -60dB reads as an empty bar, 0dB (or hotter) as full.
/// A convention, not a spec requirement - matches common VU meters.
const METER_FLOOR_DB: f32 = -60.0;

/// New-cassette length from the UI's New button. The CLI's `new` has
/// --minutes/--seed/--character flags with no UI equivalent yet; this
/// is a fixed, reasonable default, not a re-implementation of those.
const DEFAULT_MINUTES: f32 = 15.0;

/// Whatever the UI is currently driving the engine through. See the
/// module doc comment for the Silent/Live split.
enum Backend {
    // Boxed: Engine is ~900 bytes, and every Backend value (including
    // the Live variant, much smaller) would otherwise pay for the
    // larger one's stack space.
    Silent(Box<Engine>),
    #[cfg(feature = "realtime")]
    Live(Box<LiveState>),
}

/// A running `RealtimeSession` plus a mirror of its state, refreshed
/// from `session.poll()` each timer tick. Nothing echoes
/// arm/fader/pan/master back from the audio thread, so `Backend::send`
/// is the only place these four update - they're set here optimistically
/// at send time, which is correct as long as this UI is the only
/// source of those commands (true today).
#[cfg(feature = "realtime")]
struct LiveState {
    session: crate::realtime::RealtimeSession,
    transport_state: TransportState,
    playhead: usize,
    armed: [bool; NUM_TRACKS],
    fader_db: [f32; NUM_TRACKS],
    pan: [f32; NUM_TRACKS],
    master_db: f32,
    track_level_db: [f32; NUM_TRACKS],
    master_level_db: (f32, f32),
}

#[cfg(feature = "realtime")]
impl LiveState {
    /// Carries a snapshot's mirrored values into a freshly (re)connected
    /// session, so switching backends never resets what the user had
    /// set - the values are all still correct, they just haven't been
    /// touched by this particular session yet.
    fn from_snapshot(session: crate::realtime::RealtimeSession, snap: &Snapshot) -> Self {
        LiveState {
            session,
            transport_state: snap.transport_state,
            playhead: snap.playhead,
            armed: snap.armed,
            fader_db: snap.fader_db,
            pan: snap.pan,
            master_db: snap.master_db,
            track_level_db: snap.track_level_db,
            master_level_db: snap.master_level_db,
        }
    }
}

/// Everything `refresh` needs, gathered from whichever `Backend`
/// variant is active. Decouples `refresh` (Slint setter calls only)
/// from how the values were actually obtained.
struct Snapshot {
    transport_state: TransportState,
    playhead: usize,
    armed: [bool; NUM_TRACKS],
    fader_db: [f32; NUM_TRACKS],
    pan: [f32; NUM_TRACKS],
    track_level_db: [f32; NUM_TRACKS],
    master_db: f32,
    master_level_db: (f32, f32),
    can_undo: bool,
    connected: bool,
    connection_status: String,
}

impl Backend {
    /// Mutate the engine, wherever it currently lives. Non-blocking
    /// commands only (REQ-902 forbids anything else reaching the
    /// realtime thread) - see `with_engine` for Save/Undo/export.
    fn send(&mut self, cmd: Command) {
        match self {
            Backend::Silent(engine) => {
                let _ = apply(engine, cmd);
            }
            #[cfg(feature = "realtime")]
            Backend::Live(live) => {
                match cmd {
                    Command::Arm { track, on } => live.armed[track] = on,
                    Command::Fader { track, db } => live.fader_db[track] = db,
                    Command::Pan { track, value } => live.pan[track] = value,
                    Command::Master { db } => live.master_db = db,
                    _ => {}
                }
                let _ = live.session.send(cmd);
            }
        }
    }

    /// Silent's half of the per-tick refresh: advance the engine with
    /// silence, standing in for the audio thread. A no-op when Live.
    #[cfg_attr(not(feature = "realtime"), allow(irrefutable_let_patterns))]
    fn tick_silent(&mut self, silence: &[f32], mix_l: &mut [f32], mix_r: &mut [f32]) {
        if let Backend::Silent(engine) = self {
            let inputs: [&[f32]; NUM_TRACKS] = [silence; NUM_TRACKS];
            let _ = engine.process_block(&inputs, mix_l, mix_r);
        }
    }

    /// Live's half of the per-tick refresh: drain whatever the audio
    /// thread reported since last time. A no-op when Silent.
    #[cfg(feature = "realtime")]
    fn poll_live(&mut self) {
        if let Backend::Live(live) = self {
            for event in live.session.poll() {
                match event {
                    EngineEvent::State(s) => live.transport_state = s,
                    EngineEvent::Playhead { sample } => live.playhead = sample,
                    EngineEvent::Levels { tracks, master } => {
                        live.track_level_db = tracks;
                        live.master_level_db = master;
                    }
                    // Xrun is visible via the Connect status line
                    // instead of a per-event pop-up; Rejected can't
                    // happen here since blocking commands never reach
                    // send() from this file.
                    EngineEvent::Xrun { .. } | EngineEvent::Rejected(_) => {}
                }
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        match self {
            Backend::Silent(engine) => Snapshot {
                transport_state: engine.state(),
                playhead: engine.playhead(),
                armed: std::array::from_fn(|t| engine.is_armed(t)),
                fader_db: std::array::from_fn(|t| engine.fader_db(t)),
                pan: std::array::from_fn(|t| engine.pan(t)),
                track_level_db: std::array::from_fn(|t| engine.track_level_db(t)),
                master_db: engine.master_db(),
                master_level_db: engine.master_level_db(),
                can_undo: engine.can_undo(),
                connected: false,
                connection_status: "not connected - silent, no real audio".to_string(),
            },
            #[cfg(feature = "realtime")]
            Backend::Live(live) => Snapshot {
                transport_state: live.transport_state,
                playhead: live.playhead,
                armed: live.armed,
                fader_db: live.fader_db,
                pan: live.pan,
                track_level_db: live.track_level_db,
                master_db: live.master_db,
                master_level_db: live.master_level_db,
                // Nothing echoes undo-journal state back from the
                // audio thread, so this can't be exact while Live -
                // Undo stays clickable regardless; the disconnect it
                // triggers reports "nothing to undo" same as ever if
                // there wasn't anything.
                can_undo: true,
                connected: true,
                connection_status: format!(
                    "connected: out {} / in {} (period {})",
                    live.session.output_device,
                    live.session.input_device.as_deref().unwrap_or("(none)"),
                    live.session.period,
                ),
            },
        }
    }
}

/// Take the engine out of `backend` outright. If it was Live, this
/// disconnects (stops the audio thread) - callers that want to stay
/// connected afterward are responsible for reconnecting; see
/// `with_engine`. Falls back to reopening `cassette_path` from disk in
/// the near-impossible case shutdown itself fails (the audio thread
/// would have to hang for the full multi-second handoff window) rather
/// than leave the caller with nothing.
#[cfg_attr(not(feature = "realtime"), allow(unused_variables))]
fn take_engine(backend: Backend, cassette_path: &str) -> Engine {
    match backend {
        Backend::Silent(engine) => *engine,
        #[cfg(feature = "realtime")]
        Backend::Live(live) => match live.session.shutdown() {
            Ok(engine) => engine,
            Err(e) => {
                eprintln!("shutdown failed ({e}), reopening {cassette_path}");
                Engine::open(cassette_path).expect("cassette directory should still be there")
            }
        },
    }
}

/// Run `f` on the real Engine, disconnecting a live session first if
/// necessary and reconnecting with the same settings afterward. This
/// is the only way Save/Undo/export/New/Load can happen while
/// connected (REQ-902 - the realtime thread can't do disk I/O), and it
/// briefly interrupts audio to do it.
#[cfg_attr(not(feature = "realtime"), allow(unused_variables))]
fn with_engine(
    slot: &mut Option<Backend>,
    cassette_path: &str,
    f: impl FnOnce(&mut Engine) -> String,
) -> String {
    let backend = slot.take().expect("backend always present between ticks");
    let snap = backend.snapshot();
    #[cfg(feature = "realtime")]
    let reconnect = match &backend {
        Backend::Live(live) => Some((
            live.session.output_device.clone(),
            live.session.input_device.clone(),
            live.session.period,
            live.session.input_channel_offset,
        )),
        Backend::Silent(_) => None,
    };
    let mut engine = take_engine(backend, cassette_path);
    let status = f(&mut engine);

    #[cfg(feature = "realtime")]
    if let Some((output, input, period, channel_offset)) = reconnect {
        match crate::realtime::start(
            engine,
            input.as_deref(),
            Some(&output),
            Some(period),
            channel_offset,
        ) {
            Ok(session) => {
                *slot = Some(Backend::Live(Box::new(LiveState::from_snapshot(
                    session, &snap,
                ))));
                return format!("{status} (reconnected)");
            }
            Err(e) => {
                let reason = e.reason().to_string();
                let engine = e.into_engine().unwrap_or_else(|| {
                    eprintln!(
                        "reconnect failed past the point of engine recovery, \
                         reopening {cassette_path}"
                    );
                    Engine::open(cassette_path).expect("cassette directory should still be there")
                });
                *slot = Some(Backend::Silent(Box::new(engine)));
                return format!("{status}; reconnect failed: {reason}");
            }
        }
    }
    *slot = Some(Backend::Silent(Box::new(engine)));
    status
}

#[cfg(feature = "realtime")]
fn connect(
    slot: &mut Option<Backend>,
    cassette_path: &str,
    input: Option<&str>,
    output: Option<&str>,
    period: Option<usize>,
    channel_offset: usize,
) -> String {
    let backend = slot.take().expect("backend always present between ticks");
    let snap = backend.snapshot();
    let engine = take_engine(backend, cassette_path);
    match crate::realtime::start(engine, input, output, period, channel_offset) {
        Ok(session) => {
            let status = format!(
                "connected: out {} / in {}",
                session.output_device,
                session.input_device.as_deref().unwrap_or("(none)")
            );
            // Remember what worked (M6.1) so next launch pre-fills it -
            // see connect_audio's startup pre-fill and device_config's
            // module doc comment.
            if let Some(name) = &session.input_device {
                crate::device_config::DeviceConfig::remember(
                    name,
                    &session.output_device,
                    session.period,
                    session.input_channel_offset,
                );
            }
            *slot = Some(Backend::Live(Box::new(LiveState::from_snapshot(
                session, &snap,
            ))));
            status
        }
        Err(e) => {
            let reason = e.reason().to_string();
            let engine = e.into_engine().unwrap_or_else(|| {
                eprintln!(
                    "connect failed past the point of engine recovery, \
                     reopening {cassette_path}"
                );
                Engine::open(cassette_path).expect("cassette directory should still be there")
            });
            *slot = Some(Backend::Silent(Box::new(engine)));
            format!("connect failed: {reason}")
        }
    }
}

#[cfg(feature = "realtime")]
fn disconnect(slot: &mut Option<Backend>, cassette_path: &str) -> String {
    let backend = slot.take().expect("backend always present between ticks");
    let was_live = matches!(backend, Backend::Live(_));
    let engine = take_engine(backend, cassette_path);
    *slot = Some(Backend::Silent(Box::new(engine)));
    if was_live {
        "disconnected".to_string()
    } else {
        "not connected".to_string()
    }
}

/// Blank (or the device dropdown's own "(default)" placeholder entry)
/// means "system default" - matches --in/--out on `live`. Kept
/// unconditional (only `connect`, realtime-only, calls it) so its unit
/// test always runs regardless of feature flags.
#[cfg_attr(not(feature = "realtime"), allow(dead_code))]
fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s == "(default)" {
        None
    } else {
        Some(s.to_string())
    }
}

/// One track strip's arm/fader/pan handlers are identical apart from
/// the track index and which generated callback they hook - a macro
/// keeps that in one place instead of four hand-copied closures.
macro_rules! wire_track {
    ($ui:expr, $backend:expr, $index:expr, $arm:ident, $fader:ident, $pan:ident) => {{
        let backend = Rc::clone(&$backend);
        let ui_weak = $ui.as_weak();
        $ui.$arm(move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            let on = !b.snapshot().armed[$index];
            b.send(Command::Arm { track: $index, on });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
        let backend = Rc::clone(&$backend);
        let ui_weak = $ui.as_weak();
        $ui.$fader(move |db| {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::Fader { track: $index, db });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
        let backend = Rc::clone(&$backend);
        let ui_weak = $ui.as_weak();
        $ui.$pan(move |value| {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::Pan {
                track: $index,
                value,
            });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }};
}

pub fn run(dir: &str, kiosk: bool) -> Result<(), String> {
    let backend: Rc<RefCell<Option<Backend>>> = Rc::new(RefCell::new(Some(Backend::Silent(
        Box::new(Engine::open(dir).map_err(|e| e.to_string())?),
    ))));
    let ui = MainWindow::new().map_err(|e| e.to_string())?;
    ui.set_kiosk_mode(kiosk);
    refresh(&ui, &backend.borrow().as_ref().unwrap().snapshot());
    // Default export path resolves against the cassette, not whatever
    // directory the process happened to be launched from.
    ui.set_export_path(default_export_path(dir).into());
    ui.set_cassette_path(dir.into());
    #[cfg(feature = "realtime")]
    {
        refresh_device_lists(&ui);
        prefill_remembered_audio_settings(&ui);
    }

    connect_transport(&ui, &backend);
    connect_cassette(&ui, &backend);
    connect_audio(&ui, &backend);
    wire_track!(
        ui,
        backend,
        0,
        on_track1_arm_pressed,
        on_track1_fader_changed,
        on_track1_pan_changed
    );
    wire_track!(
        ui,
        backend,
        1,
        on_track2_arm_pressed,
        on_track2_fader_changed,
        on_track2_pan_changed
    );
    wire_track!(
        ui,
        backend,
        2,
        on_track3_arm_pressed,
        on_track3_fader_changed,
        on_track3_pan_changed
    );
    wire_track!(
        ui,
        backend,
        3,
        on_track4_arm_pressed,
        on_track4_fader_changed,
        on_track4_pan_changed
    );
    {
        let backend = Rc::clone(&backend);
        let ui_weak = ui.as_weak();
        ui.on_master_fader_changed(move |db| {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::Master { db });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }
    {
        let backend = Rc::clone(&backend);
        let ui_weak = ui.as_weak();
        ui.on_save_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let path = ui.get_cassette_path().to_string();
            let mut slot = backend.borrow_mut();
            let status = with_engine(&mut slot, &path, |engine| {
                status_message("save", engine.save())
            });
            ui.set_status_text(status.into());
            refresh(&ui, &slot.as_ref().unwrap().snapshot());
        });
    }
    {
        let backend = Rc::clone(&backend);
        let ui_weak = ui.as_weak();
        ui.on_undo_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let path = ui.get_cassette_path().to_string();
            let mut slot = backend.borrow_mut();
            let status = with_engine(&mut slot, &path, |engine| {
                status_message("undo", engine.undo())
            });
            ui.set_status_text(status.into());
            refresh(&ui, &slot.as_ref().unwrap().snapshot());
        });
    }
    {
        let backend = Rc::clone(&backend);
        let ui_weak = ui.as_weak();
        ui.on_export_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let cassette_path = ui.get_cassette_path().to_string();
            let export_path = ui.get_export_path().to_string();
            let mut slot = backend.borrow_mut();
            let status = with_engine(&mut slot, &cassette_path, |engine| {
                export_wav(engine, &export_path)
            });
            ui.set_status_text(status.into());
            refresh(&ui, &slot.as_ref().unwrap().snapshot());
        });
    }
    {
        let backend = Rc::clone(&backend);
        let ui_weak = ui.as_weak();
        ui.on_export_mp3_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let cassette_path = ui.get_cassette_path().to_string();
            let mp3_path = mp3_export_path(ui.get_export_path().as_ref());
            let mut slot = backend.borrow_mut();
            let status = with_engine(&mut slot, &cassette_path, |engine| {
                export_mp3(engine, &mp3_path)
            });
            ui.set_status_text(status.into());
            refresh(&ui, &slot.as_ref().unwrap().snapshot());
        });
    }

    let timer = slint::Timer::default();
    {
        let backend = Rc::clone(&backend);
        let ui_weak = ui.as_weak();
        let silence = vec![0.0f32; TICK_SAMPLES];
        let mut mix_l = vec![0.0f32; TICK_SAMPLES];
        let mut mix_r = vec![0.0f32; TICK_SAMPLES];
        timer.start(slint::TimerMode::Repeated, TICK, move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.tick_silent(&silence, &mut mix_l, &mut mix_r);
            #[cfg(feature = "realtime")]
            b.poll_live();
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }

    ui.run().map_err(|e| e.to_string())
}

fn connect_transport(ui: &MainWindow, backend: &Rc<RefCell<Option<Backend>>>) {
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_play_pressed(move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::Play);
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_stop_pressed(move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::Stop);
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_record_pressed(move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::Record);
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_seek_start_pressed(move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::Seek { sample: 0 });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_seek_end_pressed(move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            // Transport::seek clamps to the tape's length itself, so
            // the UI doesn't need to track it separately (and Live
            // mode has no cheap way to ask the engine for it anyway).
            b.send(Command::Seek { sample: usize::MAX });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_rewind_pressed(move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::Rewind {
                samples: SEEK_STEP_SAMPLES,
            });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_fast_forward_pressed(move || {
            let mut slot = backend.borrow_mut();
            let b = slot.as_mut().expect("backend always present between ticks");
            b.send(Command::FastForward {
                samples: SEEK_STEP_SAMPLES,
            });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &b.snapshot());
            }
        });
    }
}

/// New/Load swap the engine a running UI is driving, going through
/// `with_engine` the same as Save/Undo/export so a live session
/// disconnects and reconnects around the swap instead of being left
/// driving a now-orphaned engine.
fn connect_cassette(ui: &MainWindow, backend: &Rc<RefCell<Option<Backend>>>) {
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_new_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let path = ui.get_cassette_path().to_string();
            let mut slot = backend.borrow_mut();
            let mut created = false;
            let status = with_engine(&mut slot, &path, |engine| {
                match create_default_cassette(&path) {
                    Ok(new_engine) => {
                        *engine = new_engine;
                        created = true;
                        format!("created {path}")
                    }
                    Err(e) => format!("new failed: {e}"),
                }
            });
            if created {
                ui.set_export_path(default_export_path(&path).into());
            }
            ui.set_status_text(status.into());
            refresh(&ui, &slot.as_ref().unwrap().snapshot());
        });
    }
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_load_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let path = ui.get_cassette_path().to_string();
            let mut slot = backend.borrow_mut();
            let mut loaded = false;
            let status = with_engine(&mut slot, &path, |engine| match Engine::open(&path) {
                Ok(new_engine) => {
                    *engine = new_engine;
                    loaded = true;
                    format!("loaded {path}")
                }
                Err(e) => format!("load failed: {e}"),
            });
            if loaded {
                ui.set_export_path(default_export_path(&path).into());
            }
            ui.set_status_text(status.into());
            refresh(&ui, &slot.as_ref().unwrap().snapshot());
        });
    }
}

/// Pre-fills the input/output/period/offset fields from whatever
/// connected successfully last time (M6.1). The input field itself
/// gets filled in too, not left blank - cpal's own "default device"
/// resolution can't be trusted to land back on the same device (on
/// the pipewire host it's a generic two-channel pseudo-device, not a
/// real proxy to a multichannel interface like the L6; found
/// 2026-08-21). A fresh install with nothing remembered yet leaves
/// main.slint's own hardcoded defaults untouched.
#[cfg(feature = "realtime")]
fn prefill_remembered_audio_settings(ui: &MainWindow) {
    let config = crate::device_config::DeviceConfig::load();
    let Some(name) = config.last_input_device() else {
        return;
    };
    let Some(remembered) = config.get(name) else {
        return;
    };
    ui.set_input_device_text(name.into());
    if let Some(output) = &remembered.output_device {
        ui.set_output_device_text(output.clone().into());
    }
    ui.set_period_text(remembered.period.to_string().into());
    ui.set_channel_offset_text(remembered.input_channel_offset.to_string().into());
}

/// (Re)scans available devices into the Settings view's two dropdowns -
/// called once at startup and again every time Settings opens, so a
/// device plugged in after launch shows up without restarting the app.
/// A scan failure just leaves the dropdowns as they were rather than
/// clearing them out from under whatever was already selected.
#[cfg(feature = "realtime")]
fn refresh_device_lists(ui: &MainWindow) {
    let Ok((mut outputs, mut inputs)) = crate::realtime::list_device_names() else {
        return;
    };
    outputs.insert(0, "(default)".to_string());
    inputs.insert(0, "(default)".to_string());
    let to_model = |names: Vec<String>| {
        slint::ModelRc::new(slint::VecModel::from(
            names
                .into_iter()
                .map(slint::SharedString::from)
                .collect::<Vec<_>>(),
        ))
    };
    ui.set_output_device_names(to_model(outputs));
    ui.set_input_device_names(to_model(inputs));
}

/// Connect/Disconnect always exist so a `ui`-only build (no
/// `realtime`) has a discoverable, non-broken button rather than a
/// silently dead one - it just explains why nothing happens.
#[cfg_attr(not(feature = "realtime"), allow(unused_variables))]
fn connect_audio(ui: &MainWindow, backend: &Rc<RefCell<Option<Backend>>>) {
    #[cfg(feature = "realtime")]
    {
        let ui_weak = ui.as_weak();
        ui.on_refresh_devices_pressed(move || {
            if let Some(ui) = ui_weak.upgrade() {
                refresh_device_lists(&ui);
            }
        });
    }
    #[cfg(feature = "realtime")]
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_connect_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let cassette_path = ui.get_cassette_path().to_string();
            let input = non_empty(&ui.get_input_device_text());
            let output = non_empty(&ui.get_output_device_text());
            let period = ui.get_period_text().parse::<usize>().ok();
            let channel_offset = ui.get_channel_offset_text().parse::<usize>().unwrap_or(0);
            let mut slot = backend.borrow_mut();
            let status = connect(
                &mut slot,
                &cassette_path,
                input.as_deref(),
                output.as_deref(),
                period,
                channel_offset,
            );
            ui.set_status_text(status.into());
            refresh(&ui, &slot.as_ref().unwrap().snapshot());
        });
    }
    #[cfg(not(feature = "realtime"))]
    {
        let ui_weak = ui.as_weak();
        ui.on_connect_pressed(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status_text(
                    "this build has no realtime feature (rebuild with --features realtime,ui)"
                        .into(),
                );
            }
        });
    }

    #[cfg(feature = "realtime")]
    {
        let backend = Rc::clone(backend);
        let ui_weak = ui.as_weak();
        ui.on_disconnect_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let cassette_path = ui.get_cassette_path().to_string();
            let mut slot = backend.borrow_mut();
            let status = disconnect(&mut slot, &cassette_path);
            ui.set_status_text(status.into());
            refresh(&ui, &slot.as_ref().unwrap().snapshot());
        });
    }
    #[cfg(not(feature = "realtime"))]
    {
        let ui_weak = ui.as_weak();
        ui.on_disconnect_pressed(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status_text("this build has no realtime feature".into());
            }
        });
    }
}

/// Mirrors `wire_track!`: the four track strips refresh identically
/// apart from index and which generated setter they call.
macro_rules! refresh_track {
    ($ui:expr, $snap:expr, $index:expr, $set_armed:ident, $set_fader:ident, $set_pan:ident, $set_meter:ident) => {
        $ui.$set_armed($snap.armed[$index]);
        $ui.$set_fader($snap.fader_db[$index]);
        $ui.$set_pan($snap.pan[$index]);
        $ui.$set_meter(meter_fraction($snap.track_level_db[$index]));
    };
}

fn refresh(ui: &MainWindow, snap: &Snapshot) {
    ui.set_transport_state(format!("{:?}", snap.transport_state).into());
    ui.set_counter_text(format_counter(snap.playhead).into());

    refresh_track!(
        ui,
        snap,
        0,
        set_track1_armed,
        set_track1_fader_db,
        set_track1_pan,
        set_track1_meter_fraction
    );
    refresh_track!(
        ui,
        snap,
        1,
        set_track2_armed,
        set_track2_fader_db,
        set_track2_pan,
        set_track2_meter_fraction
    );
    refresh_track!(
        ui,
        snap,
        2,
        set_track3_armed,
        set_track3_fader_db,
        set_track3_pan,
        set_track3_meter_fraction
    );
    refresh_track!(
        ui,
        snap,
        3,
        set_track4_armed,
        set_track4_fader_db,
        set_track4_pan,
        set_track4_meter_fraction
    );

    ui.set_master_fader_db(snap.master_db);
    ui.set_master_meter_l_fraction(meter_fraction(snap.master_level_db.0));
    ui.set_master_meter_r_fraction(meter_fraction(snap.master_level_db.1));

    ui.set_can_undo(snap.can_undo);
    ui.set_connected(snap.connected);
    ui.set_connection_status(snap.connection_status.clone().into());
}

fn format_counter(playhead_samples: usize) -> String {
    let total_seconds = playhead_samples / porta_engine::SAMPLE_RATE as usize;
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
}

/// Maps a dBFS reading onto the 0..1 the meter bar draws with.
fn meter_fraction(db: f32) -> f32 {
    ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

fn status_message(action: &str, result: Result<(), porta_engine::engine::EngineError>) -> String {
    match result {
        Ok(()) => format!("{action}: ok"),
        Err(e) => format!("{action} failed: {e}"),
    }
}

/// Create a fresh cassette at `path` with the UI's fixed New-button
/// defaults (15 minutes, cassette character, seed 0).
fn create_default_cassette(path: &str) -> Result<Engine, String> {
    let len = (porta_engine::SAMPLE_RATE as f32 * 60.0 * DEFAULT_MINUTES) as usize;
    Engine::create_with_character(path, len, TapeCharacter::new(0)).map_err(|e| e.to_string())
}

/// A sensible default export target: next to the cassette, not
/// wherever the process's cwd happened to be.
fn default_export_path(cassette_dir: &str) -> String {
    std::path::Path::new(cassette_dir)
        .join("export.wav")
        .to_string_lossy()
        .into_owned()
}

/// Export the whole tape from the top as a 16-bit WAV.
fn export_wav(engine: &mut Engine, path: &str) -> String {
    engine.seek(0);
    let len = engine.manifest().len_samples;
    let (l, r) = crate::render::mixdown(engine, len);
    match crate::render::write_wav(path, &l, &r, crate::render::BitDepth::Sixteen) {
        Ok(()) => format!("exported to {path}"),
        Err(e) => format!("export failed: {e}"),
    }
}

/// Same base name/location as the WAV export path, extension swapped
/// to .mp3 - "Export MP3" reuses whatever's already typed there rather
/// than needing a second path field.
fn mp3_export_path(wav_export_path: &str) -> String {
    std::path::Path::new(wav_export_path)
        .with_extension("mp3")
        .to_string_lossy()
        .into_owned()
}

/// Export the whole tape from the top as an MP3 (the share format,
/// fixed bitrate - see render::write_mp3; WAV is the master).
fn export_mp3(engine: &mut Engine, path: &str) -> String {
    engine.seek(0);
    let len = engine.manifest().len_samples;
    let (l, r) = crate::render::mixdown(engine, len);
    match crate::render::write_mp3(path, &l, &r) {
        Ok(()) => format!("exported to {path}"),
        Err(e) => format!("export failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_formats_minutes_and_seconds() {
        assert_eq!(format_counter(0), "00:00");
        assert_eq!(format_counter(48_000), "00:01");
        assert_eq!(format_counter(48_000 * 65), "01:05");
    }

    #[test]
    fn meter_fraction_maps_the_60db_window() {
        assert_eq!(meter_fraction(-60.0), 0.0);
        assert_eq!(meter_fraction(0.0), 1.0);
        assert!((meter_fraction(-30.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn meter_fraction_clamps_outside_the_window() {
        assert_eq!(meter_fraction(-160.0), 0.0, "silence floor stays empty");
        assert_eq!(
            meter_fraction(6.0),
            1.0,
            "a hot signal doesn't overflow the bar"
        );
    }

    #[test]
    fn status_message_reports_ok_and_error() {
        assert_eq!(status_message("save", Ok(())), "save: ok");
        let err = Err(porta_engine::engine::EngineError::NotStopped("save"));
        assert_eq!(
            status_message("save", err),
            "save failed: save is only allowed while stopped"
        );
    }

    #[test]
    fn default_export_path_resolves_against_the_cassette() {
        assert_eq!(
            default_export_path("/Users/me/takes/song.porta"),
            "/Users/me/takes/song.porta/export.wav"
        );
        assert_eq!(
            default_export_path("relative/dir"),
            "relative/dir/export.wav"
        );
    }

    #[test]
    fn mp3_export_path_swaps_the_extension() {
        assert_eq!(
            mp3_export_path("/Users/me/takes/export.wav"),
            "/Users/me/takes/export.mp3"
        );
        assert_eq!(mp3_export_path("relative/take"), "relative/take.mp3");
    }

    #[test]
    fn non_empty_blank_means_default() {
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty("   "), None);
        assert_eq!(non_empty("(default)"), None);
        assert_eq!(non_empty(" ZOOM L6 "), Some("ZOOM L6".to_string()));
    }

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("porta-ui-{name}"));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn export_wav_writes_a_real_wav_file() {
        let dir = TempDir::new("export");
        let mut engine = Engine::create(&dir.0, 4_800, 1).unwrap();
        let out = dir.0.join("out.wav");
        let status = export_wav(&mut engine, out.to_str().unwrap());
        assert!(status.starts_with("exported to"), "got: {status}");
        assert!(out.exists(), "export_wav should have written a file");

        let reader = hound::WavReader::open(&out).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, porta_engine::SAMPLE_RATE);
        assert_eq!(reader.duration(), 4_800);
    }

    #[test]
    fn export_mp3_writes_a_real_mp3_file() {
        let dir = TempDir::new("export-mp3");
        let mut engine = Engine::create(&dir.0, 4_800, 1).unwrap();
        let out = dir.0.join("out.mp3");
        let status = export_mp3(&mut engine, out.to_str().unwrap());
        assert!(status.starts_with("exported to"), "got: {status}");
        // A real MPEG frame sync (11 set bits) at the start, not just a
        // file with the right name - encoding actually happened.
        let bytes = std::fs::read(&out).unwrap();
        assert!(!bytes.is_empty(), "export_mp3 should have written data");
        assert_eq!(bytes[0], 0xFF, "should start with an MPEG frame sync");
        assert_eq!(
            bytes[1] & 0xE0,
            0xE0,
            "should start with an MPEG frame sync"
        );
    }

    #[test]
    fn create_default_cassette_makes_a_15_minute_tape() {
        let dir = TempDir::new("new-cassette");
        let engine = create_default_cassette(dir.0.to_str().unwrap()).unwrap();
        let expected = (porta_engine::SAMPLE_RATE as f32 * 60.0 * 15.0) as usize;
        assert_eq!(engine.manifest().len_samples, expected);
        assert!(dir.0.join("manifest.json").exists());
    }

    #[test]
    fn silent_backend_snapshot_reflects_engine_state() {
        let dir = TempDir::new("snapshot");
        let mut engine = Engine::create(&dir.0, 48_000, 1).unwrap();
        engine.set_armed(1, true);
        engine.mixer().set_fader_db(1, -12.0);
        let backend = Backend::Silent(Box::new(engine));
        let snap = backend.snapshot();
        assert!(snap.armed[1]);
        assert!(!snap.armed[0]);
        assert_eq!(snap.fader_db[1], -12.0);
        assert!(!snap.connected);
        assert_eq!(snap.transport_state, TransportState::Stopped);
    }

    #[test]
    fn with_engine_runs_the_operation_and_leaves_backend_silent() {
        let dir = TempDir::new("with-engine");
        let engine = Engine::create(&dir.0, 4_800, 1).unwrap();
        let mut slot = Some(Backend::Silent(Box::new(engine)));
        let status = with_engine(&mut slot, dir.0.to_str().unwrap(), |engine| {
            status_message("save", engine.save())
        });
        assert_eq!(status, "save: ok");
        assert!(matches!(slot, Some(Backend::Silent(_))));
    }

    #[test]
    fn take_engine_unwraps_silent_directly() {
        let dir = TempDir::new("take-engine");
        let engine = Engine::create(&dir.0, 4_800, 1).unwrap();
        let backend = Backend::Silent(Box::new(engine));
        let engine = take_engine(backend, dir.0.to_str().unwrap());
        assert_eq!(engine.manifest().len_samples, 4_800);
    }
}
