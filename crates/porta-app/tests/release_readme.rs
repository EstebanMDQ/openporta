//! REQ-1003: the archive ships its own README, not the repo's.
//!
//! Cheap assertions on the one document a downloader actually reads,
//! and the one most likely to rot quietly. Whether the prose is any
//! good stays a manual check (M8.11); these are the parts that can be
//! stated as facts.

fn readme() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/release-readme.md")
        .canonicalize()
        .expect("docs/release-readme.md must exist: release.yml ships it as the archive README");
    std::fs::read_to_string(path).expect("release readme must be readable")
}

#[test]
fn carries_the_required_sections_in_order() {
    let text = readme();
    let required = [
        "## Open it",
        "### macOS: let it run",
        "### Windows: let it run",
        "## What it is",
        "## Connect an interface",
        "## Where cassettes live",
        "## The command line",
        "## Honest limits",
    ];
    let mut cursor = 0usize;
    for heading in required {
        let at = text[cursor..]
            .find(heading)
            .unwrap_or_else(|| panic!("missing heading (or out of order): {heading}"));
        cursor += at + heading.len();
    }
}

/// The Gatekeeper step is the first thing a macOS user meets, so it is
/// first-class rather than a troubleshooting footnote - and it must be
/// the command a bare Unix executable actually needs.
#[test]
fn gives_the_quarantine_command_before_the_gui_route() {
    let text = readme();
    let xattr = text
        .find("xattr -d com.apple.quarantine")
        .expect("the xattr command must be present");
    let settings = text
        .find("Open Anyway")
        .expect("the System Settings route must be present");
    assert!(xattr < settings, "the one-line command comes first");
    // Removed by Apple in macOS 15 for quarantined downloads, and it
    // was never the gesture for a bare executable anyway.
    assert!(
        !text.contains("right-click and choose Open\n") && !text.contains("Right-click -> Open"),
        "must not tell people to right-click -> Open as the fix"
    );
}

#[test]
fn gives_the_windows_equivalent() {
    let text = readme();
    assert!(text.contains("SmartScreen"));
    assert!(text.contains("Run anyway"));
    assert!(text.contains("Unblock-File"));
}

#[test]
fn states_that_the_build_is_unsigned() {
    let text = readme();
    assert!(
        text.contains("unsigned"),
        "REQ-1003: the document must say so plainly"
    );
}

#[test]
fn names_every_shipped_platform_in_the_limits_section() {
    let text = readme();
    let limits = &text[text.find("## Honest limits").expect("limits section")..];
    for platform in ["macOS", "Windows", "Linux"] {
        assert!(limits.contains(platform), "limits must cover {platform}");
    }
}

/// The two concrete defects of the document v0.1.1 actually shipped:
/// every runnable instruction was `cargo run`, and line 3 linked to a
/// Spanish README that is not inside the archive.
#[test]
fn is_not_the_repo_readme() {
    let text = readme();
    assert!(
        !text.contains("cargo run"),
        "this document is for a prebuilt binary, not a checkout"
    );
    assert!(
        !text.contains("README.es.md"),
        "that link is dead inside the archive"
    );
}
