//! M5.1 UI skeleton (Slint, behind the `ui` feature): transport buttons
//! and a tape counter. Track strips, undo, save, and export land in
//! M5.2/M5.3.
//!
//! Per the M5 acceptance gate, this drives the engine through the
//! command queue only - button handlers and the timer below call
//! `porta_engine::command::apply` and read back the public
//! `state()`/`playhead()` accessors, never Engine's internal fields.
//!
//! There is no real audio here yet (that's the realtime adapter's job,
//! wired in once M5.2/M5.3 need it); a repeating Slint timer stands in
//! for the audio thread, feeding silence through `process_block` so the
//! transport and counter behave the way they will once real input is
//! connected. Unlike the cpal callback, this timer runs on the UI's own
//! thread - Slint's model is single-threaded and reactive, so there is
//! no cross-thread queue to build for a UI-only skeleton; REQ-902 is
//! about the *audio* callback, which this isn't.

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

pub fn run(dir: &str) -> Result<(), String> {
    let engine = Rc::new(RefCell::new(Engine::open(dir).map_err(|e| e.to_string())?));
    let ui = MainWindow::new().map_err(|e| e.to_string())?;
    refresh(&ui, &engine.borrow());

    {
        let engine = Rc::clone(&engine);
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
        let engine = Rc::clone(&engine);
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
        let engine = Rc::clone(&engine);
        let ui_weak = ui.as_weak();
        ui.on_record_pressed(move || {
            let mut e = engine.borrow_mut();
            let _ = apply(&mut e, Command::Record);
            if let Some(ui) = ui_weak.upgrade() {
                refresh(&ui, &e);
            }
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

fn refresh(ui: &MainWindow, engine: &Engine) {
    ui.set_transport_state(format!("{:?}", engine.state()).into());
    ui.set_counter_text(format_counter(engine.playhead()).into());
}

fn format_counter(playhead_samples: usize) -> String {
    let total_seconds = playhead_samples / porta_engine::SAMPLE_RATE as usize;
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
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
}
