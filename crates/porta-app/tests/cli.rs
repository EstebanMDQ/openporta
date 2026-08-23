//! CLI surface: new / script / render / export driven as a real binary.

use porta_testkit::meter::rms_dbfs;
use porta_testkit::signal::sine;
use porta_testkit::wav::write_wav_16;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("porta-cli-{name}"));
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

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_porta-app"))
        .args(args)
        .output()
        .expect("run porta-app")
}

fn run_ok(args: &[&str]) {
    let out = run(args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn wav_spec(path: &Path) -> hound::WavSpec {
    hound::WavReader::open(path).expect("open wav").spec()
}

fn read_stereo(path: &Path) -> (Vec<f32>, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).expect("open wav");
    let bits = reader.spec().bits_per_sample;
    let scale = 1.0 / (1i64 << (bits - 1)) as f32;
    let s: Vec<f32> = reader
        .samples::<i32>()
        .map(|v| v.expect("sample") as f32 * scale)
        .collect();
    (
        s.iter().step_by(2).copied().collect(),
        s.iter().skip(1).step_by(2).copied().collect(),
    )
}

#[test]
fn help_is_available_and_unknown_commands_fail() {
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("portastudio"));

    let bad = run(&["frobnicate"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown command"));
}

#[test]
fn new_creates_a_cassette_that_render_can_open() {
    let dir = TempDir::new("new");
    let project = dir.0.join("tape.porta");
    let out = dir.0.join("blank.wav");
    run_ok(&[
        "new",
        project.to_str().unwrap(),
        "--minutes",
        "1",
        "--seed",
        "5",
    ]);
    assert!(project.join("manifest.json").exists());
    assert!(project.join("tape/track0.raw").exists());

    run_ok(&[
        "render",
        project.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--seconds",
        "2",
    ]);
    let (l, _) = read_stereo(&out);
    assert_eq!(l.len(), 96_000, "two seconds of blank tape");
    assert!(rms_dbfs(&l) < -80.0, "blank tape should be silent");
}

#[test]
fn new_rejects_bad_arguments() {
    let dir = TempDir::new("badargs");
    let p = dir.0.join("x.porta");
    for args in [
        vec!["new", p.to_str().unwrap(), "--minutes", "99"],
        vec!["new", p.to_str().unwrap(), "--minutes", "banana"],
        vec!["new", p.to_str().unwrap(), "--character", "vinyl"],
    ] {
        assert!(!run(&args).status.success(), "{args:?} should have failed");
    }
}

#[test]
fn render_honours_bit_depth_and_export_is_an_alias() {
    let dir = TempDir::new("depth");
    let project = dir.0.join("tape.porta");
    run_ok(&["new", project.to_str().unwrap(), "--minutes", "1"]);

    let sixteen = dir.0.join("a.wav");
    let twentyfour = dir.0.join("b.wav");
    run_ok(&[
        "render",
        project.to_str().unwrap(),
        "--out",
        sixteen.to_str().unwrap(),
        "--seconds",
        "1",
    ]);
    run_ok(&[
        "export",
        project.to_str().unwrap(),
        "--out",
        twentyfour.to_str().unwrap(),
        "--seconds",
        "1",
        "--bits",
        "24",
    ]);
    assert_eq!(wav_spec(&sixteen).bits_per_sample, 16);
    assert_eq!(wav_spec(&twentyfour).bits_per_sample, 24);
    assert_eq!(wav_spec(&twentyfour).sample_rate, 48_000);
    assert_eq!(wav_spec(&twentyfour).channels, 2);

    let bad = run(&[
        "render",
        project.to_str().unwrap(),
        "--out",
        dir.0.join("c.wav").to_str().unwrap(),
        "--bits",
        "32",
    ]);
    assert!(!bad.status.success(), "--bits 32 should be refused");
}

#[test]
fn script_then_render_reproduces_the_same_mix() {
    let dir = TempDir::new("script-render");
    write_wav_16(dir.0.join("take.wav"), &sine(1000.0, -6.0, 48_000));
    let script = dir.0.join("s.json");
    std::fs::write(
        &script,
        r#"{"ops":[
            {"op":"new","dir":"tape.porta","minutes":1,"seed":7,"character":"clean"},
            {"op":"arm","track":0},
            {"op":"record","input_wav":"take.wav"},
            {"op":"arm","track":0,"on":false},
            {"op":"seek","seconds":0},
            {"op":"export","out":"monitor.wav"},
            {"op":"play","seconds":1},
            {"op":"export","out":"from_script.wav"},
            {"op":"save"}
        ]}"#,
    )
    .unwrap();
    run_ok(&["script", script.to_str().unwrap()]);

    let via_cli = dir.0.join("from_render.wav");
    run_ok(&[
        "render",
        dir.0.join("tape.porta").to_str().unwrap(),
        "--out",
        via_cli.to_str().unwrap(),
        "--seconds",
        "1",
    ]);

    let (script_l, _) = read_stereo(&dir.0.join("from_script.wav"));
    let (render_l, _) = read_stereo(&via_cli);
    assert_eq!(script_l.len(), render_l.len());
    assert_eq!(
        script_l, render_l,
        "the script runner and the render command must agree (REQ-803)"
    );
}

#[test]
fn bounce_op_runs_from_a_script() {
    let dir = TempDir::new("bounce");
    write_wav_16(dir.0.join("take.wav"), &sine(600.0, -12.0, 24_000));
    let script = dir.0.join("s.json");
    std::fs::write(
        &script,
        r#"{"ops":[
            {"op":"new","dir":"tape.porta","minutes":1,"seed":2},
            {"op":"arm","track":0},
            {"op":"record","input_wav":"take.wav"},
            {"op":"arm","track":0,"on":false},
            {"op":"bounce_arm"},
            {"op":"seek","seconds":0},
            {"op":"bounce","seconds":1.5},
            {"op":"bounce_arm","on":false},
            {"op":"fader","track":0,"db":-60},
            {"op":"fader","track":1,"db":-60},
            {"op":"fader","track":2,"db":-60},
            {"op":"seek","seconds":0},
            {"op":"export","out":"discard.wav"},
            {"op":"play","seconds":0.5},
            {"op":"export","out":"track4.wav"},
            {"op":"save"}
        ]}"#,
    )
    .unwrap();
    run_ok(&["script", script.to_str().unwrap()]);

    // Only track 4 is up, so hearing the tone proves the bounce landed.
    let (l, _) = read_stereo(&dir.0.join("track4.wav"));
    assert!(
        rms_dbfs(&l[8192..]) > -40.0,
        "bounced material missing from track 4 ({:.1} dBFS)",
        rms_dbfs(&l[8192..])
    );
}
