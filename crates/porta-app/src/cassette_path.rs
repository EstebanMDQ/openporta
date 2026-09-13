//! Which cassette the UI opens when it was not given one (REQ-1002).
//!
//! Deliberately ungated and free of environment reads: `resolve` takes
//! both candidate paths as parameters, so every branch runs in the
//! default build's tests here rather than only in the `ui` build that
//! CI compiles in one job. The `input_map.rs` precedent, for the same
//! reason its module doc gives.

use porta_dsp::character::TapeCharacter;
use porta_engine::engine::Engine;
use porta_engine::project::Project;
use porta_engine::SAMPLE_RATE;
use std::path::{Path, PathBuf};

/// `porta-app new`'s own defaults, so a first run is deterministic and
/// REQ-103's seed identity is specified rather than incidental.
const DEFAULT_MINUTES: f32 = 15.0;
const DEFAULT_SEED: u64 = 0;

/// The per-user home directory, for callers that need to build the
/// default cassette path. `USERPROFILE` after `HOME` because Windows
/// normally leaves `HOME` unset, and a HOME-only lookup there means
/// nothing is ever remembered and the fallback fires every launch.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// `~/openporta/tape1`. Visible in Finder and Explorer on purpose: the
/// Tapes view lists siblings of the open cassette, so a directory the
/// user can find is where new ones should land. `deploy/` already
/// hardcodes this path for the Pi kiosk (M6.3); the two move together.
pub fn default_cassette_dir(home: &Path) -> PathBuf {
    home.join("openporta").join("tape1")
}

/// True when `dir` holds anything at all. The occupancy test is
/// deliberately NOT "does it contain a manifest.json": creation writes
/// the manifest last, so a directory holding tape audio with a missing
/// or unreadable manifest would pass a manifest-keyed guard - and that
/// is exactly the case where the raw audio is the only thing left
/// worth saving. An unreadable directory counts as occupied: refusing
/// to write into something we cannot inspect is the safe way to be
/// wrong.
fn is_occupied(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_some(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

/// Existence is not openability - a remembered path may now be a file,
/// or a directory whose manifest is gone.
fn opens_as_cassette(dir: &Path) -> bool {
    dir.is_dir() && Project::open(dir).is_ok()
}

/// Resolve the cassette to open when none was named on the command
/// line. `remembered` is the path the UI last had open, if any;
/// `default` is the fixed per-user default cassette.
///
/// 1. The remembered path, if it opens as a cassette. A miss falls
///    through and creates nothing at that path.
/// 2. Otherwise `default`: absent or empty means create and open;
///    occupied and openable means open; occupied and not openable is
///    an error, having created, truncated or overwritten nothing.
pub fn resolve(remembered: Option<&Path>, default: &Path) -> Result<PathBuf, String> {
    if let Some(path) = remembered {
        if opens_as_cassette(path) {
            return Ok(path.to_path_buf());
        }
    }
    if is_occupied(default) {
        if opens_as_cassette(default) {
            return Ok(default.to_path_buf());
        }
        return Err(format!(
            "{} is not a cassette and is not empty - refusing to write over it. \
             Move it aside, or pass a cassette path explicitly.",
            default.display()
        ));
    }
    create_default(default)?;
    Ok(default.to_path_buf())
}

fn create_default(dir: &Path) -> Result<(), String> {
    let len = (SAMPLE_RATE as f32 * 60.0 * DEFAULT_MINUTES) as usize;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let mut engine = Engine::create_with_character(dir, len, TapeCharacter::new(DEFAULT_SEED))
        .map_err(|e| format!("cannot create a cassette at {}: {e}", dir.display()))?;
    engine
        .save()
        .map_err(|e| format!("cannot save the new cassette at {}: {e}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("porta-resolve-{name}"));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Every file under `dir` as (relative path, bytes), sorted.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in fs::read_dir(&d).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push((
                        path.strip_prefix(dir).unwrap().to_path_buf(),
                        fs::read(&path).unwrap(),
                    ));
                }
            }
        }
        out.sort();
        out
    }

    fn make_cassette(dir: &Path) {
        let mut e = Engine::create_with_character(dir, 48_000, TapeCharacter::clean()).unwrap();
        e.save().unwrap();
    }

    /// THE tape-loss regression. An occupied default is opened, never
    /// created over: four tracks, both bus channels, the manifest and
    /// every file under undo/ come out byte-identical. The manifest
    /// matters as much as the audio - REQ-103's character seed lives
    /// there, so a rewritten one changes the tape's identity even if
    /// every sample survives.
    #[test]
    fn an_occupied_default_is_opened_and_left_byte_identical() {
        let tmp = TempDir::new("occupied-default");
        let default = tmp.join("tape1");
        make_cassette(&default);
        let before = snapshot(&default);

        let got = resolve(None, &default).unwrap();

        assert_eq!(got, default);
        assert_eq!(before, snapshot(&default));
        assert!(before.iter().any(|(p, _)| p.ends_with("manifest.json")));
        assert_eq!(
            before
                .iter()
                .filter(|(p, _)| p.extension().is_some_and(|e| e == "raw"))
                .count(),
            6,
            "four tracks plus both bus channels"
        );
    }

    /// The case a manifest-keyed guard would have destroyed: raw audio
    /// present, manifest gone.
    #[test]
    fn a_default_with_tape_but_no_manifest_is_an_error_that_writes_nothing() {
        let tmp = TempDir::new("no-manifest");
        let default = tmp.join("tape1");
        make_cassette(&default);
        fs::remove_file(default.join("manifest.json")).unwrap();
        let before = snapshot(&default);

        let err = resolve(None, &default).unwrap_err();

        assert!(err.contains(&default.display().to_string()));
        assert_eq!(before, snapshot(&default));
    }

    #[test]
    fn a_remembered_plain_file_falls_through_without_creating_anything_there() {
        let tmp = TempDir::new("remembered-file");
        let remembered = tmp.join("not-a-cassette");
        fs::write(&remembered, b"hello").unwrap();
        let default = tmp.join("tape1");

        let got = resolve(Some(&remembered), &default).unwrap();

        assert_eq!(got, default);
        assert!(remembered.is_file());
        assert_eq!(fs::read(&remembered).unwrap(), b"hello");
    }

    #[test]
    fn a_remembered_non_cassette_directory_falls_through_without_creating_anything_there() {
        let tmp = TempDir::new("remembered-dir");
        let remembered = tmp.join("junk");
        fs::create_dir_all(&remembered).unwrap();
        fs::write(remembered.join("readme.txt"), b"x").unwrap();
        let default = tmp.join("tape1");

        let got = resolve(Some(&remembered), &default).unwrap();

        assert_eq!(got, default);
        assert_eq!(snapshot(&remembered).len(), 1, "nothing created there");
    }

    #[test]
    fn a_remembered_cassette_is_used_and_the_default_is_never_created() {
        let tmp = TempDir::new("remembered-ok");
        let remembered = tmp.join("mine");
        make_cassette(&remembered);
        let default = tmp.join("tape1");

        let got = resolve(Some(&remembered), &default).unwrap();

        assert_eq!(got, remembered);
        assert!(!default.exists(), "the default must not be created");
    }

    #[test]
    fn nothing_remembered_and_an_absent_default_creates_the_documented_cassette() {
        let tmp = TempDir::new("absent-default");
        let default = tmp.join("nested").join("tape1");

        let got = resolve(None, &default).unwrap();

        assert_eq!(got, default);
        let project = Project::open(&default).unwrap();
        assert_eq!(project.manifest.noise_seed, DEFAULT_SEED);
        assert_eq!(
            project.manifest.len_samples,
            (SAMPLE_RATE as f32 * 60.0 * DEFAULT_MINUTES) as usize
        );
    }

    #[test]
    fn an_empty_default_directory_is_created_in_place() {
        let tmp = TempDir::new("empty-default");
        let default = tmp.join("tape1");
        fs::create_dir_all(&default).unwrap();

        let got = resolve(None, &default).unwrap();

        assert_eq!(got, default);
        assert!(Project::open(&default).is_ok());
    }

    #[test]
    fn default_cassette_dir_joins_openporta_tape1() {
        assert_eq!(
            default_cassette_dir(Path::new("/home/someone")),
            Path::new("/home/someone/openporta/tape1")
        );
    }
}
