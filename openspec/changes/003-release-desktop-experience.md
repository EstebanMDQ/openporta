# 003: A release you can actually download and open

## Motivation

Requested directly by the owner after checking the v0.1.1 artifacts:
the release README is the wrong document, double-clicking the binary
does nothing useful, and macOS refuses to run it at all.

All three were verified against the published v0.1.1 archives rather
than assumed:

1. **The bundled README is the developer README.** `release.yml`'s
   Package step is `cp README.md LICENSE "$stage/"`, so every archive
   carries the repo's own README. A first review corrected an earlier,
   sloppier version of this paragraph: it does **not** open with crate
   layout - it opens with a product description, which is fine. What is
   actually wrong with it, checked against the shipped copy:
   - every runnable instruction under "Try it" is `cargo run -p
     porta-app -- ...` (7 of them). A document that only explains how
     to build the thing you just downloaded prebuilt.
   - `## Status` is a milestone table and change-history prose -
     project bookkeeping, not user documentation.
   - line 3 is `***English** - [Espanol](README.es.md)*`, a dead link
     inside the archive: `README.es.md` is not packaged.
   - Gatekeeper, quarantine and signing appear nowhere, though on macOS
     that is the very first thing the user meets.
2. **Double-clicking does nothing.** `porta-app` with no arguments
   prints CLI usage and exits 0. On macOS and Windows that is a
   terminal flash and an empty screen. The UI is in the binary - the
   release is built `--features realtime,ui` - it just cannot be
   reached without typing `porta-app ui <dir>`.
3. **macOS blocks it.** The shipped binary is `flags=0x20002
   (adhoc,linker-signed)`, which is only what the linker does by
   default, not a Developer ID signature. `spctl -a -vv` returns
   **rejected**. A user who downloads and extracts the archive gets
   Gatekeeper's "cannot be opened because the developer cannot be
   verified" and no obvious way past it.

None of this is an engine problem. The instrument works; it is the
front door that does not.

## Owner decisions already made (asked directly, 2026-08-24)

- **Signing: document the workaround, do not buy a certificate.**
  Notarization needs a paid Apple Developer account ($99/yr) and CI
  secrets. Recorded here as a deferred follow-up with its cost stated,
  not adopted.
- **Launching: no-args opens the UI**, rather than building a macOS
  `.app` bundle and a Windows GUI-subsystem launcher. Smaller change;
  its limits are stated honestly below rather than glossed.

## Change

### 1. `porta-app` with no arguments opens the UI

- Invoked with **no arguments at all**, and built with the `ui`
  feature, `porta-app` MUST open the UI in **windowed** mode (never
  kiosk - kiosk is for a dedicated appliance and is reached only by an
  explicit `--kiosk`).
- `--help`/`-h` MUST still print the usage text. Discoverability is not
  lost, it just stops being what an accidental double-click gets.
- Every existing subcommand (`new`, `script`, `render`, `export`,
  `live`, `devices`, `probe`, `ui`) is unchanged. This adds a default
  for the empty case only; it reverses no existing invocation.
- Built **without** the `ui` feature, no-args MUST print the usage text
  exactly as today - there is no UI to open, and a binary that silently
  did nothing would be worse than one that explains itself.
- An unknown first argument still errors as it does today. "No
  arguments" is a specific case, not a catch-all.
- **When the UI cannot be opened at all, the binary MUST print the
  usage text plus a one-line reason and exit non-zero.** It MUST NOT
  hang and MUST NOT panic. This is not hypothetical: the release
  binaries are built `realtime,ui`, so bare `porta-app` over ssh on the
  Pi with no `DISPLAY`, or in a container, now tries to open a window
  where it used to print usage. `ui::run` already surfaces
  `MainWindow::new()` as an `Err`, but backend selection can also
  panic, so the requirement is on the behaviour, not on one call site.
- Two dispatch details that are easy to assume and are **not** true
  today, so they are stated: `porta-app --kiosk` does **not** work -
  it hits the unknown-command arm - and kiosk still requires
  `ui <dir> --kiosk`. This proposal does not change that; a
  boot-into-kiosk-on-the-remembered-cassette invocation is a natural
  follow-up for M6.3 but is out of scope here.
- `porta-app ui` with no directory currently errors "ui needs a project
  directory". Under this proposal it MUST resolve a cassette the same
  way a bare `porta-app` does, rather than erroring - the resolution
  rule belongs to "the UI was asked to start without a path", not to
  one spelling of that.

### 2. Which cassette does it open?

Double-clicking supplies no path, so one has to be chosen. Order,
first hit wins:

1. The cassette the UI last had open, **if it opens as a cassette**.
   Existence is not openability - a remembered path may now be a file,
   or a directory whose manifest is gone. A failed step 1 falls through
   to step 2 and MUST NOT create anything at the remembered path.
2. Otherwise the **default cassette** at a fixed, documented path -
   **opened if anything is already there, created only if nothing is**.

**Step 2's two halves are not a detail, they are the whole safety of
this feature (a first review caught the single-step version as tape
loss, and it was right).** `Project::create_with_character` builds each
track with `fs::File::create`, which **truncates**. Pointed at a
directory that already holds a cassette it would zero all four track
files and both bus channels and rewrite the manifest - destroying a
user's recording, with nothing in the undo journal that maps to it.

That is not a remote edge case. Step 1 misses for *any* reason, not
just first run: no remembered path yet, an unreadable or unparseable
config, or no home directory. And `device_config::path()` is
`std::env::var_os("HOME")` only - on Windows, a platform this release
ships, `HOME` is normally unset, so nothing would ever be remembered,
step 1 would miss every time, and step 2 would fire at the same fixed
path on **every** double-click. Under a create-unconditionally rule
that is tape loss on the second launch of the app.

**The occupancy test is "is the directory non-empty", NOT "does it
contain a manifest.json"** - a second review caught the manifest
version as still destructive, and it is right. `create_with_character`
writes the manifest **last**, after truncating all six raw files, so a
directory holding tape audio with a missing or unreadable manifest
(a crash mid-create, an interrupted copy, a hand-edited or deleted
manifest) would *pass* a manifest-keyed guard and have its audio
destroyed. That is exactly the case where the raw audio is the only
thing left worth saving.

So, normatively, resolution against a candidate directory:

- **Absent or empty** -> create a cassette there and open it.
- **Non-empty and it opens as a cassette** -> open it.
- **Non-empty and it does not open** -> report the reason and exit
  non-zero, having created, truncated or overwritten **nothing**.

The third branch is a legitimate start failure, and saying so resolves
a contradiction the previous version carried (it demanded both "never
create over an existing cassette" and "never fail to start", which
cannot both hold for a corrupt one). "MUST NOT fail to start" is scoped
to what it can actually promise: when the default path is absent or
holds a readable cassette. A corrupt or unwritable default is worth
failing over, with a reason - silently truncating it is not.

**Belt and braces, in the engine rather than in the caller.**
`create_with_character` MUST open the six raw files with
`create_new(true)` and fail if they already exist, instead of
`File::create`'s truncate. That turns the safety property from "every
caller remembers to check first" into "the API cannot truncate", and it
closes the check-then-act race two double-clicks in quick succession
would otherwise open (both see an absent default, both create, the
second truncates under the first). Stated as a consequence rather than
discovered later: `porta-app new <existing-dir>` today silently wipes a
cassette, and this turns that into an error. That is a fix, but it is a
user-visible behaviour change and belongs in the record.

**The auto-created cassette's parameters are `porta-app new`'s
defaults** - 15 minutes, the cassette character, seed 0 - so a first
run is deterministic and REQ-103's seed identity is specified rather
than incidental.

A first-ever launch landing on a usable blank tape is the entire point;
being told to go away and pass a command-line argument is not. Equally,
deleting or moving a cassette MUST NOT leave the app unable to start.

**Where things live**, named rather than hand-waved as "a per-user
location":

- The **default cassette** is `~/openporta/tape1` (on Windows,
  `%USERPROFILE%\openporta\tape1`). Deliberately visible in Finder /
  Explorer, not buried under `~/.local/share`: the Tapes view's picker
  lists *siblings of the open cassette* (`sibling_cassettes`), so a
  hidden default would give a first-run user a picker rooted somewhere
  they will never find. It also matches what `deploy/kiosk-launch.sh`
  and both `.desktop` files already hardcode, so the project keeps one
  convention instead of two.
- The remembered path MUST be stored **absolute**: the working
  directory differs between a double-click and a terminal launch, so a
  relative one would resolve to different cassettes.
- The **remembered path** goes in its own small file beside the device
  config (`~/.config/openporta/session.json`), **not** as a new field
  on `DeviceConfig`. That struct is device-keyed and lives in a file
  called `audio.json`; a cassette path is neither. There is also a
  concrete hazard in extending it: `device_config::load()` funnels any
  parse failure into `unwrap_or_default()`, so a field that broke
  deserialization would silently discard every remembered input map -
  a REQ-908 violation introduced by accident. A separate file cannot do
  that, and "file absent" already means "nothing remembered".
- Home resolution MUST fall back to `USERPROFILE` where `HOME` is
  unset, so Windows gets a real path rather than `None`. (Today's
  `HOME`-only lookup is why the Windows case above is so bad.)
- The remembered path is written **when a cassette is opened or
  switched** - launch, New, Load, and the Tapes picker all count - on
  the control thread, never on a timer and never while the transport is
  rolling.

### 3. A release README written for the download, not the repo

`release.yml` stops copying the repo `README.md` and ships a
purpose-written `docs/release-readme.md` instead, covering, in this
order:

- **Open it.** Double-click; what you should see.
- **macOS: the Gatekeeper step**, first-class and up front rather than
  a troubleshooting footnote, because on macOS it is the first thing
  that happens. A first review flagged the steps originally written
  here as stale, and it is right: "right-click -> Open" is the *app
  bundle* gesture, and Apple removed that bypass for quarantined
  downloads in macOS 15. What we ship is a bare Unix executable, which
  changes which dialog appears. So the README gives, in this order:
  1. `xattr -d com.apple.quarantine ./porta-app` - one line, works on
     every macOS version, and is what a bare executable actually needs.
  2. The GUI route for people who would rather not use a terminal:
     attempt to run it, then System Settings > Privacy & Security >
     **Open Anyway**.
  and states plainly that the build is unsigned, what that does and
  does not mean, and that the source and the exact build workflow are
  public. **The [manual] check below verifies these steps on a
  genuinely quarantined download** - not a locally built file, which
  carries no quarantine attribute and would "pass" meaninglessly.
- **Windows: the SmartScreen step** (More info -> Run anyway), for the
  same reason and with the same honesty.
- **What it is** in three lines: four mono tracks, destructive, real
  generation loss.
- **Connect an interface**, including `probe` and `--in-map`, since
  getting sound *in* is the first real obstacle after launching.
- **Where cassettes live** on disk.
- **The CLI**, briefly, for the people who want it - not first.
- Links to the site (both languages) and the repo.

The repo `README.md` stays as it is; it is correct for its own
audience. The two documents have different readers and should not be
the same file.

### 4. Honest limits of this approach

Stated because the alternative was considered and declined, not
because they went unnoticed:

A first review found this list covered only one of the four shipped
platforms. Corrected - "double-click opens the UI" is true on none of
them without caveats:

- **Linux (x86_64 and aarch64 - which includes the Pi): double-clicking
  a bare ELF binary does not launch it** in most file managers; it does
  nothing, opens a text editor, or asks. The archive ships no `.desktop`
  file - `deploy/` has three, and none of them are packaged. On Linux
  this change effectively only improves `./porta-app` from a terminal.
- **macOS opens a stray Terminal window.** Finder launches a bare Unix
  executable *through* Terminal. Same class of defect as the Windows
  console window below, on the very platform this was reported from.
  Only an `.app` bundle removes it.
- **Windows still opens a console window.** The binary is
  console-subsystem (verified: PE subsystem 3). A GUI-subsystem build
  would suppress it but would break every CLI use of the same binary.
  Fixing it properly means a second, GUI-subsystem launcher executable.
- **Windows SmartScreen** will show "Windows protected your PC" for an
  unsigned `.exe`, needing More info -> Run anyway. Same category as
  Gatekeeper and equally worth documenting.
- **macOS still has no `.app` bundle**, so no icon and no
  Applications-folder install.
- **macOS microphone permission is a real open question, not a
  cosmetic one.** A non-bundled, ad-hoc-signed executable has no
  `Info.plist` and therefore no `NSMicrophoneUsageDescription`; input
  access gets attributed to whatever launched it. Since this proposal
  puts "connect an interface" third in the README precisely because
  getting sound *in* is the first real obstacle, the manual check below
  must actually record from an input on a freshly downloaded,
  quarantined macOS binary - and if the prompt misbehaves, that belongs
  in this list rather than in a bug report later.
- **The Gatekeeper step remains manual.** Documenting it well is not
  removing it. Only notarization removes it.

## Requirements affected

- **REQ-1001 (new)**: Invoked with no arguments, a build including the
  UI MUST open the UI in windowed mode on a cassette; a build without
  the UI MUST print usage. `--help` MUST print usage in both.
- **REQ-1005 (new)**: If the UI cannot be opened, the binary MUST print
  the usage text and a one-line reason and exit non-zero, without
  hanging and without panicking. (Its own id rather than a clause of
  REQ-1001: it has its own dedicated CI assertion.)
- **REQ-1002 (new)**: **When the UI is started without a cassette
  path**, it MUST open the cassette it last had open, if that still
  exists. Otherwise it MUST open the default cassette at a documented,
  fixed per-user path, creating it only if nothing is there - it MUST
  NOT invoke a cassette-creating operation against a directory that
  already contains a `manifest.json`, and MUST NOT fail to start in
  either case. An explicit path argument always wins over both.
- **REQ-1003 (new)**: Released archives MUST contain a `README.md`
  sourced from `docs/release-readme.md` (not the repo `README.md`),
  and that document MUST state that the build is unsigned and MUST
  carry a launch section for every platform the release ships. The
  assertable core is: the archive entry is named `README.md`, its
  content comes from `docs/release-readme.md`, the repo `README.md` is
  no longer copied, and the required headings and the unsigned-build
  statement are present. Whether the prose is any good stays
  `[manual]`.
- **REQ-1004 (new)**: The **absolute** path of the last-opened cassette
  MUST be remembered **per user**, outside any cassette and separately
  from the device configuration, and MUST be updated whenever the open
  cassette changes - including an explicit `porta-app ui <dir>`, or CLI
  and kiosk users would never accumulate a remembered value at all. The
  update happens on the control thread only, and a failure to write it
  MUST NOT fail an open that already succeeded (the same best-effort
  policy `device_config::remember` already uses).

- No existing requirement is reversed. REQ-901's feature-gating and
  REQ-902's realtime rules are untouched - none of this reaches the
  audio callback.
- **Section 2 (Scope)** gains one in-scope line - "prebuilt release
  archives for the shipped platforms, and the first-run documentation
  that ships with them" - so section 7 has something to hang from.
  Today scope mentions distribution neither in nor out, which would
  leave the new requirements without an anchor. No capability is added:
  this changes how the existing UI is reached.

Numbering note, corrected (a first review caught the original: it said
"a new section 6", and `spec.md` section 6 already exists - "Acceptance
gates per milestone"). These land in a **new section 7, "Distribution and first run"**
(named for what it actually holds: REQ-1003 is packaging, but 1001,
1002, 1004 and 1005 are launch and session behaviour), appended after the acceptance gates so that nothing
existing is renumbered and no cross-reference anywhere goes stale.

The `10xx` ids stand: 4.1..4.8 own `1xx..8xx`, section 5 owns `9xx`, so
the next block is `10xx`. Section 5 was considered and rejected -
packaging will grow (bundles, signing, installers) and would crowd the
platform requirements - and it is worth noting the next free `9xx` id
is **910**, not 909: change 002's review deliberately retired REQ-909
and this proposal does not lift that.

## Verification (headless, REQ-906)

**The CI hole first, because this proposal would otherwise repeat
change 002's mistake one level deeper.** `ci.yml` runs three
default-feature commands and `porta-app`'s `default = []`, so *nothing*
behind `#[cfg(feature = "ui")]` is compiled, linted or tested on any
commit - only `release.yml` builds it, and only on a tag or dispatch. A
change to `ui.rs` cannot fail CI today, and this proposal touches
`ui.rs`. Two consequences, both required:

- **A CI job MUST build, lint AND run the `realtime,ui` build on every
  commit.** Three details, because a second review found the original
  one-line version would not have worked at all:
  - `ci.yml` installs no system packages today. The job needs exactly
    what `release.yml` already installs for its Linux legs:
    `pkg-config libasound2-dev libpipewire-0.3-dev libclang-dev
    libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
    libfontconfig-dev`. Without them the required job fails on its
    first commit.
  - `cargo clippy` alone executes nothing, and the headless assertion
    below needs the binary to actually run. So: `cargo clippy
    --features realtime,ui --all-targets -- -D warnings` **and**
    `cargo test --features realtime,ui --workspace`.
  - "MUST NOT hang" is only an assertion if it is bounded. The headless
    check runs under an explicit timeout (`timeout 30 ./porta-app` or a
    spawned child with a bounded wait) and asserts a non-zero exit
    within it. Unbounded, the job would sit until the runner's own
    limit and the requirement would quietly become a hope - the failure
    mode this proposal is careful about everywhere else.
- **Extracting a "pure function" is necessary but not sufficient.** If
  that function asks `cfg!(feature = "ui")` internally, the
  UI-available arm is unreachable in CI's build and the very test that
  matters can never run. So the dispatch decision MUST take UI
  availability as a **parameter** - `fn dispatch(args, ui_available:
  bool) -> Action` - with `cfg!` evaluated once at the call site in
  `main()`. Both arms are then exercised in the default build.
  Likewise the cassette resolver MUST take its candidate paths as
  parameters rather than reading `HOME` itself, so its tests need no
  environment mutation.

- Argument-dispatch tests, in an ungated module (the `input_map.rs`
  precedent, whose module doc already explains exactly why): no args +
  UI available -> "open the UI"; no args without UI -> usage; `--help`
  -> usage in both; unknown argument -> today's error; every existing
  subcommand still routes where it did; `ui` with no directory ->
  resolve rather than error.
- **Headless failure, as a real CI assertion rather than a `[manual]`
  hope**: in the `--features realtime,ui` job, with `DISPLAY` and
  `WAYLAND_DISPLAY` unset, bare `porta-app` MUST exit non-zero having
  printed usage and a reason - not hang, not panic.
- Cassette-resolution tests: remembered path that exists -> opened;
  remembered path that no longer exists -> falls through to the
  default; nothing remembered -> default; the resolution never returns
  an error case. **And the one that guards the tape-loss hazard: the
  default location already holds a cassette -> it is OPENED, and its
  four track files, both bus channels, `manifest.json` and the `undo/`
  directory are all byte-identical afterwards.** The manifest matters
  as much as the audio: the character seed lives there (REQ-103), so a
  rewritten manifest changes the tape's identity even if every sample
  survives. That assertion is the difference between this feature and a
  data-loss bug, so it is named here rather than left to implementation
  judgement.
- The hazard's other two shapes, also named: a directory holding raw
  tape files but **no readable manifest** -> reported, exit non-zero,
  nothing written (this is the case a manifest-keyed guard would have
  destroyed); and a remembered path that is **not a cassette** -> falls
  through to the default, with nothing created at the remembered path.
- Round-trip test for the remembered path through its config file,
  including a config written before this field existed.
- A packaging test asserting the release README exists and that the
  workflow ships it - cheap, and the one thing most likely to silently
  regress in a workflow edit.
- [manual] On each shipped platform: extract the archive, double-click,
  and record what actually happens - including the Linux and macOS
  caveats above, which this change does not fix and which the README
  must therefore describe accurately. On macOS specifically: confirm
  the documented Gatekeeper steps on a **genuinely quarantined
  download**, then record from a real input to settle the microphone-
  permission question in "Honest limits".

## Impact on tasks

- `TASKS.md`'s `## Release process` section: **R1 is `[x]` and its body
  states "Packages each binary with README.md and LICENSE"**. That
  sentence goes stale the moment `release.yml` changes, so R1 gets a
  follow-up note rather than being edited (the same treatment M3.1 got
  when change 001 superseded it).
- New tasks for: the dispatch/resolution split and its ungated tests,
  the `docs/release-readme.md` document, the `release.yml` packaging
  change plus its test, and the new `--features realtime,ui` CI job.
- Interacts with the still-unchecked **M6.3** (systemd/kiosk launch):
  the `deploy/` files hardcode `$HOME/openporta/tape1`, which this
  proposal adopts as the default cassette path precisely so the two do
  not diverge. If M6.3 later changes that path, both move together.
- `docs/manual-checklist.md` gains the per-platform launch check above.

## Relationship to the site documentation

`site/getting-started.md` (and its Spanish twin) already cover the CLI,
session scripts and `--in-map`. The release README **links** to them
rather than restating them; it carries only what someone holding a
downloaded archive needs and cannot get anywhere else - launching,
Gatekeeper/SmartScreen, where cassettes live. Stated so the two do not
drift into two half-maintained copies of the same instructions.

## Alternatives considered and rejected

- **A macOS `.app` bundle and a Windows GUI launcher** (owner decision
  above): the better end state, and the only way to remove the console
  window and get an icon. Declined for now as meaningfully more
  packaging work; recorded as follow-up rather than dropped.
- **Notarizing the macOS build**: the only thing that removes the
  Gatekeeper prompt. Declined on cost ($99/yr) - an owner call, not a
  technical one.
- **A separate `porta-ui` binary**: avoids overloading the empty
  argument case, but doubles binary size in every archive and gives a
  user two things to choose between when they wanted one.
- **Opening a file picker on double-click**: rejected as a worse first
  run. An instrument should power on ready to play, matching the
  behaviour the device-config work already established for audio
  hardware.
- **Shipping the repo README with a prepended note**: rejected. The
  first screen would still be about crate layout and the test gate.

## History

**v1**: initial proposal, after the owner checked the
v0.1.1 artifacts. All three problems verified against the published
archives (bundled README diffed against the repo one, no-args behaviour
run, `codesign`/`spctl` checked, Windows PE subsystem read). Two owner
decisions taken before drafting: document the Gatekeeper workaround
rather than buy a certificate, and make no-args open the UI rather than
build platform bundles.

A first review returned REVISE with three blocking findings, all
confirmed against the code before fixing:

**The fallback would have destroyed tape.** "Create a fresh default
cassette" was unconditional, and `Project::create_with_character` uses
`fs::File::create`, which truncates - so pointing it at an existing
cassette would zero four tracks and both bus channels. And because
`device_config::path()` reads only `HOME`, which Windows normally
leaves unset, the fallback would have fired on *every* Windows launch
at the same path: tape loss on the second double-click. Now split into
open-if-present / create-only-if-absent, with a normative "MUST NOT
invoke a creating API against a directory containing manifest.json", a
named regression test, and a `USERPROFILE` fallback so Windows gets a
real path at all.

**Section 6 was already taken** by "Acceptance gates per milestone".
The requirements move to a new section 7 appended after it, so nothing
existing is renumbered; the `10xx` block is unchanged, with a note that
the next free `9xx` is 910 because change 002 retired REQ-909.

**The verification plan could not have run.** `ui` is not a default
feature, so nothing behind it is built or tested in CI - and the
proposed "pure function" would still have been untestable if it read
`cfg!(feature = "ui")` internally, which is change 002's hole one level
deeper. Now: UI availability is a parameter, resolution takes paths as
parameters, and a `--features realtime,ui` CI job is required so the
code this proposal touches can fail CI at all.

Also fixed: the motivation mis-described the bundled README (it opens
with a product description, not crate layout - the real defects are
`cargo run`-only instructions, a milestone table, a dead Spanish link
inside the archive, and no mention of signing); "honest limits" covered
one of four shipped platforms, missing that Linux double-click does not
launch a bare ELF at all, that macOS opens a stray Terminal window,
Windows SmartScreen, and an open question about microphone permission
without an `Info.plist`; the Gatekeeper instructions were the app-bundle
gesture Apple removed in macOS 15, replaced with `xattr` first and the
System Settings route second; REQ-1002 was unscoped and collided with
an explicit path argument; the remembered path was going into
`audio.json`, where a deserialization failure would silently discard
every remembered input map (REQ-908); and there was no Impact-on-tasks
section. Ready for a second review.

**v3 (this revision)**: the second review returned **APPROVE WITH
NOTES**, with two must-fix edits before implementation - both applied.

The tape-loss guard was **still destructive**, keyed on the wrong file.
`create_with_character` writes `manifest.json` *last*, after truncating
all six raw files, so a directory holding tape audio with a missing or
unreadable manifest would pass a manifest-keyed check and lose its
audio - the exact case where the raw files are the only thing left. The
occupancy test is now "non-empty directory", with three explicit
branches (absent -> create; opens -> open; occupied but won't open ->
report and exit non-zero, writing nothing). That third branch also
resolves a contradiction the previous version carried, which demanded
both "never create over existing" and "never fail to start". Belt and
braces added in the engine as well: `create_new(true)` instead of
`File::create`, so the API cannot truncate at all and the two-launch
race closes for free - noting honestly that this turns today's silent
`porta-app new <existing-dir>` wipe into an error.

The required CI job **would not have run**: `ci.yml` installs no system
packages, `clippy` alone executes nothing, and an unbounded "MUST NOT
hang" is not an assertion. Now specifies the exact package list
`release.yml` already uses, requires `cargo test --features
realtime,ui`, and bounds the headless check with a timeout.

Also: the byte-identical assertion widened to the manifest and `undo/`
(the character seed lives in the manifest, so a rewritten one changes
the tape's identity even if every sample survives); step 1 changed from
"still exists" to "opens as a cassette", falling through without
creating at the remembered path; the remembered path specified as
absolute and per-user (not "per install" - two installs share one
`~/.config`), updated by explicit `ui <dir>` too, best-effort on write;
the cannot-open contract split into REQ-1005 since it has its own CI
assertion; section 7 renamed "Distribution and first run" for what it
actually holds; and section 2 gains a scope line so the new
requirements have an anchor.

**Status: APPROVED.** Ready to implement; not yet started.
