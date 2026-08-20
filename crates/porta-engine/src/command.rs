//! The control protocol between whatever drives the machine (UI, CLI,
//! MIDI foot switch one day) and the engine.
//!
//! Commands are plain data so they can cross a wait-free queue into the
//! audio thread without allocating (REQ-902). Commands that touch disk
//! or resize buffers are marked `is_blocking` and must only be applied
//! while stopped, on the control thread; the audio thread rejects them
//! rather than stalling the callback.

use crate::engine::{Engine, EngineError};
use crate::transport::TransportState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Play,
    Stop,
    Record,
    Seek { sample: usize },
    Rewind { samples: usize },
    FastForward { samples: usize },
    Arm { track: usize, on: bool },
    Fader { track: usize, db: f32 },
    Pan { track: usize, value: f32 },
    Master { db: f32 },
    Bounce,
    Undo,
    Redo,
    Save,
}

impl Command {
    /// True for commands that read or write the filesystem, or otherwise
    /// take unbounded time. These are never safe inside an audio
    /// callback.
    pub fn is_blocking(self) -> bool {
        matches!(
            self,
            Command::Bounce | Command::Undo | Command::Redo | Command::Save
        )
    }
}

/// What the engine reports back to the UI. Also plain data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EngineEvent {
    State(TransportState),
    Playhead {
        sample: usize,
    },
    Levels {
        left: f32,
        right: f32,
    },
    /// The audio callback missed its deadline; the count is cumulative.
    Xrun {
        total: u64,
    },
    Rejected(Command),
}

/// Apply a command. Blocking commands are applied here too, so this is
/// the control-thread entry point; the realtime adapter filters them out
/// before handing commands to the audio thread.
pub fn apply(engine: &mut Engine, command: Command) -> Result<(), EngineError> {
    match command {
        Command::Play => engine.play(),
        Command::Stop => engine.stop(),
        Command::Record => engine.record(),
        Command::Seek { sample } => {
            engine.seek(sample);
        }
        Command::Rewind { samples } => {
            let target = engine.playhead().saturating_sub(samples);
            engine.seek(target);
        }
        Command::FastForward { samples } => {
            let target = engine.playhead().saturating_add(samples);
            engine.seek(target);
        }
        Command::Arm { track, on } => engine.set_armed(track, on),
        Command::Fader { track, db } => engine.mixer().set_fader_db(track, db),
        Command::Pan { track, value } => engine.mixer().set_pan(track, value),
        Command::Master { db } => engine.mixer().set_master_db(db),
        Command::Bounce => engine.bounce()?,
        Command::Undo => engine.undo()?,
        Command::Redo => engine.redo()?,
        Command::Save => engine.save()?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_touching_commands_are_marked_blocking() {
        for c in [Command::Bounce, Command::Undo, Command::Redo, Command::Save] {
            assert!(c.is_blocking(), "{c:?} should be blocking");
        }
        for c in [
            Command::Play,
            Command::Stop,
            Command::Record,
            Command::Seek { sample: 0 },
            Command::Arm { track: 0, on: true },
            Command::Fader { track: 0, db: -6.0 },
            Command::Pan {
                track: 0,
                value: 0.0,
            },
            Command::Master { db: 0.0 },
        ] {
            assert!(!c.is_blocking(), "{c:?} should be safe in the callback");
        }
    }
}
