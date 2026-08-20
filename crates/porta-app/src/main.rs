//! openporta CLI.

mod render;
mod script;

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
from the start, or of the first N seconds.";

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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("new") => cmd_new(&args[1..]),
        Some("script") => cmd_script(&args[1..]),
        Some("render") | Some("export") => cmd_render(&args[1..]),
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
