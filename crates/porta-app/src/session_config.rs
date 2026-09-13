//! The cassette the UI last had open (REQ-1004).
//!
//! Its own file rather than a field on the device config, for a
//! concrete reason: `device_config::load` funnels any parse failure
//! into `unwrap_or_default()`, so a new field that broke
//! deserialization would silently discard every remembered input map -
//! a REQ-908 violation introduced by accident. A separate file cannot
//! do that, and "file absent" already means "nothing remembered". A
//! cassette path is not a device property either.
//!
//! Paths are stored absolute because the working directory differs
//! between a double-click and a terminal launch.
//!
//! Ungated, and every function takes `home` as a parameter, so the
//! tests run in the default CI gate without mutating the environment.
//! Writes happen on the control thread only, never from an audio
//! callback (REQ-902).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Absent in a config written before this field existed, which
    /// reads as "nothing remembered" rather than as an error.
    #[serde(default)]
    pub last_cassette: Option<PathBuf>,
}

pub fn path(home: &Path) -> PathBuf {
    home.join(".config").join("openporta").join("session.json")
}

/// Missing, unreadable or unparseable all read as "nothing
/// remembered": there is no first-run setup step, so a fresh install
/// has to behave exactly like an empty config.
pub fn load(home: &Path) -> SessionConfig {
    std::fs::read_to_string(path(home))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Best-effort, like `device_config::remember`: failing to record
/// where we are is not a reason to fail an open that already
/// succeeded.
pub fn remember(home: &Path, cassette: &Path) {
    let config = SessionConfig {
        last_cassette: Some(absolutize(cassette)),
    };
    let file = path(home);
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&config) {
        let _ = std::fs::write(file, json);
    }
}

/// `remember` for the call sites that have a cassette but no home in
/// hand. Silently does nothing when there is no home directory, which
/// is the same "nothing gets remembered this time" outcome as an
/// unwritable config dir.
pub fn remember_current(cassette: &Path) {
    if let Some(home) = crate::cassette_path::home_dir() {
        remember(&home, cassette);
    }
}

fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(p),
        Err(_) => p.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("porta-session-{name}"));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_config(home: &Path, body: &str) {
        let f = path(home);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, body).unwrap();
    }

    #[test]
    fn round_trips_through_the_config_file() {
        let home = TempDir::new("roundtrip");
        assert!(load(&home.0).last_cassette.is_none());
        remember(&home.0, Path::new("/tapes/mine"));
        assert_eq!(
            load(&home.0).last_cassette,
            Some(PathBuf::from("/tapes/mine"))
        );
    }

    #[test]
    fn a_config_written_before_this_field_existed_reads_as_nothing_remembered() {
        for body in ["{}", r#"{"something_else": 3}"#] {
            let home = TempDir::new("legacy");
            write_config(&home.0, body);
            assert!(load(&home.0).last_cassette.is_none(), "body {body}");
        }
    }

    #[test]
    fn a_relative_path_is_stored_absolute() {
        let home = TempDir::new("relative");
        remember(&home.0, Path::new("mytape"));
        let stored = load(&home.0).last_cassette.unwrap();
        assert!(stored.is_absolute(), "{stored:?}");
        assert!(stored.ends_with("mytape"));
    }

    /// The REQ-908 hazard this module exists to avoid, asserted
    /// directly: a corrupt session config must not cost the user their
    /// remembered input maps, which live in a different file.
    #[test]
    fn a_corrupt_session_config_leaves_the_device_config_intact() {
        let home = TempDir::new("corrupt");
        let audio = home.0.join(".config").join("openporta").join("audio.json");
        let audio_body = r#"{"last_input_device":"L6","devices":{}}"#;
        fs::create_dir_all(audio.parent().unwrap()).unwrap();
        fs::write(&audio, audio_body).unwrap();
        write_config(&home.0, "{ this is not json");

        assert!(load(&home.0).last_cassette.is_none());

        assert_eq!(fs::read_to_string(&audio).unwrap(), audio_body);
    }

    #[test]
    fn an_unwritable_config_directory_is_not_an_error() {
        let home = TempDir::new("unwritable");
        // A file where the .config directory should be, so every
        // create_dir_all and write below must fail.
        fs::write(home.0.join(".config"), b"not a directory").unwrap();
        remember(&home.0, Path::new("/tapes/mine"));
        assert!(load(&home.0).last_cassette.is_none());
    }
}
