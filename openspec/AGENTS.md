# Agent Workflow

This project is built largely by an AI agent working unattended. These rules
keep that safe and productive.

## The contract

- `openspec/spec.md` is the constitution. Its decisions are settled. Do not
  relitigate them, do not "improve" them in passing.
- `TASKS.md` is the queue and the single source of progress truth.
- CI green (`fmt --check`, `clippy -D warnings`, `test --workspace`) is the
  only definition of done.

## The loop

1. Read `TASKS.md`. Take the topmost `[ ]` task. Mark it `[>]` (at most one
   task may be `[>]` at a time).
2. Implement only that task. Resist adjacent refactors.
3. Run the gate: `cargo fmt --check && cargo clippy --workspace
   --all-targets -- -D warnings && cargo test --workspace`
   (on hosts without rustup: `scripts/cargo-docker.sh ...`).
4. Green: mark `[x]`, commit code + TASKS.md together, push, continue.
5. Red after 3 honest attempts: revert or park on a branch, mark `[!]` with
   a one-line reason and date, notify, take the next task that does not
   depend on the blocked one.

## Changing the spec

- User-visible behavior changes or reversals of settled decisions require a
  proposal: `openspec/changes/NNN-short-title.md` with sections Motivation,
  Change, Impact on tasks.
- Run the spec-reviewer agent on the proposal. Notify the owner BEFORE
  implementing. Do not implement while the proposal is unreviewed.
- Engine internals that keep all requirements intact need no proposal.

## Golden files

Regenerating the golden render (`UPDATE_GOLDEN=1`) requires a note in
`TASKS.md` and a notification. Never bless a golden to make a red test
green without understanding why it changed.

## Notifications

```bash
curl -s -X POST http://127.0.0.1:9876/hook \
  -H "Content-Type: application/json" \
  -d '{"event_type": "Notification", "data": {"reason": "[openporta] MESSAGE"}}'
```

Send on: milestone complete, task blocked, spec-change proposal, golden
regenerated, end-of-run summary. Not on every task.

## Milestone boundaries

At each milestone: run the milestone's acceptance gate (spec.md section 6),
render an audition WAV into `auditions/` via the session-script runner so
the owner can listen, notify with a summary, then continue.
