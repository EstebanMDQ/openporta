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
- **The improved model becomes the default** for newly created
  cassettes. (The original wording here said "existing cassettes will
  replay differently". A first review found that **false**, and it is:
  `Engine::process_block` runs a chain only in the recording and
  bouncing branches - every other path is a bare `tape.read`, and tape
  holds post-chain i16. Already-recorded audio replays bit-identically
  forever. What changes is **new passes** and **script renders**, which
  is why the golden moves at all.)

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
`--model simple|full`. The flag is **creation-only and never a UI
control** - it is a property of the cassette, not a quality preference
to be toggled while working.

**A missing `model` field MUST deserialize as `Simple`, not as the
creation default.** This is the sharpest edge in the whole proposal and
a first review caught it: `Manifest.character` uses `#[serde(default =
"TapeCharacter::default")]`, so the obvious implementation of a new
field is a serde default - and if that default were `Full`, every
cassette recorded before this change would silently switch formulation
the next time you overdubbed onto it. That is precisely what REQ-103
exists to prevent. `Full` is the default *at creation*; absence means
`Simple`, forever.

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

**A rule that spans all of a-h, because it is what makes `Simple`'s
preservation achievable at all:** every new stochastic stage MUST draw
from **its own RNG stream**, and under `Simple` both the executed code
path *and the sequence of RNG draws* MUST be exactly today's. A first
review found this would otherwise break by construction:
`FlutterModulator` has a single xorshift state consuming exactly one
draw per sample, so folding scrape flutter into it shifts the random
walk **even with scrape depth set to zero** - the generator has already
advanced. Same hazard in `Hiss` for modulation noise. Relatedly,
`reseed_chain`'s hardcoded stage indices (`HISS_STAGE = 1`,
`FLUTTER_STAGE = 3`, `SPLIT_HISS_STAGE = 1`) become model-dependent and
MUST be resolved per model rather than as global constants.

**a. Hysteretic saturation, replacing memoryless `tanh`.** Magnetic
domains do not follow the field instantaneously; the magnetisation
curve depends on where it has been. Intended method is Jiles-Atherton
with a fixed-step solver and a **stated iteration cap** (a number, in
the implementing task, not "bounded") plus a stated per-sample
transcendental count, because REQ-902 forbids unbounded callback work
and REQ-905 needs a predictable worst case rather than an average.

Three constraints that are easy to miss and are all load-bearing:

- **`REQ-702` is the real risk of this whole change, and calling it
  "unchanged in intent" underplayed it.** REQ-702 tolerates a couple of
  LSBs across platforms because libm transcendentals differ in their
  last bits. A memoryless `tanh` cannot accumulate that error - each
  sample is independent. An iterative solver with state feedback
  *does*: last-bit differences re-enter its own state sample after
  sample, and `golden.rs`'s `TOLERANCE = 3` is exactly the check that
  would then fail on Linux against a macOS-blessed render.
  **Position taken: constrain the solver rather than weaken the
  guarantee.** The feedback path MUST use only operations that are
  bit-exact across IEEE-754 platforms (add, subtract, multiply,
  divide, sqrt), with the Langevin term evaluated from a table plus
  polynomial rather than libm `coth`/`exp`. A cross-platform
  equivalence check belongs in the implementing task. If that proves
  impossible, the fallback is an explicit amendment to REQ-702 and the
  golden tolerance - a spec change in its own right, not something to
  absorb silently.
- **Stability MUST NOT regress.** `saturation.rs`'s
  `output_is_bounded_and_finite_under_abuse` asserts finite, bounded
  output for input around `1e9`. `tanh` cannot diverge; a J-A solver
  can NaN or blow up. That test MUST pass unchanged for `Full`.
- The requirement is written in terms of observable behaviour, so a
  cheaper solver that meets it is permitted.

**b. Restore headroom.** The saturator MUST NOT impose a fixed output
ceiling well below full scale. Concretely this means decoupling makeup
gain from `1/drive`, which is what creates the -9 dBFS ceiling.

**The operating reference level is stated here because without it the
THD target is meaningless** (a first review caught the motivation
saying "1-3% at 0 VU" and the requirement saying "at 0 dBFS" - which
differ by whatever headroom you pick, and the loose reading would be
satisfied by a chain that is essentially clean at real tracking
levels, the opposite of what was asked for). **0 VU = -18 dBFS**, the
usual digital convention. So: roughly unity below 0 VU, gentle
compression above it, 1-3% THD at 0 VU, and progressively more above -
with no hard ceiling short of full scale.

Two consequences to state rather than discover: print levels rise by
roughly the 7 dB of gain reduction being removed, so the record path
MUST still land below full scale so that the i16 clamp is not the
audible limiter; and
`bounce_acceptance.rs::hot_generations_engage_the_quantize_clamp`'s
documented rationale - that the clamp is unreachable on a default
cassette because saturation gets there first - becomes obsolete and
must be revisited with the test.

**c. Head bump.** A peaking filter around 50-100 Hz, a few dB, ahead of
the existing high-pass. Models playback-head/tape geometry resonance.

**d. Modulation noise.** Hiss amplitude follows a smoothed envelope of
the signal, over a static bias-noise floor. Models noise from the
magnetised particles themselves. Per-sample envelope, never per-block,
or it breaks REQ-203.

**e. Level-dependent HF loss.** High-level content loses more top end
(self-erasure). The **mechanism must be specified, not just the
effect**: recomputing RBJ biquad coefficients per sample means a
`sin_cos` and divides per sample per track on the Pi, and
continuously-varying biquad coefficients raise their own stability and
zipper questions. Acceptable implementations are a modulated one-pole,
bounded interpolation between two precomputed coefficient sets, or a
rate-limited coefficient update at a stated interval - and whichever is
chosen, its update rate MUST NOT make the result depend on block size
(see the invariance note below).

**f. Scrape flutter.** A third modulation term, fast (order 100 Hz+),
in `FlutterModulator` - drawing from its **own** RNG stream, per the
rule above. Shared between the bounce bus's two channels like the rest
of the modulator.

**Its depth must be a real number chosen to be audible and
measurable.** A first review did the arithmetic on "fractions of a
cent": at 0.05 cents and 100 Hz the delay swing is ~0.002 samples,
below the Catmull-Rom interpolator's own error, putting sidebands near
-77 dB relative to carrier - beneath the default -66 dBFS hiss bed.
That is a stage that costs CPU and changes nothing. The implementing
task picks a depth from measurement, and the acceptance test measures
**sideband energy at f0 +/- the scrape rate on a hiss-free character**,
not a pitch histogram. `porta-testkit` has `pitch_track` and
`deviation_cents` (min/max only) and no pitch-deviation spectrum, so
that measurement is **its own testkit task**, scheduled before the
scrape task that depends on it.

**g. Dropouts.** Brief, shallow, rare level dips from oxide
imperfections, from their own seeded stream (REQ-702). Rate, depth and
duration are stated numbers in the implementing task.

**The test shape matters as much as the numbers.** With a fixed seed
the dropout positions are deterministic, so "a rate within a window" is
the wrong assertion for a single render: assert an **exact count and
exact positions for one named seed**, and a **rate window across N
stated seeds** for the statistical claim. `porta-testkit` has
`find_clicks` and `rms_envelope`, neither of which counts dips, so a
dip detector is part of the same testkit task as the scrape
measurement. The render length needed for a statistically meaningful
rate is long enough to be felt in CI, so the implementing task states
the length and whether the multi-seed test runs in the default gate or
is marked `[manual]`/ignored.

**h. Inter-track crosstalk.** A small amount of neighbouring-track
signal, as adjacent tracks on a physical 4-track cassette pick up each
other at the head.

**Crosstalk is architecturally different from a-g and is scheduled
last for that reason.** Everything else is a per-track effect that fits
inside `AudioProcessor` (mono, in-place). Crosstalk needs more than one
track's signal at once, so it cannot live in a track's own `Chain`.

But it does **not** follow that it belongs in `Engine` - a first review
corrected an earlier version of this paragraph that said so, and this
codebase already has the precedent: `StereoFlutter` is a porta-dsp type
that is deliberately *not* an `AudioProcessor` and is called directly
from the record path (change 001). Crosstalk follows that shape - it
**lives in porta-dsp**, owned by `TapeCharacter`/`TapeModel`, invoked
from the record path. Putting the coefficients in the engine would
mean REQ-704's "passthrough chain for engine testing" no longer gives
an uncoloured engine. No conflict with REQ-901 (the engine may hold
DSP; it must only stay hardware-free) and none with change 001's bus
(a bounce pass has no neighbouring tracks).

Two things the implementing task must settle: **which signal is the
source.** `playback[t]` during a multi-armed pass holds *post-chain
monitor* audio for other armed tracks and plain tape reads for the
rest, which would make crosstalk order-dependent and asymmetric
between two tracks recording simultaneously - so the source must be
named explicitly rather than taken as whatever is convenient. And
REQ-306 is preserved (nothing is written to unarmed tracks), which
should be asserted rather than assumed.

### What does not change

- The chain's **order** (saturation, hiss, bandwidth, flutter, crush),
  and the reasoning in `character.rs`'s module doc for why hiss sits
  before bandwidth, are unchanged. `Full` adds stages and makes
  existing ones level-aware; it does not reorder them.
- **REQ-701 does not currently describe the shipped chain, and the
  rewrite must fix that openly rather than quietly.** The spec says
  "saturation, bandwidth limiting, wow/flutter, hiss, then optional
  bitcrush"; `build_chain` actually builds `Saturation, Hiss,
  Bandwidth, Flutter, Crush`, and `character.rs`'s module doc argues
  the hiss-before-bandwidth order deliberately (hiss printed inside the
  passband is what makes generations pile up; after the filter, the
  next generation just removes it again). The code is right and the
  spec text is stale. `site/architecture.md` and its Spanish twin
  repeat the stale order too and need the same correction. Without
  this, "Simple is exactly today's chain" is ambiguous between two
  different orders.
- `TapeCharacter`'s existing fields keep their meaning. `clean()` stays
  near-transparent and stays the formulation tests use when they want
  mechanics without colour.
- Nothing reaches the audio callback that allocates or locks
  (REQ-902), and every added stage is per-sample so REQ-203 holds.

## Requirements affected

A first review noted this section named requirements without drafting
any, and it was right - nothing was reviewable as a requirement. Drafted
below. New DSP requirements take **7xx**, which section 4.7 (character
chain) already owns; REQ-701..704 exist, so these are REQ-705 onward.

**Amended:**

- **REQ-103**: gains the model. "A cassette's TapeCharacter (including
  noise seed) **and tape model** MUST be fixed at creation and stored
  in the project manifest."
- **REQ-701**: rewritten, and **corrected**: it currently lists an
  order the code does not build (see "What does not change"). New text
  states the real order and makes it model-dependent - under `Simple`,
  saturation, hiss, bandwidth limiting, wow/flutter and optional
  bitcrush, then TPDF dither; under `Full`, additionally head bump,
  hysteretic rather than memoryless saturation, signal-dependent hiss,
  level-dependent bandwidth, scrape flutter, dropouts and inter-track
  crosstalk.
- **REQ-702**: restated to cover the new stochastic elements
  (modulation noise, dropouts, scrape flutter), each from its own
  seeded stream, for both models - **and** to keep its cross-platform
  bound intact, which constrains the hysteresis solver (see item a).
- **REQ-905**: gains the two-model, worst-case obligation below
  (REQ-711).

**New:**

- **REQ-705**: A cassette's tape model MUST be one of `Simple` or
  `Full`. `Full` is the default for newly created cassettes. A manifest
  with **no** model field MUST load as `Simple`, so that a cassette
  recorded before this change never changes formulation. The model MUST
  be selectable only at creation.
- **REQ-706**: Under `Simple`, the record path MUST be bit-identical to
  the pre-004 implementation for the same input, character and seed -
  including the sequence of random draws. New stochastic stages MUST
  use their own independent streams.
- **REQ-707**: Under `Full`, the record path MUST NOT impose an output
  ceiling below full scale. Referenced to 0 VU = -18 dBFS: gain MUST be
  within 0.5 dB of unity below 0 VU, and THD at 0 VU MUST fall between
  1% and 3%.
- **REQ-708**: Under `Full`, the record path MUST apply a low-frequency
  resonance ("head bump") such that band energy around 60-80 Hz is
  above the 200 Hz reference, while 40 Hz remains below it - a bump,
  not a shelf.
- **REQ-709**: Under `Full`, the printed noise floor MUST rise with
  programme level (modulation noise): measured in a gap following loud
  programme it MUST be measurably above the floor following silence, by
  a stated margin. (It is presently identical to 0.01 dB.)
- **REQ-710**: Under `Full`, high-frequency response MUST decrease with
  increasing programme level (self-erasure), measurable as lower HF
  band energy relative to the fundamental for a hot input than a quiet
  one from the same source.
- **REQ-711**: The Pi headroom measurement (REQ-905) MUST cover both
  models, for record and bounce, at the **worst case of four
  simultaneously armed tracks** - not the two-chain bounce case. If
  `Full` does not fit at 128-256 frames, `Simple` becomes the Pi's
  creation default, and REQ-705's "default is Full" is then
  platform-dependent by explicit exception rather than by accident.
- **REQ-712**: Under `Full`, adjacent tracks MUST exhibit inter-track
  crosstalk at a stated level. Crosstalk MUST NOT write to unarmed
  tracks (REQ-306 is preserved).

**Untouched**: REQ-104, REQ-301, REQ-302, all of 4.4 (the bus prints
through whatever model the cassette has), REQ-703 (which stays
load-bearing - see item a's iteration cap and stability constraint),
REQ-901, REQ-902, and section 2 (this adds no capability and no control
surface beyond one creation-time flag). **REQ-904**: `Full`'s added
per-track state is a handful of filter and solver variables - negligible
against the tape buffers, and noted here only so it is not left
unaddressed.

**REQ-804 (session scripts)**: `Op::New` gains an optional `model`
field. Without it, scripts keep working and get the creation default.

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
- **Hysteresis**: "different output depending on trajectory" is passed
  by *any* stateful stage - a biquad passes it - so a first review
  correctly rejected it as a discriminator. The real assertion is
  **minor-loop width**: on a slow triangle of amplitude A, measure
  `|out(rising at x) - out(falling at x)|` at `x = A/2`; it MUST exceed
  a stated margin, and MUST **grow with A**. A biquad's difference does
  not scale that way; hysteresis's does. Paired with a stated
  harmonic-structure assertion.
- **Scrape flutter**: sideband energy at `f0 +/- scrape rate`, on a
  hiss-free character, MUST exceed a stated margin - and MUST be absent
  under `Simple`. Depends on the testkit task above.
- **Dropouts**: for one named seed, an **exact count and exact
  positions**; across N stated seeds, a **rate window**. Needs the dip
  detector from the testkit task.
- **Both models bit-reproducible**: two renders, same seed, identical
  bytes, for `Full` and `Simple` (REQ-702).
- **Block-size invariance per new stage**, via
  `porta_dsp::testing::assert_block_size_invariant`. (Two earlier
  citations of REQ-203 here were wrong: REQ-203 is about *playhead
  position* under differing block sizes, not audio content. If spec.md
  should carry a DSP-level invariance requirement, that is worth
  proposing on its own rather than borrowing REQ-203.)
- **Allocator harness extended** (`tests/realtime_alloc.rs`) to a
  `Full` record pass and a `Full` bounce. REQ-902 is argued
  structurally again for a brand-new solver, and that harness exists
  precisely because structural arguments here have failed four times.
- **`Simple` is unchanged.** A first review found the original plan for
  this incoherent, and the fix changes how the golden is handled:
  - The **existing golden is pinned to `Simple`** (its script gains
    `"model": "simple"`), so it never needs re-blessing at all and
    stays a standing regression check on the preserved path.
  - A **second, `Full` golden is blessed once**, at the end of the
    milestone. Otherwise the default flipping at task (b) would move
    the single golden again at (c), (d), (e), (f), (a) and (g) - seven
    re-blessings, not the promised one.
  - The preservation assertion itself is **bit-exact equality of an
    in-crate DSP render on one machine**, not "byte-identical to
    today's golden": once the golden moves there is nothing left to
    compare against, and byte-identity across platforms was never what
    REQ-702 promised anyway - which is exactly why `golden.rs` carries
    `TOLERANCE = 3`.
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
  measurement (REQ-711). Two corrections a first review forced:
  - **The worst case is four `Full` chains in one callback**, not two.
    M6.2's existing clause covers the bounce (two chains); an ordinary
    record pass with all four tracks armed runs four.
  - **The sequencing is uncomfortable and should be stated rather than
    hidden**: M6.1 and M6.2 are both still open and M4's hardware item
    is `[!]`, so `Full` would become the creation default before anyone
    has measured it on the deployment target. Either M6.2's two-model
    measurement lands **before** the default flips, or the default
    flips knowingly on an unmeasured target. The former is preferable
    and costs only ordering.
  - If `Full` does not fit, the honest consequence is **not** just
    "Simple becomes the Pi's default": it makes the creation default
    platform-dependent (the same command produces differently-sounding
    cassettes on different machines - REQ-711 makes that an explicit
    exception rather than a surprise), and it leaves a `Full` cassette
    **unrecordable on the Pi**. A project tracked on the Mac could be
    played, mixed, bounced and exported there but not overdubbed. That
    is a real limitation, not a footnote. The alternative shape - raise
    the period on the Pi and keep `Full`, as M6.2 already contemplates
    for the bus - should be weighed against it at measurement time.
- `docs/manual-checklist.md` gains the listening pass, marked
  `[manual]` in the verification list above.
- **A porta-testkit task, scheduled first**, for the two measurement
  tools that do not exist yet: sideband energy for scrape flutter, and
  a dip detector for dropouts. The scrape and dropout tasks depend on
  it.
- **Existing tests calibrated to the current transfer curve will need
  re-tuning**, and naming them now avoids discovering them one failure
  at a time: `character.rs`'s `default_character_colours_the_signal`,
  `default_character_kills_the_top_end` and `hiss_reaches_the_output`;
  the `saturation.rs` suite; `generation_loss.rs`'s thresholds; and
  `bounce_acceptance.rs::hot_generations_engage_the_quantize_clamp`,
  whose stated rationale becomes obsolete once the ceiling is removed.
- **Milestone and sequencing**: this becomes **M8**, after change 003's
  work (which is approved and unqueued). It does not depend on 003, but
  running them concurrently would put two golden-affecting changes in
  flight at once.
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
default with a single golden re-bless.

**v2 (this revision)**: a first review returned REVISE with fifteen
findings; all are addressed. The most consequential were factual
errors in v1, each verified against the code before fixing:

- **"Existing cassettes will replay differently" was false.** Chains
  run only in the recording and bouncing branches of `process_block`;
  every other path is a bare `tape.read` of post-chain i16. Recorded
  audio replays bit-identically forever - only new passes and script
  renders change. v1 argued this correctly in one section and
  contradicted it in another.
- **The manifest default would have broken REQ-103.** Following
  `character`'s `#[serde(default = ...)]` pattern with `Full` would
  have silently switched every pre-004 cassette's formulation on its
  next overdub. A missing field now MUST load as `Simple`; `Full` is
  the *creation* default only (REQ-705).
- **`Simple`'s preservation would have broken by construction.**
  `FlutterModulator` has one xorshift stream drawing once per sample,
  so folding scrape flutter into it shifts the walk even at zero depth.
  New stochastic stages now MUST use their own streams, and `Simple`'s
  draw sequence is normative (REQ-706).
- **"One golden re-bless" was wrong** - flipping the default early
  would have moved it at seven separate tasks. The existing golden is
  now pinned to `Simple` (so it never moves and stays a regression
  check on the preserved path) and a second `Full` golden is blessed
  once at the end.
- **REQ-702 was the underplayed risk.** An iterative state-feedback
  solver accumulates the last-bit libm differences REQ-702 tolerates,
  which a memoryless `tanh` cannot - and `golden.rs`'s 3 LSB tolerance
  is exactly what would fail cross-platform. Position taken: constrain
  the solver to bit-exact-portable operations with a table+polynomial
  Langevin term, rather than weaken the guarantee.

Also: v1 drafted no requirement text at all (REQ-705..712 now drafted,
plus amendments to REQ-103/701/702/905 and a REQ-804 note); REQ-701 was
found to already describe an order the code does not build, corrected
openly rather than quietly; the THD target had no operating reference
(now 0 VU = -18 dBFS); the hysteresis discriminator was one any biquad
would pass (now minor-loop width, growing with amplitude); scrape
flutter at "fractions of a cent" was computed to sit below the
interpolator's own error and beneath the hiss bed; the dropout and
scrape tests needed testkit tools that do not exist (now their own
task, scheduled first); crosstalk was placed in `Engine` when
`StereoFlutter` is the codebase's own precedent for DSP the engine
calls directly; REQ-203 was cited twice for a property it does not
describe; the Pi worst case is four chains, not two; and the
consequences of a `Simple`-on-Pi fallback are now stated plainly.
Ready for a second review.
