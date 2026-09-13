//! What the command line asks for, decided separately from doing it
//! (REQ-1001).
//!
//! `ui_available` is a PARAMETER, with `cfg!(feature = "ui")` evaluated
//! once at the call site in `main`. A function that asked `cfg!`
//! internally would leave the UI-available arm unreachable in the
//! default build, so the one test that matters here could never run -
//! change 002's feature-gating hole, one level deeper.

/// What `main` should do. Argument slices are passed through
/// untouched: this module decides routing, not flag parsing.
#[derive(Debug, PartialEq, Eq)]
pub enum Action<'a> {
    /// Print the usage text and exit successfully.
    Usage,
    Unknown(&'a str),
    New(&'a [String]),
    Script(&'a [String]),
    Render(&'a [String]),
    Devices,
    Probe(&'a [String]),
    Live(&'a [String]),
    /// `dir: None` means no cassette was named and one has to be
    /// resolved (REQ-1002). `kiosk` is only ever true when `--kiosk`
    /// was typed: an accidental double-click gets a window, never a
    /// full-screen appliance.
    OpenUi {
        dir: Option<&'a str>,
        kiosk: bool,
    },
}

/// The first argument that is not a flag, i.e. the cassette path.
fn first_positional(args: &[String]) -> Option<&str> {
    args.iter()
        .map(String::as_str)
        .find(|a| !a.starts_with('-'))
}

pub fn dispatch(args: &[String], ui_available: bool) -> Action<'_> {
    match args.first().map(String::as_str) {
        // No arguments at all: the double-click case. With a UI in the
        // binary that opens it windowed; without one there is nothing
        // to open, and a binary that silently did nothing would be
        // worse than one that explains itself.
        None => {
            if ui_available {
                Action::OpenUi {
                    dir: None,
                    kiosk: false,
                }
            } else {
                Action::Usage
            }
        }
        Some("new") => Action::New(&args[1..]),
        Some("script") => Action::Script(&args[1..]),
        Some("render") | Some("export") => Action::Render(&args[1..]),
        Some("devices") => Action::Devices,
        Some("probe") => Action::Probe(&args[1..]),
        Some("live") => Action::Live(&args[1..]),
        // `ui` with no directory resolves one rather than erroring:
        // that rule belongs to "the UI was asked to start without a
        // path", not to one spelling of it.
        Some("ui") => Action::OpenUi {
            dir: first_positional(&args[1..]),
            kiosk: args[1..].iter().any(|a| a == "--kiosk"),
        },
        Some("--help") | Some("-h") | Some("help") => Action::Usage,
        Some(other) => Action::Unknown(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arguments_opens_the_ui_windowed_when_there_is_one() {
        assert_eq!(
            dispatch(&[], true),
            Action::OpenUi {
                dir: None,
                kiosk: false
            }
        );
    }

    #[test]
    fn no_arguments_prints_usage_without_a_ui() {
        assert_eq!(dispatch(&[], false), Action::Usage);
    }

    #[test]
    fn help_prints_usage_in_both_builds() {
        for spelling in ["--help", "-h", "help"] {
            let a = args(&[spelling]);
            assert_eq!(dispatch(&a, true), Action::Usage, "{spelling} with ui");
            assert_eq!(dispatch(&a, false), Action::Usage, "{spelling} without ui");
        }
    }

    #[test]
    fn an_unknown_argument_still_errors() {
        let a = args(&["frobnicate"]);
        assert_eq!(dispatch(&a, true), Action::Unknown("frobnicate"));
        assert_eq!(dispatch(&a, false), Action::Unknown("frobnicate"));
    }

    /// `--kiosk` alone is not a command. Stated because it is easy to
    /// assume otherwise: booting straight into kiosk on the remembered
    /// cassette is an M6.3 follow-up, not this.
    #[test]
    fn kiosk_alone_is_an_unknown_argument() {
        let a = args(&["--kiosk"]);
        assert_eq!(dispatch(&a, true), Action::Unknown("--kiosk"));
    }

    #[test]
    fn every_existing_subcommand_routes_where_it_did() {
        let a = args(&["new", "mytape", "--minutes", "5"]);
        assert_eq!(dispatch(&a, true), Action::New(&a[1..]));
        let a = args(&["script", "session.json"]);
        assert_eq!(dispatch(&a, true), Action::Script(&a[1..]));
        let a = args(&["render", "mytape", "--out", "mix.wav"]);
        assert_eq!(dispatch(&a, true), Action::Render(&a[1..]));
        // export is render's alias and must stay one.
        let a = args(&["export", "mytape", "--out", "mix.wav"]);
        assert_eq!(dispatch(&a, true), Action::Render(&a[1..]));
        let a = args(&["devices"]);
        assert_eq!(dispatch(&a, true), Action::Devices);
        let a = args(&["probe", "--in", "L6"]);
        assert_eq!(dispatch(&a, true), Action::Probe(&a[1..]));
        let a = args(&["live", "mytape", "--period", "256"]);
        assert_eq!(dispatch(&a, true), Action::Live(&a[1..]));
    }

    #[test]
    fn ui_with_a_directory_is_unchanged() {
        let a = args(&["ui", "mytape"]);
        assert_eq!(
            dispatch(&a, true),
            Action::OpenUi {
                dir: Some("mytape"),
                kiosk: false
            }
        );
        let a = args(&["ui", "mytape", "--kiosk"]);
        assert_eq!(
            dispatch(&a, true),
            Action::OpenUi {
                dir: Some("mytape"),
                kiosk: true
            }
        );
    }

    #[test]
    fn ui_without_a_directory_resolves_instead_of_erroring() {
        let a = args(&["ui"]);
        assert_eq!(
            dispatch(&a, true),
            Action::OpenUi {
                dir: None,
                kiosk: false
            }
        );
        // A flag must not be mistaken for the cassette path.
        let a = args(&["ui", "--kiosk"]);
        assert_eq!(
            dispatch(&a, true),
            Action::OpenUi {
                dir: None,
                kiosk: true
            }
        );
    }
}
