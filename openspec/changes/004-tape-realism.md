# 004: A tape model that behaves like tape

## Motivation

Requested directly by the owner: "it only adds some flutter and noise
... there might be better ways to be more subtle and get better
results." Measured before proposing, because that impression turns out
to have three specific, quantifiable causes rather than being a matter
of taste.

(`porta_testkit::signal::sine` takes **peak** dBFS, so raw RMS readings
sit 3.01 dB below the nominal figure. Everything below is corrected for
that; the numbers are real gain and real THD.)

### 1. The chain hard-limits at about -9 dBFS peak, and over-distorts getting there

| input (peak) | real gain | THD |
|---|---|---|
| -40 dB | 0.00 dB | -50 dB |
| -18 dB | -0.27 dB | -49 dB |
| -12 dB | -0.98 dB | -38 dB |
| -6 dB | -3.08 dB | -29 dB (3.7%) |
| 0 dB | **-7.20 dB** | **-22 dB (8%)** |

`Saturation` is `tanh(x * drive) * makeup` with `makeup = 1/drive`.
At the default `drive_db: 9.0` that is a ceiling of `1/2.818 = 0.355`
- **-9 dBFS, whatever you feed it.** A real cassette at 0 VU runs
about 1-3% THD; this reaches 8% *while also* pulling the level down
7 dB. That combination is heard as small and dull, not as saturated.
It is also why the clamp test in `tests/bounce_acceptance.rs` had to
use a low-drive character: on the default formulation the i16 clamp is
mathematically unreachable, because saturation gets there first.

### 2. There is no head bump - the low end is removed rather than added

Measured at -18 dBFS: **40 Hz is -8.1 dB**, 60 Hz -3.3 dB, 80 Hz
-1.5 dB. Real cassette geometry produces a broad **+2 to +4 dB rise
around 50-100 Hz** before rolling off below it - a large part of why
tape is described as "fat". `Bandwidth`'s 60 Hz high-pass does the
opposite of the thing that makes tape sound like tape.

### 3. The noise floor is perfectly static

Silence: **-72.73 dBFS**. The tail after a -6 dBFS tone: **-72.74
dBFS**. Identical to two decimal places. Real tape's noise is made by
the same magnetised particles carrying the signal, so it **rises and
falls with the programme** - modulation noise. A fixed bed of hiss
sitting behind the music is exactly the "it adds some noise"
impression.

### And structurally

`Saturation` is **memoryless**. Real tape is hysteretic: the output
depends on magnetic history, not only the present sample. That is the
single largest difference between "sounds like tape" and "sounds like
a waveshaper", and no amount of tuning `drive_db` reaches it.

Also absent: level-dependent HF loss (tape self-erases highs when hit
hard), scrape flutter (the fast grain of tape dragging across the
heads - we model wow and flutter but not this), dropouts, and
inter-track crosstalk.

## Owner decisions already made (asked directly, 2026-08-24)

- **Build the full model**, including hysteresis, scrape flutter,
  dropouts, level-dependent HF loss and crosstalk.
- **Keep the current, cheaper model available behind a flag**, so
  constrained devices can still record.
- **The improved model becomes the default.** Existing cassettes will
  replay differently and the golden render is re-blessed once.

## Change

### Two models, chosen per cassette

A new `TapeModel` with two values:

- **`Full`** - everything below. The default.
- **`Simple`** - exactly today's chain, unchanged, bit-for-bit. Not a
  degraded version of `Full` but the existing code path preserved, so
  it stays cheap and stays a known quantity.

`TapeModel` joins `TapeCharacter` in the manifest and is **fixed at
creation**, for the same reason REQ-103 fixes the character: a cassette
must sound like itself for its whole life. `porta-app new` gains
`--model simple|full`.

**Why fixing it at creation does not strand a weak device**, which is
the obvious objection: degradation is baked at record time and the
playback path stays clean (REQ-303, and the project's own stated
invariant). The model's cost is therefore paid **only while recording
or bouncing** - any cassette, `Full` or `Simple`, plays back on any
device at the same cost as today. A constrained device that needs to
*record* creates `Simple` cassettes; it can still play, mix and export
a `Full` one made elsewhere.

### What `Full` adds

Each item says what it models physically, because that is what makes
the parameters arguable rather than arbitrary.

**a. Hysteretic saturation, replacing memoryless `tanh`.** Magnetic
domains do not follow the field instantaneously; the magnetisation
curve depends on where it has been. Intended method is Jiles-Atherton
with a **fixed-step, bounded-iteration** solver - bounded because
REQ-902 forbids unbounded work in the callback and REQ-905 needs a
predictable per-sample cost, not an average one. The requirement below
is written in terms of observable behaviour (history dependence,
level-dependent harmonic structure), so a cheaper solver that meets it
is permitted.

**b. Restore headroom.** The saturator MUST NOT impose a fixed output
ceiling well below full scale. Target: roughly unity through -12 dBFS,
gentle compression above it, and THD around 1-3% at 0 VU rather than
8%. Concretely this means decoupling makeup gain from `1/drive` - the
present coupling is what creates the -9 dBFS ceiling.

**c. Head bump.** A peaking filter around 50-100 Hz, a few dB, ahead of
the existing high-pass. Models playback-head/tape geometry resonance.

**d. Modulation noise.** Hiss amplitude follows a smoothed envelope of
the signal, over a static bias-noise floor. Models noise from the
magnetised particles themselves. Per-sample envelope, never per-block,
or it breaks REQ-203.

**e. Level-dependent HF loss.** High-level content loses more top end
(self-erasure). Implemented as an envelope-driven modulation of the
low-pass corner; again per-sample.

**f. Scrape flutter.** A third modulation term, fast (order 100 Hz+)
and very shallow (fractions of a cent), added to the existing wow and
flutter in `FlutterModulator`. Shares the one modulator, so it costs
almost nothing and stays shared between the bounce bus's two channels.

**g. Dropouts.** Brief, shallow, rare level dips from oxide
imperfections. Seeded per pass like every other stochastic element
(REQ-702), with a specified rate and depth so "rare" is testable rather
than a feel.

**h. Inter-track crosstalk.** A small amount of neighbouring-track
signal, as adjacent tracks on a physical 4-track cassette pick up each
other at the head.

**Crosstalk is architecturally different from a-g and is scheduled
last for that reason.** Everything else is a per-track effect that fits
inside `AudioProcessor` (mono, in-place). Crosstalk needs more than one
track's signal at once, so it cannot live in a track's own `Chain` -
it belongs where the tracks are visible together, at the record path in
`Engine`. It is specified here but MUST be implemented as its own task,
after a-g, and MAY be deferred without blocking them.

### What does not change

- The chain's **order** (saturation, hiss, bandwidth, flutter, crush),
  and the reasoning in `character.rs`'s module doc for why hiss sits
  before bandwidth, are unchanged. `Full` adds stages and makes
  existing ones level-aware; it does not reorder them.
- `TapeCharacter`'s existing fields keep their meaning. `clean()` stays
  near-transparent and stays the formulation tests use when they want
  mechanics without colour.
- Nothing reaches the audio callback that allocates or locks
  (REQ-902), and every added stage is per-sample so REQ-203 holds.

## Requirements affected

- **REQ-103**: extended - the cassette's **model** is fixed at creation
  and stored in the manifest, alongside the character and seed.
- **REQ-701**: rewritten. It currently enumerates the chain as a fixed
  list. It must describe the chain per model, and state that the record
  path applies hysteretic saturation, head bump, modulation noise,
  level-dependent bandwidth, wow/flutter/scrape and dropouts under
  `Full`, and the existing list under `Simple`.
- **REQ-702**: unchanged in intent, restated to cover the new
  stochastic elements - dropouts and modulation noise MUST derive from
  the cassette seed plus pass id, and two renders of the same script
  MUST stay bit-identical, for **both** models.
- **REQ-703**: unchanged and load-bearing: the hysteresis solver MUST
  be bounded, allocation-free and lock-free.
- **REQ-403**: the generation-loss acceptance test MUST pass for both
  models. This is the one that decides whether the whole change is
  real: `Full` should show *more* convincing generation loss, not less.
- **REQ-905**: gains a second obligation - the Pi headroom measurement
  MUST be taken for `Full` and `Simple`, recording and bouncing, since
  the entire point of keeping `Simple` is that `Full` may not fit.
  **If `Full` does not fit at 128-256 frames on the Pi, that is
  information, not a failure**: it makes `Simple` the Pi's default and
  is recorded as such.
- **REQ-104/REQ-301/REQ-302** and everything in 4.4 (bounce):
  untouched. The bus prints through whatever model the cassette has.
- Section 2 (Scope): untouched. This changes how the existing tape
  character sounds; it adds no capability and no control surface beyond
  one creation-time flag.

## Verification (headless, REQ-906)

Every item below is a numeric assertion on an offline render, in the
style the DSP suite already uses - no listening required.

- **Transfer curve**: at -40, -24, -12, -6 and 0 dBFS, assert `Full`'s
  gain and THD land in stated windows - in particular THD at 0 dBFS
  between roughly 1% and 3%, and no output ceiling below -3 dBFS. The
  current chain fails this test today, which is the point.
- **Head bump**: band energy at 60-80 Hz MUST be above the 200 Hz
  reference by a stated margin, and 40 Hz MUST be below it - a bump,
  not a shelf.
- **Modulation noise**: noise floor measured in a gap after loud
  programme MUST be measurably above the floor after silence, by a
  stated margin. Today that difference is 0.01 dB; a real assertion is
  the whole feature.
- **Level-dependent HF loss**: HF band energy relative to fundamental
  MUST be lower for a hot input than a quiet one, same source.
- **Hysteresis**: the same sample value MUST produce different output
  depending on the preceding trajectory - assert directly by feeding
  two signals that arrive at the same instantaneous value from
  different directions and comparing. A memoryless waveshaper cannot
  pass this, so it is a real discriminator rather than a proxy.
- **Scrape flutter**: pitch-deviation spectrum MUST show energy in the
  scrape band that `Simple` does not have.
- **Dropouts**: over a stated duration, count dips exceeding a stated
  depth and assert the rate falls in a window - and assert the same
  seed reproduces the same dropout positions exactly.
- **Both models bit-reproducible**: two renders, same seed, identical
  bytes, for `Full` and `Simple` (REQ-702).
- **`Simple` is unchanged**: a render on a `Simple` cassette MUST be
  byte-identical to today's output for the same script and seed. This
  is what makes "the old model is preserved" a fact rather than a
  claim, and it can be asserted against the current golden before it is
  re-blessed.
- **REQ-403 for both models**, three generations, monotonic HF decay
  and monotonic noise-floor rise.
- [manual] Listening pass on the Pi, both models, before and after.
  The numbers can prove the mechanism is present; only ears settle
  whether it sounds like tape.

## Impact on tasks

- A new milestone. Sized roughly one task per lettered item, ordered:
  headroom (b) first because it is the largest audible defect and the
  cheapest fix; then head bump (c) and modulation noise (d); then
  level-dependent HF (e) and scrape flutter (f); then hysteresis (a),
  which is the big one; then dropouts (g); crosstalk (h) last and
  separable.
- **One golden regeneration event**, at the point the default model
  changes - with its TASKS.md note and owner notification, exactly as
  change 001's was handled. The `Simple`-is-unchanged test above is
  what proves the re-bless reflects the new default and not an
  accidental regression.
- **M6.2** (Pi performance pass, still open) gains the two-model
  measurement described under REQ-905. It already had to cover the
  bounce path; this adds the model dimension.
- `docs/manual-checklist.md` gains the listening pass.
- Site and README copy describing the chain (`site/architecture.md`
  and its Spanish twin both enumerate the stages) will need updating
  once this lands.

## Alternatives considered and rejected

- **Tune `drive_db` and the filter corners and stop there.** Cheapest
  option, and it would fix defect 1 partially. Rejected as the answer
  on its own: it cannot produce a head bump, cannot make noise track
  the programme, and cannot add memory to a memoryless waveshaper. The
  owner's "more subtle, better results" is precisely about the cues
  tuning cannot reach.
- **Full model only, no `Simple`.** Rejected by the owner directly, and
  correctly: the Pi is the deployment target and `Full`'s cost is
  unmeasured. Keeping the existing path is also the only way to prove
  the new one did not silently change the old behaviour.
- **Make the model a runtime/playback setting rather than fixed at
  creation.** Rejected: degradation is baked at record time, so a
  playback-side switch would not change what a cassette sounds like
  anyway, and a record-side switch that changed mid-cassette would
  break "a cassette sounds like itself" (REQ-103's whole purpose).
- **Convolution with impulse responses of a real deck.** Would capture
  the linear response faithfully, but tape's defining behaviours here
  are non-linear and time-varying - exactly what an IR cannot hold.

## History

**v1 (this revision)**: initial proposal. The three defects were
measured against the current chain before drafting (transfer curve,
frequency response and noise floor, via a temporary harness that was
removed afterwards), rather than asserted. Two owner decisions taken
before drafting: build the full model but keep the current one behind a
flag for constrained devices, and let the improved model become the
default with a single golden re-bless. Not yet reviewed.
