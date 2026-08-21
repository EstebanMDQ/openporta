//! openporta CLI.

#[cfg(feature = "realtime")]
mod realtime;
mod render;
mod script;
#[cfg(feature = "ui")]
mod ui;

use porta_dsp::character::TapeCharacter;
use porta_engine::engine::Engine;
use render::BitDepth;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
openporta - 4-track cassette portastudio

usage:
  porta-app new <dir> [--minutes N] [--seed N] [--character cassette|clean]
  porta-app script <file.json>
  porta-app render <dir> --out <file.wav> [--seconds N] [--bits 16|24]
  porta-app export <dir> --out <file.wav> [--seconds N] [--bits 16|24]

render and export are the same thing: a stereo mixdown of the whole tape
from the start, or of the first N seconds.

built with --features realtime:
  porta-app devices
  porta-app probe [--in NAME]
  porta-app live <dir> [--in NAME] [--out NAME] [--period N]
                       [--in-offset N]

--in-offset skips that many leading input channels before assigning
the rest to tracks 1-4 in order. Use it on interfaces whose first
channels carry something other than a per-track send - e.g. --in-offset
2 on a Zoom L6, whose channels 1-2 are its own main mix. Don't guess
the offset: run `probe` first, play into one input at a time, and read
off which channel index actually lights up - interfaces don't always
order their channels the way you'd expect.

built with --features ui:
  porta-app ui <dir>";

/// Minimal flag parsing: `--name value`. Returns the value if present.
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn parse_num<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>, String> {
    match flag(args, name) {
        None => Ok(None),
        Some(v) => v
            .parse::<T>()
            .map(Some)
            .map_err(|_| format!("{name} expects a number, got '{v}'")),
    }
}

fn cmd_new(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("new needs a directory")?;
    let minutes = parse_num::<f32>(args, "--minutes")?.unwrap_or(15.0);
    let seed = parse_num::<u64>(args, "--seed")?.unwrap_or(0);
    let character = match flag(args, "--character").unwrap_or("cassette") {
        "cassette" => TapeCharacter::new(seed),
        "clean" => TapeCharacter {
            noise_seed: seed,
            ..TapeCharacter::clean()
        },
        other => return Err(format!("unknown character '{other}'")),
    };
    if !(0.1..=30.0).contains(&minutes) {
        return Err(format!("tape length {minutes} is outside 0.1-30 minutes"));
    }
    let len = (porta_engine::SAMPLE_RATE as f32 * 60.0 * minutes) as usize;
    let mut engine =
        Engine::create_with_character(dir, len, character).map_err(|e| e.to_string())?;
    engine.save().map_err(|e| e.to_string())?;
    println!("created {dir} ({minutes} minute tape, seed {seed})");
    Ok(())
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("render needs a project directory")?;
    let out = flag(args, "--out").ok_or("render needs --out <file.wav>")?;
    let depth = match flag(args, "--bits") {
        None => BitDepth::Sixteen,
        Some(b) => BitDepth::parse(b).ok_or_else(|| format!("--bits expects 16 or 24, got {b}"))?,
    };
    let mut engine = Engine::open(dir).map_err(|e| e.to_string())?;
    let len = engine.manifest().len_samples;
    let samples = match parse_num::<f32>(args, "--seconds")? {
        Some(s) => ((porta_engine::SAMPLE_RATE as f32 * s) as usize).min(len),
        None => len,
    };
    engine.seek(0);
    let (l, r) = render::mixdown(&mut engine, samples);
    render::write_wav(out, &l, &r, depth).map_err(|e| e.to_string())?;
    println!(
        "wrote {out} ({:.1}s, {} bit)",
        l.len() as f32 / porta_engine::SAMPLE_RATE as f32,
        if depth == BitDepth::Sixteen { 16 } else { 24 }
    );
    Ok(())
}

fn cmd_script(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("script needs a file")?;
    let base = Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    script::Runner::new(base)
        .run_file(path)
        .map_err(|e| e.to_string())
}

#[cfg(feature = "realtime")]
fn cmd_devices() -> Result<(), String> {
    for line in realtime::list_devices().map_err(|e| e.to_string())? {
        println!("{line}");
    }
    Ok(())
}

#[cfg(feature = "realtime")]
fn cmd_probe(args: &[String]) -> Result<(), String> {
    realtime::probe_input(flag(args, "--in")).map_err(|e| e.to_string())
}

#[cfg(feature = "ui")]
fn cmd_ui(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("ui needs a project directory")?;
    ui::run(dir)
}

/// "1R - 2 - 3 - 4R" style summary of which tracks are record-armed.
#[cfg(feature = "realtime")]
fn arm_status(armed: &[bool; porta_engine::NUM_TRACKS]) -> String {
    armed
        .iter()
        .enumerate()
        .map(|(t, &on)| {
            if on {
                format!("{}R", t + 1)
            } else {
                (t + 1).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" - ")
}

/// Drive a cassette from the keyboard against real hardware. This is the
/// harness for the manual checklist in docs/manual-checklist.md; the UI
/// proper arrives in M5.
#[cfg(feature = "realtime")]
fn cmd_live(args: &[String]) -> Result<(), String> {
    use porta_engine::command::Command;
    use std::io::BufRead;

    let dir = args.first().ok_or("live needs a project directory")?;
    let engine = Engine::open(dir).map_err(|e| e.to_string())?;
    let period = parse_num::<usize>(args, "--period")?;
    let channel_offset = parse_num::<usize>(args, "--in-offset")?.unwrap_or(0);
    let mut session = realtime::start(
        engine,
        flag(args, "--in"),
        flag(args, "--out"),
        period,
        channel_offset,
    )
    .map_err(|e| e.to_string())?;

    println!("output: {}", session.output_device);
    match session.input_device.as_deref() {
        Some(name) => println!(
            "input:  {name} (channels {}-{} -> tracks 1-{}{})",
            session.input_channel_offset + 1,
            session.input_channel_offset + session.input_tracks,
            session.input_tracks,
            if session.input_tracks < porta_engine::NUM_TRACKS {
                " - fewer input channels than tracks, the rest record silence"
            } else {
                ""
            }
        ),
        None => println!("input:  (none)"),
    }
    println!("period: {} frames", session.period);
    println!(
        "keys: p play, s stop, r record, 1-4 arm/disarm, 0 seek to start, \
         [ rew 1s, ] ff 1s, q quit"
    );
    let mut armed = [false; porta_engine::NUM_TRACKS];
    println!("  {}", arm_status(&armed));

    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let cmd = match line.trim() {
            "p" => Some(Command::Play),
            "s" => Some(Command::Stop),
            "r" => Some(Command::Record),
            "0" => Some(Command::Seek { sample: 0 }),
            "[" => Some(Command::Rewind { samples: 48_000 }),
            "]" => Some(Command::FastForward { samples: 48_000 }),
            t if matches!(t, "1" | "2" | "3" | "4") => {
                let track = t.parse::<usize>().unwrap() - 1;
                armed[track] = !armed[track];
                println!("  {}", arm_status(&armed));
                Some(Command::Arm {
                    track,
                    on: armed[track],
                })
            }
            "q" => break,
            "" => None,
            other => {
                eprintln!("unknown key '{other}'");
                None
            }
        };
        if let Some(c) = cmd {
            if session.send(c).is_err() {
                eprintln!("command queue full or command not allowed while rolling");
            }
        }
        for event in session.poll() {
            use porta_engine::command::EngineEvent;
            // Playhead and Levels fire every callback (~200/s at a
            // 256-frame period) - telemetry for a meter, not something
            // to print a line per tick.
            if !matches!(
                event,
                EngineEvent::Playhead { .. } | EngineEvent::Levels { .. }
            ) {
                println!("  {event:?}");
            }
        }
    }
    println!("xruns: {}", session.xrun_summary());

    // Save/Bounce/Undo/Redo can't run while the audio thread owns the
    // engine (REQ-902), so quitting is the only place `live` persists:
    // stop the streams, take the engine back, finish any open pass, and
    // write it to disk.
    println!("saving...");
    let mut engine = session
        .shutdown()
        .map_err(|e| format!("shutdown failed, nothing saved: {e}"))?;
    engine.stop();
    engine.save().map_err(|e| format!("save failed: {e}"))?;
    println!("saved.");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("new") => cmd_new(&args[1..]),
        Some("script") => cmd_script(&args[1..]),
        Some("render") | Some("export") => cmd_render(&args[1..]),
        #[cfg(feature = "realtime")]
        Some("devices") => cmd_devices(),
        #[cfg(feature = "realtime")]
        Some("probe") => cmd_probe(&args[1..]),
        #[cfg(feature = "realtime")]
        Some("live") => cmd_live(&args[1..]),
        #[cfg(not(feature = "realtime"))]
        Some(c @ ("devices" | "probe" | "live")) => Err(format!(
            "{c} needs the realtime feature: cargo run -p porta-app --features realtime -- {c}"
        )),
        #[cfg(feature = "ui")]
        Some("ui") => cmd_ui(&args[1..]),
        #[cfg(not(feature = "ui"))]
        Some(c @ "ui") => Err(format!(
            "{c} needs the ui feature: cargo run -p porta-app --features ui -- {c}"
        )),
        Some("--help") | Some("-h") | Some("help") | None => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(format!("unknown command '{other}'\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
