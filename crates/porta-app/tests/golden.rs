//! The one golden render. Everything else is asserted numerically; this
//! is the single test that pins the exact sound of a full session, so a
//! change nobody intended shows up as a diff rather than as a shrug.
//!
//! It renders a four-track session with overdubs, a bounce, and a punch,
//! then compares against `tests/golden/session.wav` sample for sample.
//!
//! If this fails, the sound changed. Work out why before doing anything
//! else. Only when the change is intended and understood:
//!
//!     UPDATE_GOLDEN=1 cargo test -p porta-app --test golden
//!
//! Blessing requires a note in TASKS.md saying what changed and why,
//! plus a notification to the owner (see openspec/AGENTS.md). Never
//! bless a golden to make a red test green.

use porta_testkit::signal::{silence, sine};
use porta_testkit::wav::write_wav_16;
use std::path::{Path, PathBuf};
use std::process::Command;

const GOLDEN: &str = "tests/golden/session.wav";
/// Maximum tolerated per-sample difference, in 16-bit LSBs.
///
/// Not zero, and the reason matters. The engine is deterministic on any
/// given machine (see the reproducibility tests in porta-engine), but
/// the character chain calls tanh, sin, cos, exp and powf, and libm does
/// not produce bit-identical results across platforms and versions. A
/// golden blessed on Debian differs from the same render on Ubuntu CI by
/// one or two LSBs on a lot of samples - about -92 dBFS, inaudible.
///
/// Three LSBs is the line: last-bit libm drift stays under it, while a
/// real change in the DSP or the mixing does not. The change that
/// prompted this comment - fixing block-dependent gain smoothing -
/// showed up as five LSBs on far fewer samples, and would still fail.
const TOLERANCE: i32 = 3;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("porta-golden-{name}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_i16(path: &Path) -> Vec<i16> {
    hound::WavReader::open(path)
        .expect("open wav")
        .samples::<i16>()
        .map(|s| s.expect("sample"))
        .collect()
}

/// A short session that exercises the whole machine: three overdubs, a
/// bounce down to track 4, a punch-in over track 1, mixer moves, and an
/// undo/redo pair.
fn render_session(dir: &Path) -> PathBuf {
    let bass = sine(110.0, -9.0, 24_000);
    let mut chord = sine(330.0, -12.0, 24_000);
    for (i, s) in chord.iter_mut().enumerate() {
        *s = (*s + sine(415.3, -12.0, 24_000)[i]) * 0.6;
    }
    let mut lead = silence(6_000);
    lead.extend(sine(880.0, -9.0, 12_000));
    let punch = sine(660.0, -6.0, 6_000);

    write_wav_16(dir.join("bass.wav"), &bass);
    write_wav_16(dir.join("chord.wav"), &chord);
    write_wav_16(dir.join("lead.wav"), &lead);
    write_wav_16(dir.join("punch.wav"), &punch);

    let script = dir.join("session.json");
    std::fs::write(
        &script,
        r#"{"ops":[
            {"op":"new","dir":"session.porta","minutes":0.25,"seed":1979},
            {"op":"arm","track":0},
            {"op":"record","input_wav":"bass.wav"},
            {"op":"arm","track":0,"on":false},
            {"op":"arm","track":1},
            {"op":"record","input_wav":"chord.wav"},
            {"op":"arm","track":1,"on":false},
            {"op":"arm","track":2},
            {"op":"record","input_wav":"lead.wav"},
            {"op":"arm","track":2,"on":false},
            {"op":"fader","track":0,"db":-2.0},
            {"op":"fader","track":1,"db":-6.0},
            {"op":"pan","track":1,"value":-0.4},
            {"op":"pan","track":2,"value":0.5},
            {"op":"bounce"},
            {"op":"arm","track":1},
            {"op":"seek","seconds":0.25},
            {"op":"record","input_wav":"punch.wav"},
            {"op":"arm","track":1,"on":false},
            {"op":"undo"},
            {"op":"redo"},
            {"op":"master","db":-1.5},
            {"op":"seek","seconds":0},
            {"op":"export","out":"discard.wav"},
            {"op":"play","seconds":0.75},
            {"op":"export","out":"session.wav"},
            {"op":"save"}
        ]}"#,
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_porta-app"))
        .arg("script")
        .arg(&script)
        .output()
        .expect("run porta-app");
    assert!(
        out.status.success(),
        "session script failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir.join("session.wav")
}

#[test]
fn full_session_matches_the_golden_render() {
    let dir = TempDir::new("session");
    let rendered = render_session(&dir.0);
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::copy(&rendered, &golden).unwrap();
        eprintln!("blessed {}", golden.display());
        eprintln!("note the change in TASKS.md and notify the owner");
        return;
    }

    assert!(
        golden.exists(),
        "no golden at {}; create it with UPDATE_GOLDEN=1",
        golden.display()
    );

    let want = read_i16(&golden);
    let got = read_i16(&rendered);
    assert_eq!(
        want.len(),
        got.len(),
        "golden is {} samples, render is {}",
        want.len(),
        got.len()
    );

    let mut worst = 0i32;
    let mut worst_at = 0usize;
    let mut over_tolerance = 0usize;
    let mut any_difference = 0usize;
    for (i, (a, b)) in want.iter().zip(&got).enumerate() {
        let d = (i32::from(*a) - i32::from(*b)).abs();
        if d > 0 {
            any_difference += 1;
        }
        if d > worst {
            worst = d;
            worst_at = i;
        }
        if d > TOLERANCE {
            over_tolerance += 1;
        }
    }
    assert_eq!(
        over_tolerance,
        0,
        "the sound changed: {over_tolerance} of {} samples are more than \
         {TOLERANCE} LSB off (worst {worst} at sample {worst_at}; {any_difference} \
         samples differ at all). This is past what cross-platform libm \
         drift explains. Understand why before blessing (see the module \
         docs).",
        want.len()
    );
}
