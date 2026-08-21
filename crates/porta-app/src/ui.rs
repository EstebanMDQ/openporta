//! Slint UI (behind the `ui` feature): transport, tape counter, track
//! strips (arm/fader/pan), master, and meters (M5.1/M5.2). Undo, save,
//! and export land in M5.3.
//!
//! Per the M5 acceptance gate, this drives the engine through the
//! command queue only - every handler below calls
//! `porta_engine::command::apply` to mutate, and reads back only public
//! accessors (`state()`, `playhead()`, `is_armed()`, `fader_db()`,
//! `pan()`, `track_level_db()`, ...), never Engine's internal fields.
//!
//! There is no real audio here yet (that's the realtime adapter's job,
//! wired in once M5.3 needs it); a repeating Slint timer stands in for
//! the audio thread, feeding silence through `process_block` so the
//! transport, counter, and meters behave the way they will once real
//! input is connected. Unlike the cpal callback, this timer runs on the
//! UI's own thread - Slint's model is single-threaded and reactive, so
//! there is no cross-thread queue to build for a UI-only skeleton;
//! REQ-902 is about the *audio* callback, which this isn't.

slint::include_modules!();

use porta_engine::command::{apply, Command};
use porta_engine::engine::Engine;
use porta_engine::NUM_TRACKS;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// UI refresh rate, not an audio period.
const TICK: Duration = Duration::from_millis(20);
const TICK_SAMPLES: usize = (porta_engine::SAMPLE_RATE as usize * 20) / 1000;

/// Meter window: -60dB reads as an empty bar, 0dB (or hotter) as full.
/// A convention, not a spec requirement - matches common VU meters.
const METER_FLOOR_DB: f32 = -60.0;

/// One track strip's arm/fader/pan handlers are identical apart from
/// the track index and which generated callback they hook - a macro
/// keeps that in one place instead of four hand-copied closures.
macro_rules! wire_track {
    ($ui:expr, $engine:expr, $index:expr, $arm:ident, $fader:ident, $pan:ident) => {{
        let engine = Rc::clone(&$engine);
        let ui_weak = $ui.as_weak();
        $ui.$arm(move || {
            let mut e = engine.borrow_mut();
            let on = !e.is_armed($index);
            let _ = apply(&mut e, Command::Arm { track: $index, on });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
        });
        let engine = Rc::clone(&$engine);
        let ui_weak = $ui.as_weak();
        $ui.$fader(move |db| {
            let mut e = engine.borrow_mut();
            let _ = apply(&mut e, Command::Fader { track: $index, db });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
        });
        let engine = Rc::clone(&$engine);
        let ui_weak = $ui.as_weak();
        $ui.$pan(move |value| {
            let mut e = engine.borrow_mut();
            let _ = apply(
                &mut e,
                Command::Pan {
                    track: $index,
                    value,
                },
            );
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
        });
    }};
}

pub fn run(dir: &str) -> Result<(), String> {
    let engine = Rc::new(RefCell::new(Engine::open(dir).map_err(|e| e.to_string())?));
    let ui = MainWindow::new().map_err(|e| e.to_string())?;
    refresh(&ui, &engine.borrow());

    connect_transport(&ui, &engine);
    wire_track!(
        ui,
        engine,
        0,
        on_track1_arm_pressed,
        on_track1_fader_changed,
        on_track1_pan_changed
    );
    wire_track!(
        ui,
        engine,
        1,
        on_track2_arm_pressed,
        on_track2_fader_changed,
        on_track2_pan_changed
    );
    wire_track!(
        ui,
        engine,
        2,
        on_track3_arm_pressed,
        on_track3_fader_changed,
        on_track3_pan_changed
    );
    wire_track!(
        ui,
        engine,
        3,
        on_track4_arm_pressed,
        on_track4_fader_changed,
        on_track4_pan_changed
    );
    {
        let engine = Rc::clone(&engine);
        let ui_weak = ui.as_weak();
        ui.on_master_fader_changed(move |db| {
            let mut e = engine.borrow_mut();
            let _ = apply(&mut e, Command::Master { db });
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
        });
    }
    {
        let engine = Rc::clone(&engine);
        let ui_weak = ui.as_weak();
        ui.on_save_pressed(move || {
            let mut e = engine.borrow_mut();
            let status = status_message("save", e.save());
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status_text(status.into());
                refresh(&ui, &e);
            }
        });
    }
    {
        let engine = Rc::clone(&engine);
        let ui_weak = ui.as_weak();
        ui.on_undo_pressed(move || {
            let mut e = engine.borrow_mut();
            let status = status_message("undo", e.undo());
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status_text(status.into());
                refresh(&ui, &e);
            }
        });
    }
    {
        let engine = Rc::clone(&engine);
        let ui_weak = ui.as_weak();
        ui.on_export_pressed(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let path = ui.get_export_path().to_string();
            let mut e = engine.borrow_mut();
            let status = export_wav(&mut e, &path);
            ui.set_status_text(status.into());
            refresh(&ui, &e);
        });
    }

    let timer = slint::Timer::default();
    {
        let engine = Rc::clone(&engine);
        let ui_weak = ui.as_weak();
        let silence = vec![0.0f32; TICK_SAMPLES];
        let mut mix_l = vec![0.0f32; TICK_SAMPLES];
        let mut mix_r = vec![0.0f32; TICK_SAMPLES];
        timer.start(slint::TimerMode::Repeated, TICK, move || {
            let mut e = engine.borrow_mut();
            let inputs: [&[f32]; NUM_TRACKS] = [&silence; NUM_TRACKS];
            let _ = e.process_block(&inputs, &mut mix_l, &mut mix_r);
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
        });
    }

    ui.run().map_err(|e| e.to_string())
}

fn connect_transport(ui: &MainWindow, engine: &Rc<RefCell<Engine>>) {
    {
        let engine = Rc::clone(engine);
        let ui_weak = ui.as_weak();
        ui.on_play_pressed(move || {
            let mut e = engine.borrow_mut();
            let _ = apply(&mut e, Command::Play);
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
        });
    }
    {
        let engine = Rc::clone(engine);
        let ui_weak = ui.as_weak();
        ui.on_stop_pressed(move || {
            let mut e = engine.borrow_mut();
            let _ = apply(&mut e, Command::Stop);
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
        });
    }
    {
        let engine = Rc::clone(engine);
        let ui_weak = ui.as_weak();
        ui.on_record_pressed(move || {
            let mut e = engine.borrow_mut();
            let _ = apply(&mut e, Command::Record);
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
        });
    }
}

fn refresh(ui: &MainWindow, engine: &Engine) {
    ui.set_transport_state(format!("{:?}", engine.state()).into());
    ui.set_counter_text(format_counter(engine.playhead()).into());

    ui.set_track1_armed(engine.is_armed(0));
    ui.set_track1_fader_db(engine.fader_db(0));
    ui.set_track1_pan(engine.pan(0));
    ui.set_track1_meter_fraction(meter_fraction(engine.track_level_db(0)));

    ui.set_track2_armed(engine.is_armed(1));
    ui.set_track2_fader_db(engine.fader_db(1));
    ui.set_track2_pan(engine.pan(1));
    ui.set_track2_meter_fraction(meter_fraction(engine.track_level_db(1)));

    ui.set_track3_armed(engine.is_armed(2));
    ui.set_track3_fader_db(engine.fader_db(2));
    ui.set_track3_pan(engine.pan(2));
    ui.set_track3_meter_fraction(meter_fraction(engine.track_level_db(2)));

    ui.set_track4_armed(engine.is_armed(3));
    ui.set_track4_fader_db(engine.fader_db(3));
    ui.set_track4_pan(engine.pan(3));
    ui.set_track4_meter_fraction(meter_fraction(engine.track_level_db(3)));

    ui.set_master_fader_db(engine.master_db());
    let (ml, mr) = engine.master_level_db();
    ui.set_master_meter_l_fraction(meter_fraction(ml));
    ui.set_master_meter_r_fraction(meter_fraction(mr));

    ui.set_can_undo(engine.can_undo());
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

/// Export the whole tape from the top as a 16-bit WAV. Only ever runs
/// on the UI thread from a button press, never the timer - no REQ-902
/// concern the way the realtime callback has.
fn export_wav(engine: &mut Engine, path: &str) -> String {
    engine.seek(0);
    let len = engine.manifest().len_samples;
    let (l, r) = crate::render::mixdown(engine, len);
    match crate::render::write_wav(path, &l, &r, crate::render::BitDepth::Sixteen) {
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
}
