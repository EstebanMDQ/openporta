# 003: A release you can actually download and open

## Motivation

Requested directly by the owner after checking the v0.1.1 artifacts:
the release README is the wrong document, double-clicking the binary
does nothing useful, and macOS refuses to run it at all.

All three were verified against the published v0.1.1 archives rather
than assumed:

1. **The bundled README is the developer README.** `release.yml` copies
   the repo's `README.md` into every archive. That document opens with
   crate layout, the `cargo fmt --check && cargo clippy ... && cargo
   test` gate, and pointers to `TASKS.md` and `openspec/` - all correct
   for someone cloning the repo, none of it useful to someone who
   downloaded a 7MB tarball and wants to make a noise. It also
   describes building from source, which a binary release exists
   specifically to avoid.
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

### 2. Which cassette does it open?

Double-clicking supplies no path, so one has to be chosen. Order,
first hit wins:

1. The cassette the UI last had open, remembered across launches. This
   is the behaviour that makes it feel like an instrument: it comes
   back up where you left it. Stored next to the existing device
   settings in `~/.config/openporta/` (a `.config`-style path per
   platform), **not** in any cassette - it is a property of this
   install, not of a project.
2. If nothing is remembered, or the remembered path no longer exists
   (deleted, external drive unplugged), create a fresh default cassette
   in a documented per-user location and open that.

The second case MUST NOT be an error dialog. A first-ever launch
landing on a usable blank tape is the entire point; being told to go
away and pass a command-line argument is not.

Deleting or moving a cassette MUST NOT leave the app unable to start -
that is why the existence check is part of the rule and not an
implementation detail.

### 3. A release README written for the download, not the repo

`release.yml` stops copying the repo `README.md` and ships a
purpose-written `docs/release-readme.md` instead, covering, in this
order:

- **Open it.** Double-click; what you should see.
- **macOS: the Gatekeeper step**, first-class and up front rather than
  a troubleshooting footnote, because on macOS it is the first thing
  that happens. Right-click -> Open -> Open, or `xattr -d
  com.apple.quarantine ./porta-app`, with a plain statement that the
  build is unsigned, what that does and does not mean, and that the
  source and the exact build workflow are public.
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

- **Windows still opens a console window.** The binary is
  console-subsystem (verified: PE subsystem 3). A GUI-subsystem build
  would suppress it but would also break every CLI use of the same
  binary. Fixing this properly means a second, GUI-subsystem launcher
  executable - explicitly out of scope here, recorded as follow-up.
- **macOS still has no `.app` bundle**, so no custom icon, no
  Applications-folder install, and the Gatekeeper step is still
  manual on first run.
- **The Gatekeeper step is still a manual step.** Documenting it well
  is not the same as removing it. Only notarization removes it.

## Requirements affected

- **REQ-1001 (new)**: Invoked with no arguments, a build including the
  UI MUST open the UI in windowed mode on a cassette; a build without
  the UI MUST print usage. `--help` MUST print usage in both.
- **REQ-1002 (new)**: The UI MUST reopen the cassette it last had open.
  If none is remembered or it no longer exists, it MUST create and open
  a fresh cassette in a documented per-user location rather than
  failing to start. The remembered path MUST be stored per install,
  not inside any cassette.
- **REQ-1003 (new)**: Released archives MUST include a README written
  for someone running a prebuilt binary, and it MUST state that the
  build is unsigned and give the platform's steps for launching it
  anyway.
- No existing requirement is reversed. REQ-901's feature-gating and
  REQ-902's realtime rules are untouched - none of this reaches the
  audio callback.
- Section 2 (Scope) is untouched: this changes how the existing UI is
  reached, and adds no capability.

Numbering note: 1001+ deliberately, rather than extending 9xx. Section
5 is "non-functional requirements" about platforms and CI; how the
product is packaged and launched is a distinct concern that will grow
(bundles, signing, installers), and giving it its own block keeps that
from crowding the platform requirements. A new section 6, "Packaging
and distribution", carries them.

## Verification (headless, REQ-906)

- Argument-dispatch tests, in the ungated part of the crate so they run
  in the plain gate: no args + UI available -> "open the UI" decision;
  no args without UI -> usage; `--help` -> usage in both; an unknown
  argument -> the same error as today; every existing subcommand still
  routes where it did. The decision is extracted as a pure function
  returning an enum, so it is testable without opening a window.
- Cassette-resolution tests: remembered path that exists -> opened;
  remembered path that no longer exists -> falls through to a fresh
  default; nothing remembered -> fresh default; the resolution never
  returns an error case.
- Round-trip test for the remembered path through its config file,
  including a config written before this field existed.
- A packaging test asserting the release README exists and that the
  workflow ships it - cheap, and the one thing most likely to silently
  regress in a workflow edit.
- [manual] On each platform: extract the archive, double-click, confirm
  a windowed UI on a usable cassette. On macOS, confirm the documented
  Gatekeeper steps actually work on a genuinely quarantined download,
  not on a locally built file.

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

**v1 (this revision)**: initial proposal, after the owner checked the
v0.1.1 artifacts. All three problems verified against the published
archives (bundled README diffed against the repo one, no-args behaviour
run, `codesign`/`spctl` checked, Windows PE subsystem read). Two owner
decisions taken before drafting: document the Gatekeeper workaround
rather than buy a certificate, and make no-args open the UI rather than
build platform bundles. Not yet reviewed.
