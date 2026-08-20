# openporta

Software emulation of a 4-track cassette portastudio. Pure Rust. The engine
is a hardware-agnostic library; adapters are thin. Product decisions live in
`openspec/spec.md` and are settled - do not relitigate them. Workflow rules
live in `openspec/AGENTS.md` - follow them.

## Commands

```bash
# Full gate (definition of done for every task)
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace

# On hosts without rustup (e.g. thevault), same gate via Docker:
scripts/cargo-docker.sh 'fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace'

# Run a session script headlessly (from M3)
cargo run -p porta-app -- script <file.json>
```

## Crate map

- `crates/porta-dsp` - AudioProcessor trait, lo-fi processors, TapeCharacter
- `crates/porta-engine` - tape, transport, record/bounce/undo, mixer,
  persistence
- `crates/porta-testkit` - generators, meters, spectral analysis, click
  detector (dev-dependency only)
- `crates/porta-app` - CLI + script runner; `realtime` and `ui` features

## Invariants

- 48kHz, f32 processing, i16 on tape (TPDF dither before quantization).
- No allocation, locking, or disk I/O in `process_block` or any
  `AudioProcessor::process`.
- All randomness is seeded; offline renders must be bit-reproducible.
- Degradation is baked at record time; the playback path stays clean.
- Every task must be verifiable headlessly by `cargo test`.
- dsp <- engine <- app dependency direction; engine never sees hardware.

## Workflow

- Pick the topmost unchecked task in `TASKS.md`. Implement only that task.
- Commit only when the full gate is green. One task, one commit, TASKS.md
  updated in the same commit. Push after committing.
- Commit messages: "M1.4 record pass with punch crossfade" style. Concise.
  No attribution lines, no Co-Authored-By, no emoji.
- Blocked after 3 honest attempts: mark `[!]` in TASKS.md with a one-line
  reason, notify (see openspec/AGENTS.md), move to the next independent
  task.
- Spec deviations and golden regeneration: see openspec/AGENTS.md.

## Style

- No em dashes or en dashes anywhere, including docs and comments. Use
  regular hyphens.
- Comment non-obvious code only; state constraints the code cannot show.
