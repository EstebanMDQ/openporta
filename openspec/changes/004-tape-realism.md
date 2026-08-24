# 004: A tape model that behaves like tape

> **Reading this document**: sections 1-7 are **normative** - the
> proposal as it stands. All review history, corrected errors and
> rationale-for-changes live in **section 8 (History)**. Nothing in
> 1-7 argues with an earlier revision. This split is deliberate: v4
> carried its own audit trail inline, and a fourth review found that a
> reader could not tell a requirement from a note about a requirement.

## 1. Motivation

Requested directly by the owner: "it only adds some flutter and noise
... there might be better ways to be more subtle and get better
results." Measured before proposing, because that impression turns out
to have three specific, quantifiable causes rather than being a matter
of taste.

(`porta_testkit::signal::sine` takes **peak** dBFS, so raw RMS readings
sit 3.01 dB below the nominal figure. Everything below is corrected for
that. "Gain" throughout this document means **fundamental-band gain**,
not broadband RMS; the two differ by up to 0.25 dB at the hot end, and
REQ-713's own anchor is the fundamental.)

### 1.1 The chain hard-limits at about -9 dBFS peak, and over-distorts getting there

| input (peak) | real gain | THD |
|---|---|---|
| -40 dB | 0.00 dB | 0.01% |
| -18 dB (0 VU) | -0.27 dB | 1.02% |
| -12 dB | -0.99 dB | 3.72% |
| -6 dB | -3.14 dB | 11.28% |
| 0 dB | **-7.43 dB** | **23.88%** |

(THD here is measured **on the saturation stage in isolation**, per
REQ-714's measurement point. Read through the full default chain the
figures come out 9-10 dB lower, because 12-cent flutter smears the
fundamental over +/-7 Hz against `thd_db`'s +/-2 Hz window - the same
effect that makes REQ-714 specify a flutter-free measurement.)

`Saturation` is `tanh(x * drive) * makeup` with `makeup = 1/drive`.
At the default `drive_db: 9.0` that is a ceiling of `1/2.818 = 0.355`
- **-9 dBFS, whatever you feed it.** A real cassette at 0 VU runs
about 1-3% THD; this reaches **24%** *while also* pulling the level
down 7.4 dB. That combination is heard as small and dull, not as saturated.
It is also why the clamp test in `tests/bounce_acceptance.rs` had to
use a low-drive character: on the default formulation the i16 clamp is
mathematically unreachable, because saturation gets there first.

### 1.2 There is no head bump - the low end is removed rather than added

Measured at -18 dBFS: **40 Hz is -8.1 dB**, 60 Hz -3.3 dB, 80 Hz
-1.5 dB. Real cassette geometry produces a broad **+2 to +4 dB rise
around 50-100 Hz** before rolling off below it - a large part of why
tape is described as "fat". `Bandwidth`'s 60 Hz high-pass does the
opposite of the thing that makes tape sound like tape.

### 1.3 The noise floor is perfectly static

Silence: **-72.73 dBFS**. The tail after a -6 dBFS tone: **-72.74
dBFS**. Identical to two decimal places. Real tape's noise is made by
the same magnetised particles carrying the signal, so it **rises and
falls with the programme** - modulation noise. A fixed bed of hiss
sitting behind the music is exactly the "it adds some noise"
impression.

### 1.4 Structurally

`Saturation` is **memoryless**. Real tape is hysteretic: the output
depends on magnetic history, not only the present sample. That is the
single largest difference between "sounds like tape" and "sounds like
a waveshaper", and no amount of tuning `drive_db` reaches it.

Also absent: level-dependent HF loss (tape self-erases highs when hit
hard), scrape flutter (the fast grain of tape dragging across the
heads - we model wow and flutter but not this), dropouts, and
inter-track crosstalk.

## 2. Owner decisions (asked directly, 2026-08-24)

- **Build the full model**, including hysteresis, scrape flutter,
  dropouts, level-dependent HF loss and crosstalk.
- **Keep the current, cheaper model available behind a flag**, so
  constrained devices can still record.
- **The improved model becomes the default** for newly created
  cassettes.

Already-recorded audio is unaffected: `Engine::process_block` runs a
chain only in the recording and bouncing branches, every other path is
a bare `tape.read`, and tape holds post-chain i16. What changes is new
passes and script renders, which is why the golden moves at all.

## 3. Change

### 3.1 Two models, chosen per cassette

A new `TapeModel` with two values:

- **`Full`** - everything in 3.2. The creation default.
- **`Simple`** - exactly today's chain, unchanged, bit-for-bit. Not a
  degraded version of `Full` but the existing code path preserved, so
  it stays cheap and stays a known quantity.

`TapeModel` joins `TapeCharacter` in the manifest and is **fixed at
creation**, for the same reason REQ-103 fixes the character: a cassette
must sound like itself for its whole life. `porta-app new` gains
`--model simple|full`. The flag is **creation-only and never a UI
control** - it is a property of the cassette, not a quality preference
to be toggled while working.

`model` lives on `Manifest`, beside `character`, not inside
`TapeCharacter`: character is a knob-set `clean()` can zero, model is
an orthogonal formulation choice. A **missing `model` field
deserializes as `Simple`** (REQ-705), never as the creation default.
A pre-004 binary opening a `Full` cassette ignores the unknown field
and renders it as `Simple` - no error, the old sound. Acceptable at
v0.1.x; worth a release-note line.

**Fixing the model at creation does not strand a weak device.**
Degradation is baked at record time and the playback path stays clean
(REQ-303). The model's cost is paid **only while recording or
bouncing**: any cassette, `Full` or `Simple`, plays back on any device
at the same cost as today. A constrained device that needs to *record*
creates `Simple` cassettes, and can still play, mix and export a
`Full` one made elsewhere.

### 3.2 What `Full` adds

Each item states what it models physically, because that is what makes
its parameters arguable rather than arbitrary.

**`Full` adds stages and makes existing ones level-aware; it does not
reorder them.** This is what disambiguates "`Simple` is exactly today's
chain" on the `Full` side: the two models share a stage order, and the
new stages take stated positions within it (REQ-701, REQ-402).

**Two rules span all of a-h.**

**Rule 1 - separate RNG streams.** Every new stochastic stage MUST draw
from **its own** stream, and under `Simple` both the executed code path
*and the sequence of RNG draws* MUST be exactly today's (REQ-706).
`FlutterModulator` holds a single xorshift state consuming exactly one
draw per sample, so folding scrape flutter into it would shift the
random walk **even at zero depth** - the generator has already
advanced. `Hiss` has the same hazard for modulation noise.
`reseed_chain`'s hardcoded stage indices (`HISS_STAGE = 1`,
`FLUTTER_STAGE = 3`, `SPLIT_HISS_STAGE = 1`) become model-dependent,
and must cover the new stages (REQ-717).

**Rule 2 - bit-exact-portable feedback paths (REQ-715).** REQ-702
promises bit-reproducible renders and `golden.rs` polices it at
`TOLERANCE = 3` LSBs. A memoryless `tanh` cannot accumulate libm's
last-bit platform differences; **any stateful stage can**, because the
difference re-enters its own state each sample. This governs all of
`Full`'s signal path, not only the hysteresis solver: (c) is a biquad,
(d) smooths an envelope, (e) is permitted to be a one-pole, (g) has
decay tails, and (h) can import a neighbour's drift.

**a. Hysteresis replaces the memoryless waveshaper.** A Jiles-Atherton
style solver: output depends on magnetic history. This is the item
that separates "tape" from "waveshaper". Constraints: the Langevin
term evaluated from a **compile-time literal** table plus polynomial,
never libm `coth`/`exp` at startup; the existing
`output_is_bounded_and_finite_under_abuse` test MUST pass unchanged;
and the implementing task states both a **fixed iteration cap** and a
**per-sample transcendental count**, the second because REQ-902 bounds
callback work and REQ-905 bounds it on the Pi specifically - an
unbounded solver is a realtime hazard regardless of its memory
footprint. REQ-703 stays load-bearing here.

**b. Restore headroom.** The saturator MUST NOT impose a fixed output
ceiling well below full scale.

This is **not** simply "decouple makeup from `1/drive`". For the
current form `tanh(d*x)*m`, small-signal gain is `d*m` and the
asymptote is `m`: REQ-713 forces `d*m ~ 1`, REQ-707 forces `m ~ 1`,
so `d ~ 1` - plain `tanh(x)`, which yields **0.132% THD at 0 VU**,
about 8x below REQ-714's window. **The tanh family cannot satisfy
REQ-707, REQ-713 and REQ-714 together.** A curve family that does, and
that satisfies REQ-715 as a bonus:

```
f(x) = x / (1 + |x|)
```

**`drive_db` selects the knee exponent, not a pre-gain.** This has to
be stated, because every other reading fails a requirement, and it is
the first question task (b) faces. Writing `n` for the exponent in
`f(x) = x/(1+|x|^n)^(1/n)`:

```
n = 10^((9 - drive_db) / 20)
```

so the default `drive_db: 9.0` gives `n = 1.0`, exactly the curve
below. The rejected alternatives, all measured: keeping drive as a
multiplier with `makeup = 1/drive` reproduces the **-9 dBFS ceiling**
this change exists to remove (and `x/(1+k|x|)` with `k = drive` is the
same construction written differently - `f(dx)/d`, asymptote `1/k`,
-9 dBFS again at the default); with `makeup = 1` it gives **+9 dB of
gain**, failing REQ-713 by 18x, since 3.2b's own `gain = d*m`,
`asymptote = m` argument applies to *any* drive-scaled family and not
only to tanh. Ignoring `drive_db` entirely
breaks `clean()`, which is load-bearing for REQ-716.

Under this mapping `clean()`'s `drive_db: -30.0` gives `n = 89`, which
is numerically a wire: **+0.0000 dB gain and 0.0000% THD at 0 VU**,
peak -0.07 dBFS. That is what keeps REQ-716's transparency claim true
and the punch-crossfade, REQ-306 and click-detector suites passing.
The mapping is monotone: `drive_db` 0 gives 0.03% THD, 6 gives 0.77%,
9 gives 2.01%, 12 gives 3.90%.

**The usable range is `drive_db <= 15` (`n >= 0.5`), and the mapping
MUST state it**, because outside it the knob inverts: today
`makeup = 1/drive` holds a -40 dBFS signal at exactly 0 dB for *any*
drive, whereas under the mapping `drive_db 18` attenuates it 4.1 dB and
`drive_db 24` attenuates it 17.3 dB, becoming an attenuator rather than
a distorter. Both shipped characters (default 9.0, `clean()` -30.0) sit
well inside the range. `saturation.rs`'s module doc, which explains
makeup as "1/drive keeps quiet material at its original level
regardless of drive", and `character.rs`'s `clean()` comment about
tanh at unity drive, both become wrong under `Full` and are listed in
6.3.

**REQ-707/713/714's windows are stated for the default character.** At
`drive_db: 12` the curve reads -0.906 dB at -30 dBFS, outside
REQ-713 - correctly, because a hotter character is *meant* to
saturate earlier. This is why section 4.3 pins `MEASURE_CURVE` to the
default drive; the requirements bound the formulation, not every
character a user can build.

Measured on the default curve: gain -0.007 dB at -60 dBFS and -0.230 dB at
-30 dBFS (REQ-713), **THD 2.01% at 0 VU** (REQ-714), asymptote exactly
1.0 and bounded under abuse (REQ-707).

**On `powf` and REQ-715 - a ruling, because v7 got this wrong twice.**
The family is transcendental-free **only at `n = 1`**, the default
operating point; every other character, `clean()` at `n = 89`
included, needs `|x|^n` and `(.)^(1/n)`. So "pure add/abs/divide, so
REQ-715 holds by construction" was false as a claim about the stage,
and the parallel rejection of the `n = 1.2` neighbour "because `powf`
is forbidden on a feedback path" was doubly wrong: **a memoryless
waveshaper is not a feedback path at all.** REQ-715 governs stateful
stages, where last-bit libm differences re-enter the state and
accumulate. It does not govern a memoryless curve, which is why the
chain has shipped a libm `tanh` since M1 without troubling REQ-702 -
per-sample rounding differences stay bounded and sit well inside
`golden.rs`'s 3 LSB tolerance.

The ruling therefore: **REQ-715 and its grep are scoped to stateful
and feedback stages.** `powf` in the memoryless nonlinearity is
permitted, on exactly the standing today's `tanh` has. Once item (a)
replaces that stage with a solver, the slot **becomes** a feedback
path and REQ-715 applies to it in full - which is what REQ-720's
inheritance clause is for. The implementing task is free to choose any curve meeting
REQ-707/713/714/715; this one is offered as an existence proof so the
task is not open-ended. Note the two requirements interact across the
family: scanning `n`, REQ-713's 0.5 dB bound is violated at `n = 0.8`
(-0.582 dB) while THD at 0 VU reaches only 3.17% there, so the
**jointly achievable THD range is about 1% to 2.9%** and REQ-714's
upper edge is not reachable within REQ-713. `x/(1+|x|)` at 2.01%
spends 46% of REQ-713's budget, so a curve choice should not be
treated as having 10x margin.

Two consequences. First, print levels rise, but **by 2-3 dB at the hot
end, not by the 7 dB the old curve costs at 0 dBFS**: against
`x/(1+|x|)` the change is +2.18 dB at 0 dBFS in, +0.09 dB at -6 dBFS
and **-0.61 dB at 0 VU** (peak output for a 0 dBFS sine goes -9.06 ->
-6.02 dBFS). The 7 dB figure describes what the current curve costs,
not the difference between curves. REQ-718 bounds the result so the
i16 clamp does not become the new limiter. Second,
`bounce_acceptance.rs::hot_generations_engage_the_quantize_clamp`'s
documented rationale - that the clamp is unreachable on a default
cassette because saturation gets there first - becomes obsolete and
must be revisited with the test.

**c. Head bump.** A peaking filter around 50-100 Hz, placed **after
saturation and ahead of the existing 60 Hz high-pass**. Models
playback-head/tape geometry resonance. REQ-708 states the window as a
**net** figure measured after that high-pass; since 60 Hz currently
sits at -3.3 dB, the raw peaking gain needed is materially larger than
the net figure, and that raw gain is what REQ-718 bounds.

**d. Modulation noise.** Hiss amplitude follows a smoothed envelope of
the signal, over a static bias-noise floor. Models noise from the
magnetised particles themselves. The envelope MUST be per-sample, never
per-block, or the render depends on block size.

**e. Level-dependent HF loss.** High-level content loses more top end
(self-erasure). The **mechanism must be specified, not just the
effect**: recomputing RBJ biquad coefficients per sample means a
`sin_cos` and divides per sample per track on the Pi, and
continuously-varying coefficients raise stability and zipper questions.
Acceptable implementations are a modulated one-pole, bounded
interpolation between two precomputed coefficient sets, or a
rate-limited coefficient update at a stated interval. Whichever is
chosen, its update rate MUST NOT make the result depend on block size.

**f. Scrape flutter.** A third modulation term, fast (order 100 Hz+),
in `FlutterModulator`, drawing from its **own** RNG stream per Rule 1.
Shared between the bounce bus's two channels like the rest of the
modulator. Its depth must be chosen to be audible and measurable: at
0.05 cents and 100 Hz the delay swing is ~0.002 samples, below the
Catmull-Rom interpolator's own error, putting sidebands near -77 dB
relative to carrier and beneath the default -66 dBFS hiss bed - a stage
that costs CPU and changes nothing. The implementing task picks a depth
from measurement.

**g. Dropouts.** Brief, shallow, rare level dips from oxide
imperfections, from their own seeded stream. Rate, depth and duration
are stated numbers in the implementing task.

**h. Inter-track crosstalk.** Signal bleeding between tracks adjacent
on the physical tape. `StereoFlutter` is the codebase's precedent for
DSP that spans channels and is called directly by the engine rather
than being an `AudioProcessor`; crosstalk follows that shape. It lives
in **porta-dsp**, not the engine: engine-side coefficients would break
REQ-704's passthrough chain for engine testing.

**The source signal MUST be named, because the obvious reading is
order-dependent.** `playback[t]` holds post-chain monitor audio for
other armed tracks and plain tape reads for the rest, so "bleed from
the neighbouring track" means different things depending on which
tracks are armed and in which order the engine visits them - and for
two tracks recording simultaneously it is asymmetric, each seeing a
different vintage of the other. The implementing task states the
source explicitly and asserts the symmetric case.

### 3.3 Every `Full` stage is parameterised on `TapeCharacter`

REQ-716. 23 call sites use `Engine::create()` and 30 use
`TapeCharacter::clean()`; because model is orthogonal to character, all
of them become `Full` renders once the default flips, including the
near-transparent `clean()` paths the punch-crossfade, REQ-306 and
click-detector suites rest on. Every `Full`-only stage therefore takes
a `TapeCharacter` parameter that `clean()` sets to its no-op value,
and **each new field carries its own `#[serde(default = ...)]`**.

Hysteresis (item a) is the exception, and it must be asserted rather
than assumed: it *replaces* the saturator instead of adding a stage, so
it has no parameter of its own to zero, and `clean()`'s transparency
under `Full` rests on a J-A solver at `drive_db: -30.0` being as
transparent as `tanh` at -30 dB.

### 3.4 What does not change

- `Simple` is the existing path, bit-for-bit, including its RNG draw
  sequence (REQ-706).
- Recorded audio replays identically forever; only new passes and
  script renders differ.
- `TapeCharacter`'s existing fields keep their meaning. `clean()` stays
  near-transparent and stays the formulation tests use when they want
  mechanics without colour.
- Nothing reaching the audio callback allocates or locks (REQ-902).
  Every added stage is per-sample, so each is invariant to block size.
- The undo journal is untouched: record and bounce passes stay
  journal-covered under both models, and the model does not enter the
  journal.

## 4. Requirements affected

Listed in **strict id order** in each group, so the section can be
audited by reading down.

### 4.1 Amended

- **REQ-103** (character fixed at creation): extended to cover the
  model. A cassette's model is fixed at creation and never changes.
- **REQ-402** (bounce bus per-channel stages): **amended**, because it
  enumerates the stage set verbatim and would go stale exactly as
  REQ-701's list did. Drafted replacement:

  > Each bus channel is printed through its own chain of saturation
  > (or, under `Full`, hysteresis), modulation noise over hiss,
  > head bump, bandwidth and level-dependent HF loss, with **wow,
  > flutter and scrape flutter shared** between L and R from one
  > modulator, and optional bitcrush applied after. Under `Full`,
  > **dropouts are also shared** between L and R from a single stream.
  > Inter-track crosstalk does not apply to the bus, which has no
  > adjacent tracks.

  Concretely against `build_split_chain`, whose `pre` half is today
  `[Saturation, Hiss, Bandwidth]` and whose `post` half is `[Crush?]`:
  head bump and level-dependent HF loss join the **pre** half, and
  modulation noise (d) is part of the `Hiss` stage there. Scrape
  flutter (f) is a term inside the **shared middle** modulator, not in
  either half. Dropouts are shared and sit with the middle.

  Shared bus dropouts are the one non-obvious case, and are settled
  here: a dropout models a physical defect at a tape position, and a
  defect does not strike the left stripe and the right stripe at
  different moments. Independent per-channel seeding would produce a
  stereo flicker no tape makes. The shared stream is **seeded at
  channel term 0**, the same fixed convention
  `build_stereo_flutter` already documents for flutter, so
  REQ-702's bit-reproducibility does not depend on an implementation
  coin-flip.
- **REQ-701** (record chain stages): rewritten for two models, and
  **corrected**. Its current text lists "saturation, bandwidth,
  **wow/flutter, hiss**, optional bitcrush, then TPDF dither";
  `build_chain` actually builds
  `Saturation, Hiss, Bandwidth, Flutter, Crush`, and `character.rs`
  argues the hiss-before-bandwidth order deliberately (hiss printed
  inside the passband is what makes generations pile up; after the
  filter the next generation just removes it again). The code is right
  and the spec text is stale - and hiss is **two** positions out, not
  one. Note also that **spec.md is internally inconsistent today**:
  REQ-402 already reads "saturation, hiss, bandwidth, optional crush",
  matching the code, while REQ-701 does not. The amendment brings 701
  into line with both 402 and the code. `site/architecture.md` and its Spanish
  twin repeat the stale order and need the same correction. Drafted
  replacement:

  > Under `Simple`, a record pass prints through saturation, hiss,
  > bandwidth, wow/flutter and optional bitcrush, **then TPDF dither
  > before i16 quantization**. Under `Full`, the same order with
  > hysteresis in place of saturation, modulation noise as part of the
  > hiss stage, head bump between hysteresis and bandwidth,
  > level-dependent HF loss in the bandwidth stage, scrape flutter as a
  > term of the wow/flutter modulator, and dropouts after it -
  > **then TPDF dither** as before. Neither model reorders the others.

  The dither stage is called out because it is the one part of the
  chain this change must not touch, and it was absent from v5's
  description entirely.
- **REQ-702** (bit-reproducibility): restated to cover the new
  stochastic elements (modulation noise, dropouts, scrape flutter),
  each from its own seeded stream, for both models - and to keep its
  cross-platform bound intact, which REQ-715 is what enforces. **If
  bit-exact portability proves impossible in practice, the fallback is
  an explicit amendment to REQ-702 and to `golden.rs`'s tolerance - a
  spec change in its own right, not something to absorb silently.**
  That matters because REQ-715's only end-to-end enforcement is a
  `[manual]` cross-platform check.
- **REQ-703** (bounded per-sample work) and **REQ-704** (chain
  swappability for engine testing): both stay load-bearing and both
  live in spec.md **section 4.7**, not section 5 - so 4.4's "all of
  section 5 except REQ-905" does not account for them. REQ-703 governs
  item (a)'s iteration cap and transcendental count; REQ-704 is why
  crosstalk sits in porta-dsp.
- **REQ-804** (`Op::New` script fields): gains an **optional** `model`
  field. Optional matters - without it, existing scripts keep working
  and keep rendering as they do today.
- **REQ-905** (Pi headroom): gains the two-model obligation in REQ-711.
- **spec.md section 6** (acceptance gates): **M3's gate** says "the
  single golden render passes" and becomes both goldens. **M4's gate**
  says "block-size invariance (REQ-203)", which is a conflation -
  REQ-203 is about playhead position, not audio content - and is worth
  fixing in the same amendment, or splitting out as the DSP-level
  invariance requirement this proposal declines to borrow.

### 4.2 New

- **REQ-705**: A cassette's tape model MUST be one of `Simple` or
  `Full`, fixed at creation. `Full` is the default for newly created
  cassettes. **A manifest with no `model` field MUST load as
  `Simple`.**
- **REQ-706**: Under `Simple`, the record path MUST be bit-identical to
  the pre-004 implementation, including the per-sample RNG draw
  sequence of every stochastic stage. Verified against a frozen
  reference committed to the repo.
- **REQ-707**: Under `Full`, the record path MUST NOT impose an output
  ceiling below full scale. Concretely, on the saturation stage in
  isolation - meaning **whatever stage occupies the nonlinear slot**:
  the saturation curve under `Simple` and under the intermediate
  `Full` build, and the hysteresis solver once item (a) lands (see
  REQ-720) - **a 0 dBFS sine MUST produce peak output at or above
  -7 dBFS**, and output MUST continue to rise with input above that.
  The current saturator caps at **-9.06 dBFS** for any input whatever
  and fails this; `x/(1+|x|)` gives -6.02 dBFS and passes.
  `output_is_bounded_and_finite_under_abuse` still bounds the top,
  which together with this pins any candidate curve's asymptote at
  essentially exactly 1.0. The whole-path consequence is REQ-718.

  **REQ-707 is the discriminating requirement for item (b); REQ-713
  and REQ-714 are guard rails.** At their own measurement point the
  *current* saturator already passes both (-0.017 dB at -30 dBFS,
  1.017% THD at 0 VU), so neither can serve as an acceptance criterion
  for the change - their job is to stop the fix from breaking the
  quiet end or overshooting into fizz.
- **REQ-708**: Under `Full`, the record path MUST apply a
  low-frequency resonance ("head bump") such that band energy at
  60-80 Hz sits **2 to 4 dB above** the 200 Hz reference, while 40 Hz
  remains below it - a bump, not a shelf. The figure is **net**,
  measured after the existing 60 Hz high-pass, and matches the +2 to
  +4 dB of section 1.2.
- **REQ-709**: Under `Full`, the printed noise floor MUST rise with
  programme level (modulation noise). Measured as broadband RMS over a
  **100 ms window beginning 20 ms after** a 1 second 0 VU tone ends, it
  MUST sit **3 to 12 dB above** the same measurement following 1 second
  of silence. The offset and window are part of the requirement: the
  reading depends entirely on item (d)'s envelope release, which is
  otherwise unspecified, so without them the window can be hit or
  missed by choosing a release time. (The floor is presently identical
  to 0.01 dB.)
- **REQ-710**: Under `Full`, high-frequency response MUST decrease with
  increasing programme level (self-erasure). For a **1 kHz + 8 kHz
  two-tone source**, the 8 kHz component measured over
  **6-10 kHz** relative to the 1 kHz component measured over
  **0.8-1.2 kHz** MUST be **at least 2 dB lower** at 0 dBFS input than
  at -30 dBFS input.
- **REQ-711**: The Pi headroom measurement (REQ-905) MUST cover both
  models, for record and bounce, at the **worst case of four
  simultaneously armed tracks** - not the two-chain bounce case. This
  is a wall-clock measurement on hardware and is `[manual]`.
- **REQ-712**: Under `Full`, tracks adjacent on the physical tape
  (**1-2, 2-3, 3-4 - not 1-4**) MUST exhibit inter-track crosstalk
  between **-60 and -40 dB** relative to the source track, measured as
  broadband RMS over a **1 second** 0 VU 1 kHz tone on
  **`MEASURE_NO_HISS`** (at -60 dB relative to a 0 VU source the
  bleed sits at -78 dBFS, 12 dB below the default -66 dBFS hiss bed,
  so the window's lower half is unmeasurable on a default cassette).
  Crosstalk MUST NOT write to unarmed tracks (REQ-306 is preserved).
- **REQ-713**: Under `Full`, gain MUST be within 0.5 dB of unity for
  inputs from **-60 dBFS to -30 dBFS**, measured **at 1 kHz on the
  nonlinear stage in isolation, after discarding a stated settling
  interval**. The settling clause is inert for a memoryless curve and
  decisive once REQ-720's solver occupies the slot: a reading taken
  from a demagnetized state differs from the steady-state minor loop. The measurement point is part of the
  requirement: on the whole record path this would be contradicted
  outright by REQ-708, which mandates +2 to +4 dB at 60-80 Hz. The
  range is bounded downward at -60 dBFS because the requirement is
  about the formulation's curve, and there is no useful reading below
  it once the stage is embedded in a chain.
- **REQ-714**: Under `Full`, THD MUST fall between **1% and 3% at
  0 VU (= -18 dBFS)**, measured **at 1 kHz on the nonlinear stage in
  isolation, after discarding the same settling interval as REQ-713**,
  on a character with `flutter_depth_cents: 0.0` and hiss disabled. The measurement point is part of the requirement:
  `thd_db` sums each partial over +/-2 bins (+/-2 Hz at 48 000
  samples) while the default character's 12-cent flutter smears a
  1 kHz fundamental by +/-6.9 Hz and its 7th harmonic by +/-48 Hz, so
  on a fluttering signal the reading is meaningless. The hiss bed
  matters for the same reason at quiet levels: read through the full
  default chain, THD below about -24 dBFS is dominated by the -66 dBFS
  bed rather than by distortion. (Section 1.1's table is measured at
  this requirement's own point, so it shows neither effect.)
- **REQ-715**: Under `Full`, no operation on any feedback path may be
  other than bit-exact-portable across IEEE-754 platforms (add,
  subtract, multiply, divide, sqrt). Specifically: the Langevin table
  MUST be **compile-time literal constants** (a table built at startup
  from libm `exp`/`coth` is itself platform-dependent);
  **`f32::mul_add` MUST NOT appear on a feedback path** (FMA
  contraction differs between the aarch64 of the dev Mac and
  deployment Pi and the x86-64 of Linux CI); and the implementation
  MUST state a **denormal (FTZ/DAZ) policy**, since decaying feedback
  state generates denormals and FTZ is host-settable on both.
- **REQ-716**: Every `Full`-only stage MUST be parameterised on
  `TapeCharacter`, and `TapeCharacter::clean()` MUST set every such
  parameter to its no-op value. **Each new `TapeCharacter` field MUST
  carry its own `#[serde(default = ...)]` resolving to that no-op
  value**, without which this requirement is a hard load failure on
  every existing cassette rather than a behaviour change:
  `TapeCharacter`'s fields carry no defaults today, and `Manifest`'s
  `character` carries a **field-level**
  `#[serde(default = "TapeCharacter::default")]` (there is no
  struct-level attribute on `TapeCharacter` - an implementer grepping
  for one will not find it) which fires only when the `character` key
  is absent entirely, which it never is on a written manifest.
- **REQ-717**: Under `Full`, every new stochastic stage (modulation
  noise, dropouts, scrape flutter) MUST be reseeded per record pass,
  as REQ-304 already requires of flutter and hiss. Without this every
  pass on a cassette gets identical dropout positions - audible, and
  it would distort REQ-403's generation-loss compounding.
- **REQ-718**: Under `Full`, peak output of the **whole record path**
  MUST remain below full scale for programme whose **peak** reaches
  **0 dBFS** - not 0 VU. The anchor level is part of the requirement:
  "0 VU" left peak-versus-RMS unstated, and the two differ by the
  crest factor (10-18 dB), which is the entire margin at issue. Read as
  a peak, 0 VU through a ceiling-free curve prints near -19 dBFS and
  even with REQ-708's bump has ~15 dB of headroom, so the requirement
  would never bite; the hazard lives at the hot end. The test programme
  MUST be bass-heavy, since the interaction being bounded is
  low-frequency gain applied after the saturator. This bounds the interaction
  REQ-707 and REQ-708 create together: removing the saturator's
  ceiling and then adding low-frequency gain **after** it would let
  bass-heavy material reach the clamp first, reinstating a hard
  limiter by the back door.
- **REQ-719**: If REQ-711's measurement shows `Full` does not fit at
  128-256 frames, the creation default MAY be platform-dependent, by
  this explicit exception rather than by accident. That `cfg` decision
  lives in **porta-app**, not the engine (REQ-901).
- **REQ-720**: Under `Full`, the record path MUST exhibit **magnetic
  hysteresis**, **and the hysteresis solver MUST itself satisfy
  REQ-707, REQ-713 and REQ-714**. The second clause is what stops item
  (a) from silently undoing item (b): hysteresis *replaces* the
  waveshaper, so whatever curve task (b) chooses is discarded when (a)
  lands, and a solver that reintroduced a -9 dBFS ceiling would pass
  the minor-loop test below without complaint. The transfer-curve
  tests are therefore run **twice**, once at (b) and again at (a).

  **The solver MUST also define what `drive_db` scales, and MUST be
  transparent at `clean()`.** This is B2's finding one item over, and
  it is the one that ships, since (b)'s curve is discarded when (a)
  lands: 3.2b's knee-exponent mapping is defined for a curve family
  that has a knee exponent, and a J-A solver has none. The implementing
  task names the solver parameter `drive_db` scales (loop area, or the
  input scaling ahead of it), and asserts that **`drive_db: -30.0` is
  transparent to a stated margin** - the property REQ-716, the
  punch-crossfade suite, REQ-306 and the click detector all rest on.
  The solver MUST additionally stay bounded and finite under abuse:
  `output_is_bounded_and_finite_under_abuse` is a `Saturation` unit
  test and `Saturation` survives only on the `Simple` path, so REQ-707
  gives the solver a floor and nothing gives it a ceiling unless this
  clause does.

  Concretely: on a slow triangle sweep of amplitude A, the minor-loop
  width `|out(rising at x) - out(falling at x)|` measured at `x = A/2`
  MUST exceed a stated margin, and MUST **grow with A**. The
  growth-with-amplitude clause is the discriminator - a linear filter
  also produces a nonzero difference between rising and falling
  segments, but its width does not scale with amplitude that way. Paired
  with an odd-harmonic structure assertion, this is what distinguishes
  hysteresis from "a biquad in the signal path".
- **REQ-721**: Under `Full`, scrape flutter MUST produce **sideband
  energy at f0 +/- the scrape rate** exceeding a stated margin on a
  hiss-free character, and MUST be absent under `Simple`.
- **REQ-722**: Under `Full`, dropouts MUST occur at a stated rate,
  depth and duration. REQ-720, REQ-721 and REQ-722 each defer their
  numeric threshold to the implementing task; **the measured margin is
  folded back into the requirement text when that task lands**, so
  none of them stays permanently unfalsifiable. Specifically: for one named seed, an exact count and exact
  positions; across N stated seeds, a rate window.

### 4.3 Measurement characters

Three requirements measure on a character that is quiet in one respect
but **not** `clean()`. This must be stated because REQ-716 makes
`clean()` zero every `Full`-only parameter, including the very stage
being measured - `clean()` would assert nothing, the same trap the
block-size bullet already flags for crosstalk.

- **`MEASURE_NO_HISS`**: default character with `hiss_dbfs: -140.0`,
  everything else at default. Used by REQ-712 and REQ-721.
- **`MEASURE_CURVE`**: `flutter_depth_cents: 0.0` and
  `hiss_dbfs: -140.0`, drive at default. Used by REQ-707/713/714.

Both are test fixtures in `porta-dsp`, not new presets on the public
`TapeCharacter` API.

### 4.4 Untouched

REQ-104, REQ-301, REQ-302, REQ-303, the rest of 4.4 (the bus prints
through whatever model the cassette has), all of section 5 except
REQ-905, and spec.md section 2's scope list.

## 5. Verification (headless, REQ-906)

Every requirement in 4.2 has a bullet here. Where a bullet states a
level or window, it MUST match the requirement verbatim; after any
numeric change to a requirement, **grep this section for the old
value**.

- **Simple preservation (REQ-706)**: an in-crate render compared
  **bit-exactly against a frozen reference committed to the repo**,
  captured from the pre-004 build. The reference names its render or it
  is not reproducible: both `build_chain` **and** `build_split_chain`,
  on the **default** character - explicitly not `clean()`, whose
  `flutter_depth_cents: 0.0` and `hiss_dbfs: -140.0` make it blind to
  precisely the two RNG streams REQ-706 protects - plus stated input
  signal, length, seed and block size.
- **Hysteresis (REQ-720)**: a slow triangle sweep at two or more
  amplitudes; minor-loop width at `x = A/2` above a stated margin and
  **monotonically increasing with A**; plus odd-harmonic structure.
  Absent under `Simple`. This is the acceptance test for item (a), the
  largest item in the change - v5 shipped it with no requirement and no
  test at all.
- **Transfer curve (REQ-707, REQ-713, REQ-714)**, run **twice** - at
  task (b) on the curve and again at task (a) on the solver, per
  REQ-720 - **discarding a stated settling interval** before measuring:
  at -60, -40, -30,
  **-18 (0 VU)**, -12, -6 and 0 dBFS, **at 1 kHz on the saturation
  stage in isolation**, on a character with `flutter_depth_cents: 0.0`
  and hiss disabled: gain within 0.5 dB of unity **from -60 to -30
  dBFS**, THD **between 1% and 3% at 0 VU**, and **peak output at or
  above -7 dBFS for a 0 dBFS sine** (REQ-707). The current saturator
  fails **REQ-707 only** - it caps at -9.06 dBFS - and already passes
  REQ-713 (-0.017 dB at -30 dBFS) and REQ-714 (1.017% at 0 VU) at this
  measurement point, so those two are guard rails rather than
  acceptance criteria. (v5 claimed "the current chain fails all three",
  which is false and was carried over from when the measurement point
  was the whole record path.)
  `output_is_bounded_and_finite_under_abuse` MUST also pass unchanged.
- **Whole-path peak (REQ-718)**: bass-heavy programme at 0 VU through
  the **full `Full` record path**, asserting peak stays below full
  scale and the i16 clamp is not engaged. Stage isolation cannot
  assert this - it is precisely a property of the head bump following
  a ceiling-free saturator.
- **Head bump (REQ-708)**: band energy at 60-80 Hz **2 to 4 dB above**
  the 200 Hz reference and 40 Hz below it, measured **after** the
  existing high-pass.
- **Modulation noise (REQ-709)**: broadband RMS over a **100 ms window
  beginning 20 ms after** a 1 second 0 VU tone ends, **3 to 12 dB
  above** the same measurement after 1 second of silence.
- **Level-dependent HF loss (REQ-710)**: **1 kHz + 8 kHz two-tone**
  source; 8 kHz over **6-10 kHz** relative to 1 kHz over
  **0.8-1.2 kHz**, **at least 2 dB lower** at 0 dBFS input than at
  -30 dBFS.
- **Crosstalk (REQ-712)**: broadband RMS over a **1 second** 0 VU
  1 kHz tone; bleed into each adjacent track (1-2, 2-3, 3-4)
  **between -60 and -40 dB** relative to source, on
  **`MEASURE_NO_HISS`**; no bleed into non-adjacent track 4 from
  track 1; and **no write to unarmed tracks**, which is REQ-306 and is
  asserted directly.
- **Scrape flutter (REQ-721)**: **sideband energy at f0 +/- the scrape
  rate on a hiss-free character**, not a pitch histogram, exceeding a stated
  margin - and absent under `Simple`. `porta-testkit` has `pitch_track`
  and `deviation_cents` (min/max only) and no pitch-deviation
  spectrum, so this needs the testkit task below.
- **Dropouts (REQ-722)**: for one named seed, an **exact count and
  exact positions**; across N stated seeds, a **rate window**. Needs the dip
  detector from the testkit task. The implementing task states the
  render length and whether the multi-seed test runs in the default
  gate or is `[manual]`.
- **Per-pass reseeding (REQ-717)**: two consecutive passes on one
  cassette produce **different** dropout positions, modulation-noise
  sequences and scrape walks; and a re-run of the same pass index
  reproduces them.
- **Character parameterisation and manifest compatibility (REQ-716)**:
  a **pre-004 `manifest.json` fixture committed to the repo** MUST load
  unchanged after every new `TapeCharacter` field is added - the
  regression test for the hard-load-failure hazard. Plus: under `Full`
  with `clean()`, output is near-transparent to a stated margin, and
  the punch-crossfade, REQ-306 and click-detector suites pass
  unchanged - which is what asserts the hysteresis exception in 3.3.
- **Bit-exact-portable feedback paths (REQ-715)**: the three
  preconditions are static properties and are checked as such - a test
  that greps **the stateful `Full` stages** (the hysteresis solver, the
  modulation-noise envelope, the head-bump biquad, level-dependent HF
  loss, dropout tails; **not** the memoryless nonlinearity, which is
  permitted `powf` on the same standing as today's `tanh` - see 3.2b)
  for `mul_add` **and for the
  transcendental set (`sin`, `cos`, `tan`, `exp`, `ln`, `log`, `powf`,
  `tanh`, `sinh`, `cosh`)**, since the requirement forbids anything but
  add/sub/mul/div/sqrt and a `mul_add`-only grep would pass a `powf` on
  a feedback path; a compile-time assertion that the Langevin table is
  `const`; and a stated denormal policy in the module doc. A
  *documented* denormal policy is not an enforced one - what actually
  covers it is the `[manual]` cross-platform comparison below. The cross-platform equivalence check
  itself cannot run on Linux-only CI and is `[manual]`: a macOS vs
  Linux render comparison, recorded in `docs/manual-checklist.md`.
- **Generation loss (REQ-403)**: both models, three generations,
  **monotonic HF decay and monotonic noise-floor rise**. `Full` changes
  what each generation costs, so the compounding claim must be
  re-asserted rather than assumed to survive.
- **Both models bit-reproducible (REQ-702)**: two renders, same seed,
  identical bytes, for `Full` and `Simple`.
- **Block-size invariance per new stage**, via
  `porta_dsp::testing::assert_block_size_invariant` - **except
  crosstalk**, which that helper cannot express, since it drives a
  single mono `AudioProcessor` in isolation while crosstalk is a
  property of four tracks advancing together. Crosstalk extends
  `realtime_sim.rs::block_size_does_not_change_the_render`, which
  already renders a session at two block sizes and compares, and it
  MUST run on a character with crosstalk non-zero or it asserts
  nothing.
- **Allocator harness extended** (`tests/realtime_alloc.rs`) to a
  `Full` record pass and a `Full` bounce. REQ-902 is argued
  structurally again for a brand-new solver, and that harness exists
  precisely because structural arguments here have failed before.
- **Model plumbing (REQ-705)**, the enabling task's acceptance (see
  6.1): serde round-trip in all three cases (**absent -> `Simple`**,
  `"full"` -> `Full`, `"simple"` -> `Simple`), `--model full` reaching the manifest, `Op::New`'s field,
  and the **per-model resolved stage indices** matching what each
  model's chain actually builds. An unmoved golden is **not** an
  acceptance criterion on its own: it is equally consistent with
  correct dispatch and with a `model` field parsed and then ignored.
- **Goldens**: the existing golden is **pinned to `"model":"simple"`**
  and never re-blessed, so it stays a regression check on the
  preserved path. A **second, `Full` golden is blessed once**, at the
  end of the milestone, **with `"model":"full"` written explicitly into
  its script** - it must not rely on the creation default, which
  REQ-719 permits to be platform-dependent and which would otherwise
  render that golden as `Full` on the macOS where it was blessed and
  `Simple` on Linux CI.
- **Pi headroom, both models (REQ-711)**: `[manual]`, in
  `docs/manual-checklist.md` - wall-clock on hardware at four armed
  `Full` chains in one callback, for record and for bounce. Not
  something `cargo test` asserts.
- **Platform-dependent default (REQ-719)**: only engaged if REQ-711
  fails. Its test is that `porta-app` selects the model by `cfg` and
  the engine contains no such branch (REQ-901), asserted by the same
  grep that keeps hardware knowledge out of the engine.
- **Listening pass**: `[manual]`, in `docs/manual-checklist.md`.

## 6. Impact on tasks

### 6.1 The enabling task, scheduled first

Ahead of every lettered item, including (b), because everything after
it assumes it exists. It carries: the `TapeModel` type; the `Manifest`
field with `Simple` on absent (REQ-705); model-dependent `build_chain`
and `build_split_chain`, and the model-dependent **stage indices** that
`HISS_STAGE`/`FLUTTER_STAGE`/`SPLIT_HISS_STAGE` currently hardcode; the
`--model` CLI flag and the `Op::New` field; the existing golden pinned
to `"model":"simple"`; the **frozen `Simple` reference**, which must be
captured from the pre-004 build and so cannot be produced by any later
task; and the pre-004 `manifest.json` fixture.

`golden.rs` becomes two-golden aware here too: its module doc ("The one
golden render"), its `GOLDEN` const and its single `UPDATE_GOLDEN`
switch are all written for exactly one file.

It lands with `Full` still equal to `Simple`. Its acceptance criteria
are in section 5, not "the golden did not move".

### 6.2 Then the model itself

A new milestone, **M8**, roughly one task per lettered item, ordered:
headroom (b) first, being the largest audible defect and the cheapest
fix; then head bump (c) and modulation noise (d); then level-dependent
HF (e) and scrape flutter (f); then hysteresis (a), the big one; then
dropouts (g); crosstalk (h) last and separable.

**A porta-testkit task is scheduled before (f) and (g)** for the two
measurement tools that do not exist: sideband energy for scrape
flutter, and a dip detector for dropouts.

**The default flip is its own final task**, after REQ-711's
measurement. Until then `Full` is opt-in via `--model full`, which is
what makes every intermediate task safe to land. This ordering is what
keeps the `Full` default from arriving on an unmeasured Pi: M6.1 and
M6.2 are both still open and M4's hardware item is `[!]`, so measuring
first costs only sequencing.

If `Full` does not fit on the Pi, the honest consequence is **not**
just "Simple becomes the Pi's default": REQ-719 makes the creation
default platform-dependent, so the same command produces
differently-sounding cassettes on different machines, and it leaves a
`Full` cassette **unrecordable on the Pi** - playable, mixable,
bounceable and exportable there, but not overdubbable. That is a real
limitation. The alternative shape - raise the period on the Pi and keep
`Full`, as M6.2 already contemplates for the bus - should be weighed
against it at measurement time.

### 6.3 Elsewhere

- **M6.2** gains the two-model measurement (REQ-711), at the worst case
  of four `Full` chains in one callback, not the bounce's two.
- **Existing tests calibrated to the current transfer curve need
  re-tuning**, named now rather than discovered one failure at a time:
  `character.rs`'s `default_character_colours_the_signal`,
  `default_character_kills_the_top_end` and `hiss_reaches_the_output`;
  the `saturation.rs` suite; `generation_loss.rs`'s thresholds; and
  `bounce_acceptance.rs::hot_generations_engage_the_quantize_clamp`,
  whose stated rationale becomes obsolete once the ceiling is removed.
  Two **doc comments** go stale with them and are easy to miss:
  `saturation.rs`'s module doc explaining makeup as "1/drive keeps
  quiet material at its original level regardless of drive", and
  `character.rs`'s `clean()` comment that "tanh at unity drive still
  bends a -12 dBFS signal by about 2 percent". Neither is true under
  `Full`.
- `docs/manual-checklist.md` gains the listening pass, REQ-711's Pi
  measurement and REQ-715's cross-platform comparison.
- `site/architecture.md` and its Spanish twin enumerate the chain
  stages and need updating, including REQ-701's corrected order.
- **Sequencing against change 003**: this is M8, after 003's work
  (approved and unqueued). It does not depend on 003, but running them
  concurrently would put two golden-affecting changes in flight.

## 7. Alternatives considered and rejected

- **Tune `drive_db` and the filter corners and stop there.** Cheapest
  option, and it would partially fix defect 1.1. Rejected as the answer
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
- **Keep `tanh` and raise the makeup gain to 1.0** (proposed by a
  fourth review as the minimal fix for item b). Rejected on
  measurement: it gives **+9.0 dB of gain**, not unity, missing
  REQ-713 by 18x its bound. See 3.2b - the tanh family cannot satisfy
  REQ-707, REQ-713 and REQ-714 simultaneously at all.
## 8. History

All review forensics live here. Sections 1-7 are normative.

**v1**: initial proposal. The three defects were
measured against the current chain before drafting (transfer curve,
frequency response and noise floor, via a temporary harness that was
removed afterwards), rather than asserted. Two owner decisions taken
before drafting: build the full model but keep the current one behind a
flag for constrained devices, and let the improved model become the
default with a single golden re-bless.

**v2**: a first review returned REVISE with fifteen
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

**v3**: a second review returned REVISE with nine
blocking findings; all are addressed.

The one worth naming first is **B1**, because it is this document's own
recurring failure rather than a new mistake: v2's history claims to
have fixed the missing operating reference on the THD target, and it
did - in the requirement, while leaving "THD at 0 dBFS between roughly
1% and 3%" standing in the Verification section, contradicting the very
requirement that section verifies. A correction landing in one place
and going stale in another is now the **fourth** instance in this
proposal (two wrong REQ-203 citations, a stale golden plan, this). The
verification bullet now carries a standing instruction to grep the
whole document after any numeric change.

The most substantive finding was **B2**, which the review reached by
arithmetic rather than by reading: for a smooth odd compressive curve
fundamental gain is about `1 - 3*THD`, so "3% THD at 0 VU" and "gain
within 0.5 dB of unity below 0 VU" only co-exist below about 1.9% THD.
The requirement was over-constrained by accident. It is now split into
REQ-707 (no ceiling), REQ-713 (gain, anchored at -30 dBFS and below)
and REQ-714 (THD at 0 VU), which keeps the musically-motivated 1-3%
window rather than narrowing it to fit.

Also addressed:

- **B4**: the `Simple` preservation check still had nothing to compare
  against - "bit-exact equality of an in-crate render" is not a claim
  until something says *equal to what*, and `Simple`'s code path is not
  literally untouched. A **frozen reference from the pre-004 build** is
  now committed, in the enabling task, before anything changes.
- **B7**: nothing scheduled the enabling layer - `TapeModel`, the
  manifest field, model-dependent chain building and stage indices, the
  `--model` flag, the golden pin, the frozen reference - while every
  later task assumed it. It is now one task, scheduled first, landing
  with `Full` still equal to `Simple`.
- **B8**: the blast radius of flipping the default was understated. 23
  `Engine::create()` and 32 `TapeCharacter::clean()` call sites become
  `Full` renders, and dropouts and crosstalk had no character parameter
  at all - so `clean()` could not have switched them off and "clean
  stays near-transparent" would have been false. Every `Full` stage is
  now required to be character-parameterised.
- **B6**: REQ-402 was listed untouched but enumerates the bus's stage
  set verbatim. Amended - and the open question it exposed is settled:
  bus dropouts are **shared** between L and R, since a physical defect
  does not strike two stripes at different moments.
- **B3** (stale golden bullet deleted), **B5** (the `Full` golden pins
  `"model":"full"` rather than trusting a platform-dependent default),
  **B9** (the bit-exactness constraint governs all of `Full`'s signal
  path, not only the solver, with the review's three preconditions:
  compile-time literal Langevin table, no `mul_add` on feedback paths,
  a stated denormal policy).

The review also confirmed the two positions v2 asked about - REQ-702's
bit-exactness is achievable under those preconditions, and the
two-golden plan is implementable as `golden.rs` and `Op::New` stand -
and answered the third: REQ-707's curve was not coherent, which is B2.
Non-blocking notes taken: REQ-711's measurement is `[manual]` and its
platform default belongs in porta-app, not the engine (REQ-901);
`model` lives on `Manifest` beside `character`, with the forward-compat
consequence stated; and `assert_block_size_invariant` cannot express
crosstalk, which gets a purpose-written engine-level test instead.
Ready for a third review.

**v4**: a third review returned REVISE with ten
blocking findings. All are addressed, and three of them were the same
failure this document cannot seem to stop committing.

**The pattern got worse before it got better.** v3's History claimed
B9 was fixed "with the review's three preconditions". It was not fixed
anywhere. The paragraph existed only in the History describing itself -
`mul_add` and `denormal` appeared nowhere else in the file. The edit
had silently failed to apply, and the check that was supposed to catch
that was a `grep -c` over three alternated patterns which counted two
*other* lines and returned a number that looked like success. So: the
sixth instance, and the first where the claim of a fix was the entire
fix. Two more turned up in the same sweep - REQ-203 still cited for
audio content at item (d) (instance seven, surviving the sweep that
fixed the other two), and the over-constrained "roughly unity below
0 VU" still standing in item (b) after REQ-713 had moved the anchor.

What changed as a result, beyond the text: every edit to this document
now aborts loudly on a non-matching anchor instead of silently doing
nothing, and each fix is verified by a pattern that can only match the
new text. Running the sweep the review asked for immediately caught an
inconsistency introduced **during this same revision** - bounding
REQ-713 downward to -60 dBFS made the verification bullet stale against
its own requirement within minutes. That is the whole argument for the
sweep, demonstrated on itself.

The most consequential finding was **F5**, and it was self-inflicted:
v3's own fix for B8 (parameterise every `Full` stage on
`TapeCharacter`) would have made **every existing cassette fail to
load**. `TapeCharacter`'s fields carry no per-field serde defaults, and
`Manifest`'s struct-level default fires only when the `character` key
is absent entirely - which it never is on a written manifest. Adding
fields would produce `missing field` on every pre-004 project: a hard
load failure, strictly worse than the silent-formulation-switch hazard
REQ-705 was written to prevent. REQ-716 now carries the per-field
defaults as a requirement rather than leaving them to an implementer to
notice.

Also addressed:

- **F8**: REQ-707 and REQ-708 interact badly and nothing bounded them.
  Remove the saturator's ceiling, then add several dB of LF gain
  *after* it, and bass-heavy material reaches the i16 clamp first -
  reinstating a hard limiter by the back door, the exact defect item
  (b) exists to remove. REQ-718 bounds post-chain peak and fixes head
  bump's position.
- **F9**: REQ-713/714 were unmeasurable as written. `thd_db` sums each
  partial over +/-2 bins (+/-2 Hz at 48 000 samples) while the default
  character's flutter smears the 7th harmonic by +/-48 Hz, and at the
  quiet end the -66 dBFS hiss bed dominates - the motivation table's
  flat -50/-49 dB is a noise floor, not distortion. Now asserted on a
  flutter-free, hiss-free character against the saturation stage.
- **F6**: the parameterisation rule was a normative MUST sitting in
  prose with no id (REQ-716 now), and it cannot cover hysteresis, which
  *replaces* the saturator rather than adding a stage. `clean()`'s
  transparency under `Full` rests on a J-A solver at -30 dB drive being
  as transparent as `tanh` at -30 dB - an assumption three test suites
  depend on, now required to be asserted directly.
- **F7**: nothing said the new stochastic stages must be reseeded per
  pass (REQ-717). Without it every pass gets identical dropout
  positions - audible, and it would distort REQ-403's compounding.
- **F10**: spec.md section 6 was not listed as affected, and M3's gate
  says "the single golden render passes" - the staleness reaching the
  constitution itself. M4's gate carries the REQ-203 conflation too.
- **F2, F3, F4** (wrong REQ id on the manifest rule: REQ-710 for
  REQ-705), and the non-blocking notes: the enabling task's real
  acceptance criteria (an unmoved golden proves nothing - it is equally
  consistent with a `model` field parsed and ignored); the frozen
  reference's named render, explicitly not `clean()`, which is blind to
  the two RNG streams REQ-706 protects; **when the default actually
  flips** (v3 gave three different answers - now its own final task,
  after REQ-711); a fixed channel convention for the shared bus dropout
  stream; REQ-402's amendment to be drafted rather than described;
  extending `block_size_does_not_change_the_render` rather than writing
  a new test; `golden.rs`'s one-golden plumbing; windows for REQ-708
  and REQ-712; REQ-711 split; and the corrected 30 `clean()` call sites.

The review independently redid the B2 arithmetic and confirmed both the
`1 - 3*THD` relation and that -30 dBFS is the right anchor - about 10x
inside the 0.5 dB bound, where -24 dBFS would have left only ~0.3 dB of
margin. Ready for a fourth review.

**v5**: a fourth review returned REVISE with eight
blocking findings. All are addressed, and the document has been
**restructured**, which is the more important change.

**The structure was the problem.** By v4 roughly half the document was
self-audit: ~20 inline forensic passages on top of a 190-line History.
The review's judgment - that this had stopped being neutral - is
correct, and finding G5 proved it. G5 is a normative line reading "the
amendment MUST be drafted, not described (it still is not)": a
requirement and a note about that requirement, in one sentence, in a
tense a reader cannot resolve. The anti-staleness annotation had become
a source of staleness. Sections 1-7 are now normative only; every piece
of review forensics lives here in section 8. Requirements are in strict
id order so the section can be audited by reading down, and every
requirement in 4.2 now has exactly one matching bullet in section 5.

**The review's suggested fix for item (b) was wrong, and checking it
found something better.** N8 offered "keep `drive_db: 9.0`, set
`makeup = 1.0`" as the minimal change satisfying REQ-707/713/714,
with numbers. Measured, it gives **+9.0 dB of gain**, not unity -
missing REQ-713 by 18x its bound. The review had computed `tanh(u)/u`,
the curve's deviation from linearity, and reported it as gain,
normalizing the drive out of its own measurement.

Checking it exposed a real constraint neither side had stated: for
`tanh(d*x)*m`, small-signal gain is `d*m` and the asymptote is `m`, so
REQ-713 forces `d*m ~ 1`, REQ-707 forces `m ~ 1`, and therefore
`d ~ 1` - plain `tanh(x)`, at **0.132% THD**, 8x below REQ-714. **The
tanh family cannot satisfy the three requirements together**, so item
(b)'s original "decouple makeup from `1/drive`" was not a sufficient
instruction. `f(x) = x/(1+|x|)` does satisfy all three - 2.01% THD at
0 VU, -0.230 dB at -30 dBFS, asymptote 1.0 - and satisfies REQ-715 for
free, being pure add/abs/divide with no transcendental. It is recorded
in 3.2b as an existence proof, not a mandate.

The blocking findings:

- **G1/G2**: REQ-707/713/714 were written against "the record path"
  while their verification measured the saturation stage in isolation -
  two different objects. On the actual record path REQ-713 is
  contradicted outright by REQ-708, which mandates +2 to +4 dB at
  60-80 Hz. The measurement point and test frequency are now **inside
  the requirements**, and REQ-713's downward bound is rejustified,
  since an isolated f32 stage has neither the hiss bed nor the i16 LSB
  its old rationale appealed to.
- **G3**: REQ-718 had no verification bullet and could not have had one
  at that measurement point - it is precisely a whole-chain property.
  It now has a record-path bullet.
- **G4**: REQ-712 had no bullet at all, and half its window was
  unmeasurable: -60 dB relative to a 0 VU source is -78 dBFS, 12 dB
  below the default hiss bed - the same defect this document had
  already caught for scrape flutter. Now measured hiss-free.
- **G5**: REQ-402's amendment is now **drafted**, placing modulation
  noise and scrape flutter against `build_split_chain`'s actual halves
  (`pre` is `[Saturation, Hiss, Bandwidth]`, `post` is `[Crush?]`), and
  choosing the shared bus dropout stream's channel term - 0, the
  convention `build_stereo_flutter` already documents.
- **G6**: the M6.2 bullet still argued an open question that v4 had
  already settled ("the flip is its own final task").
- **G7**: REQ-708's 2-5 dB window had landed in the requirement but not
  its verification bullet. Now 2-4 dB in both, reconciled with section
  1.2's measured +2 to +4 dB (N1).
- **G8**: the per-field serde defaults - the most consequential finding
  of the previous round - had no regression test. A pre-004
  `manifest.json` fixture is now committed and MUST load unchanged.

Non-blocking notes taken: REQ-709 and REQ-710 given real windows (N3);
REQ-715's static preconditions given a verification mechanism, with the
cross-platform check marked `[manual]` since Linux-only CI cannot run
it (N4); REQ-717 given a bullet (N5); requirements reordered
monotonically (N6); "REQ-705 (addition)" promoted to **REQ-719** (N7);
head bump's raw-vs-net gain distinction stated (N2); the two answers
for an absent field noted (N10); and the undo journal's non-involvement
stated (N11).

Ready for a fifth review.

**v6**: a fifth review verified the whole saturation
analysis independently - every figure in 3.2b reproduced to three
decimals, and the tanh-family impossibility argument confirmed and
tightened (REQ-707 forces `m >= 1`, REQ-713 caps `d <= 1.059`, so THD
at 0 VU cannot exceed 0.148%, 6.8x under the window). That section
needs no further work. Eight blocking findings elsewhere, all
addressed.

**The restructure cost content, which is the risk a rewrite carries and
this one realised.** Constraints that existed *only* inside a forensic
passage were deleted along with the passage. Worst: **hysteresis - the
largest item in the change, the one 3.2a calls "what separates tape
from a waveshaper" - ended up with no requirement and no test.** Its
discriminator (minor-loop width on a slow triangle, growing with
amplitude, which is what a biquad cannot fake) had been written into a
v2 finding and lived in a bullet the rewrite dropped; the only surviving
mention asserted hysteresis is *transparent* under `clean()`. It is now
REQ-720. Scrape flutter and dropouts were verified but unrequired -
REQ-721 and REQ-722. Also restored: the no-reordering constraint, TPDF
dither in REQ-701's drafted text, crosstalk's source-signal ambiguity
and its porta-dsp placement argument, REQ-703/704's accounting (they
live in spec.md 4.7, not 5, so the old formula covered neither),
REQ-804's optional `model` field, REQ-403's generation-loss bullet, and
item (a)'s per-sample transcendental count.

**Two normative claims were false, both checked and both confirmed
false.**

- **H3**: "the current chain fails all three today" - it does not. At
  REQ-713/714's own new measurement point the *current* saturator
  passes both: -0.017 dB at -30 dBFS, and **1.017% THD at 0 VU**, which
  is inside the 1-3% window. Only REQ-707 fails, and it fails hard
  (-9.06 dBFS ceiling at any input). The sentence was true when the
  measurement point was the whole record path and survived G1's move.
  The consequence is not cosmetic: **REQ-713 and REQ-714 cannot be
  acceptance criteria for item (b)** because they are green on the code
  being replaced. REQ-707 is now the discriminator and is stated
  concretely (peak at or above -7 dBFS for a 0 dBFS sine: current
  -9.06 fails, `x/(1+|x|)` -6.02 passes); 713 and 714 are labelled
  guard rails.
- **H4**: "print levels rise by roughly the 7 dB of gain reduction
  being removed" - measured against the document's own existence proof,
  the rise is **+2.18 dB at 0 dBFS, +0.09 dB at -6 dBFS, and -0.61 dB
  at 0 VU**. 7 dB is what the old curve costs at 0 dBFS, not the
  difference between curves.

Also: **H5** - REQ-718 never said whether "0 VU" meant peak or RMS, and
the crest factor between them (10-18 dB) is the entire margin the
requirement exists to protect; read as a peak it had ~15 dB of headroom
and would never have bitten. Re-anchored at 0 dBFS peak with a
bass-heavy programme. **H6** - G1/G2 established that the measurement
point is part of the requirement, then applied it only to 713/714;
REQ-709 (gap offset and window, since the reading otherwise depends
entirely on an unspecified envelope release), REQ-710 (band edges and
source) and REQ-712 (band and duration) now carry theirs. **H7** - the
"hiss-free character" three requirements measure on cannot be
`clean()`, because REQ-716 makes `clean()` zero the very stage being
measured; section 4.3 now names two test fixtures.

Non-blocking taken: section 1.1's THD column was itself measured
through flutter and is 9-10 dB low by this document's own F9 reasoning
(corrected - 0 dBFS is **24%**, not 8%, which strengthens the case);
the jointly achievable THD range is about 1-2.9%, so REQ-714's upper
edge is unreachable and `x/(1+|x|)` spends 46% of REQ-713's budget
rather than sitting 10x inside it; `Manifest.character`'s serde default
is field-level, not struct-level; REQ-715's grep extended to the whole
transcendental set, since a `mul_add`-only grep would pass a `powf`;
REQ-711 and REQ-719 given their own bullets.

Ready for a sixth review.

**v7**: a sixth review re-derived every normative number
in the document - section 1.1's table, 3.2b's curve figures, H3's and
H4's replacements - and reproduced all of them to the digit. Three
blocking findings, two of which were real design gaps rather than text
defects.

- **B1**: the document misquoted **REQ-701's actual text**, the very
  text this change amends. The real order is saturation → bandwidth →
  **wow/flutter → hiss** → crush → dither; v6 transposed the last two,
  which made hiss look one position out of place when it is **two**.
  Corrected, and the amendment now also records that **spec.md is
  internally inconsistent today**: REQ-402 already matches the code
  while REQ-701 does not.
- **B2**: **nothing said what `drive_db` does under `Full`** - and the
  document's own algebra ruled out every unstated reading. Drive as a
  multiplier with `makeup = 1/drive` reproduces the -9 dBFS ceiling the
  change exists to remove; with `makeup = 1` it gives +9 dB of gain
  (the error 3.2b already rejects for tanh - and that argument applies
  to any drive-scaled family, which the document had not noticed);
  `x/(1+k|x|)` puts the asymptote at `1/k`, -9 dBFS again at the
  default; ignoring drive breaks `clean()`, which REQ-716 rests on.
  Resolved by mapping `drive_db` to the **knee exponent**,
  `n = 10^((9 - drive_db)/20)`. Measured: the default 9.0 dB gives
  `n = 1.0`, exactly the existence proof, and `clean()`'s -30 dB gives
  `n = 89` - **+0.0000 dB gain, 0.0000% THD**, a literal wire, which is
  what keeps the punch-crossfade, REQ-306 and click-detector suites
  passing. Also stated: the windows are for the **default** character,
  since `drive_db: 12` legitimately falls outside REQ-713.
- **B3**: REQ-707/713/714 measure "the saturation stage", which
  finished `Full` **does not have** - item (a) replaces it with the
  solver. So the requirements had no referent in the state they
  describe, and worse, **whatever curve task (b) picks is discarded by
  task (a)** with nothing requiring the solver to preserve it: a J-A
  solver that reintroduced a -9 dBFS ceiling would have passed REQ-720
  cleanly. REQ-707 now names the nonlinear slot rather than a
  particular occupant, and REQ-720 requires the solver to satisfy all
  three - so the transfer-curve tests run **twice**, at (b) and at (a).

Notes taken: **N1** - v6's own corrected table left two passages
describing the *pre*-correction one (the "flat -50/-49 dB" back-
reference no longer existed anywhere), and "gain" is now defined once
as fundamental-band rather than broadband RMS, which differ by up to
0.25 dB at the hot end; **N2** - strict id order broke again when v6's
restorations and REQ-720-722 landed, now monotonic in both groups;
**N3** - the new measurement windows had landed in the requirements but
not in the matching verification bullets, which is the document's own
stated rule; **N5** - the three newest requirements defer their
thresholds to their tasks, now with an explicit commitment to fold the
measured margin back into the requirement; **N6b** - REQ-715's failure
path (amend REQ-702 and the golden tolerance openly, never absorb it
silently) had been dropped in the v5 rewrite and is restored.

The review's own bottom line: the analysis is sound and independently
reproducible, and what stood between this and implementation was one
misquote and two unanswered questions. Those are answered. Sent for a
seventh review scoped to the new B2/B3 material only, since the
`drive_db` mapping is a design decision no reviewer has seen.

**v8 (this revision)**: a seventh review, scoped to v7's new material,
reproduced the whole `drive_db` mapping to the digit and confirmed B3
creates no impossible requirement - it checked specifically, since
rounds 2 and 3 both found requirement sets that were unsatisfiable, and
found the Rayleigh law (`loop width ~ A^2`) leaves REQ-713's budget
intact across the 12 dB from 0 VU to -30 dBFS. Three blocking findings,
all narrow.

**R1 was my error, twice in one paragraph.** v7 claimed the curve is
"pure add/abs/divide - no transcendental at all, so REQ-715 holds by
construction". That is true **only at `n = 1`**, the default operating
point; every other character needs `powf`, including `clean()` at
`n = 89`, which 30 call sites route through once the default flips. So
the claim was false about the stage while being true about one point on
it. And in the same breath v7 rejected the `n = 1.2` neighbour "because
`powf` is forbidden on a feedback path" - when **a memoryless
waveshaper is not a feedback path**, so that rejection was never valid
and was constraining task (b)'s curve choice on a false premise.

The ruling: **REQ-715 is scoped to stateful and feedback stages**,
which is what its own text always said. `powf` in a memoryless
nonlinearity is permitted on exactly the standing today's `tanh` has -
the chain has shipped a libm `tanh` since M1 without troubling REQ-702,
because per-sample rounding differences stay bounded instead of
accumulating, and sit well inside `golden.rs`'s 3 LSB tolerance. Once
item (a) replaces that stage with a solver the slot **becomes** a
feedback path and REQ-715 applies in full, which is what REQ-720's
inheritance clause already handles. Section 5's grep is rescoped to
name the stateful stages explicitly.

**R2 was B2's own finding, one item over - and on the item that
ships.** The knee-exponent mapping answers what `drive_db` does to a
curve family that has a knee exponent. A J-A solver has none, and
(b)'s curve is explicitly discarded when (a) lands, so after item (a)
nothing said how `drive_db: -30.0` reaches the transparency REQ-716,
the punch-crossfade suite, REQ-306 and the click detector all rest on.
REQ-720 now requires the task to name the solver parameter `drive_db`
scales and to assert transparency at `clean()`. It also gets a
boundedness clause: `output_is_bounded_and_finite_under_abuse` is a
`Saturation` unit test and `Saturation` survives only on the `Simple`
path, so REQ-707 was giving the solver a floor with nothing giving it
a ceiling.

**R3**: REQ-713 and REQ-714 specify frequency, isolation, flutter and
hiss but said nothing about **settling** - inert for a memoryless
curve, decisive for a solver, whose reading from a demagnetized state
differs from the steady-state minor loop. Both now discard a stated
settling interval, as does the verification bullet.

Non-blocking taken: the **usable range** for the mapping is
`drive_db <= 15`, stated now rather than discovered - above it the knob
inverts into an attenuator (`drive_db 24` attenuates -40 dBFS by
17.3 dB, where today's `makeup = 1/drive` holds it at exactly 0 dB for
any drive); two rejected alternatives were the same construction
written twice; and two **doc comments** go stale under `Full`
(`saturation.rs`'s makeup rationale and `character.rs`'s `clean()`
comment), now listed in 6.3 alongside the tests.

Sent to the owner for sign-off. Seven review rounds is past the point
where more review is the cheaper way to find defects: what remains is
better discharged by implementing task (b) and measuring.
