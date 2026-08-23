//! Playback mixer: per-track fader and equal-power pan into a stereo
//! master. Parameter changes ramp to their new value over a fixed 5ms,
//! so jumps never click (REQ-602). Playback-side only; nothing here
//! touches tape.
//!
//! The ramp is a fixed number of samples rather than "one block": a
//! per-block ramp would make a fader move sound different on a 64-frame
//! device than on a 512-frame one, which breaks block-size invariance
//! (REQ-203).

use crate::NUM_TRACKS;
use porta_dsp::SAMPLE_RATE;

/// Parameter ramp length: 5ms.
const SMOOTH_SAMPLES: u32 = SAMPLE_RATE / 200;

/// Lowest level a meter reports; silence clamps here instead of
/// negative infinity so a UI meter has something finite to draw.
const FLOOR_DBFS: f32 = -160.0;

fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn amp_to_db(amp: f32) -> f32 {
    if amp <= 0.0 {
        FLOOR_DBFS
    } else {
        (20.0 * amp.log10()).max(FLOOR_DBFS)
    }
}

/// Equal-power pan law. `pan` in [-1, 1], 0 = center (-3 dB per side).
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// A gain that walks to its target over a fixed time, one sample at a
/// time, so its behaviour does not depend on how the caller blocks up
/// the audio.
#[derive(Clone, Copy)]
struct Smoothed {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl Smoothed {
    fn settled(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
        }
    }

    fn set_target(&mut self, target: f32) {
        if target != self.target {
            self.target = target;
            self.step = (target - self.current) / SMOOTH_SAMPLES as f32;
            self.remaining = SMOOTH_SAMPLES;
        }
    }

    fn tick(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
            } else {
                self.current += self.step;
            }
        }
        self.current
    }
}

pub struct Mixer {
    fader_db: [f32; NUM_TRACKS],
    pan: [f32; NUM_TRACKS],
    master_db: f32,
    /// Silences a track's contribution independent of its fader value -
    /// unlike pulling the fader down, muting doesn't disturb the gain
    /// you'll get back when you unmute.
    muted: [bool; NUM_TRACKS],
    /// Keeps a track out of the *monitor* sum while leaving it at full
    /// weight in the print sum and leaving its meter live (REQ-408).
    /// Set only while a bounce pass is open: the tracks are already
    /// inside the bus's printed copy, so summing them again would be an
    /// audible double. Deliberately NOT folded into `target` the way
    /// mute is - that would take them out of the print too, which is
    /// the opposite of what a bounce needs.
    excluded_from_sum: [bool; NUM_TRACKS],
    left: [Smoothed; NUM_TRACKS],
    right: [Smoothed; NUM_TRACKS],
    /// The bounce bus's own fader/mute (REQ-409): stereo already, so no
    /// pan - one gain for both channels.
    bus_fader_db: f32,
    bus_muted: bool,
    bus_gain: Smoothed,
    /// The master fader, ramped on its own instead of folded into each
    /// track's gain (change 001). Same click-free guarantee as before,
    /// applied once at the end rather than baked into every track -
    /// which is what keeps it out of anything printed to tape
    /// (REQ-406).
    master: Smoothed,
    /// Peak of each track's contribution (post-fader, pre-pan) in the
    /// most recent block, and of the summed master output. A meter
    /// reading, not audio - fine to read from a UI timer, updated for
    /// free alongside the mix itself.
    track_peak: [f32; NUM_TRACKS],
    master_peak: (f32, f32),
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    pub fn new() -> Self {
        // Unity fader, centre pan: start settled so the first block does
        // not fade in from silence.
        let (l, r) = pan_gains(0.0);
        Self {
            fader_db: [0.0; NUM_TRACKS],
            pan: [0.0; NUM_TRACKS],
            master_db: 0.0,
            muted: [false; NUM_TRACKS],
            excluded_from_sum: [false; NUM_TRACKS],
            left: [Smoothed::settled(l); NUM_TRACKS],
            right: [Smoothed::settled(r); NUM_TRACKS],
            bus_fader_db: 0.0,
            bus_muted: false,
            bus_gain: Smoothed::settled(1.0),
            master: Smoothed::settled(1.0),
            track_peak: [0.0; NUM_TRACKS],
            master_peak: (0.0, 0.0),
        }
    }

    pub fn set_fader_db(&mut self, track: usize, db: f32) {
        self.fader_db[track] = db;
    }

    pub fn set_muted(&mut self, track: usize, muted: bool) {
        self.muted[track] = muted;
    }

    pub fn is_muted(&self, track: usize) -> bool {
        self.muted[track]
    }

    pub fn set_pan(&mut self, track: usize, pan: f32) {
        self.pan[track] = pan.clamp(-1.0, 1.0);
    }

    pub fn set_master_db(&mut self, db: f32) {
        self.master_db = db;
    }

    /// Exclude a track from the audible/monitor sum while leaving it in
    /// the print sum and on its meter (REQ-408). Only an open bounce
    /// pass sets this.
    pub fn set_excluded_from_sum(&mut self, track: usize, excluded: bool) {
        self.excluded_from_sum[track] = excluded;
    }

    pub fn set_bus_fader_db(&mut self, db: f32) {
        self.bus_fader_db = db;
    }

    pub fn bus_fader_db(&self) -> f32 {
        self.bus_fader_db
    }

    pub fn set_bus_muted(&mut self, muted: bool) {
        self.bus_muted = muted;
    }

    pub fn is_bus_muted(&self) -> bool {
        self.bus_muted
    }

    pub fn fader_db(&self, track: usize) -> f32 {
        self.fader_db[track]
    }

    pub fn pan(&self, track: usize) -> f32 {
        self.pan[track]
    }

    pub fn master_db(&self) -> f32 {
        self.master_db
    }

    /// Post-fader peak of `track` from the most recently mixed block,
    /// in dBFS. Meant for a UI meter, not audio-critical - one block of
    /// latency is fine.
    pub fn track_level_db(&self, track: usize) -> f32 {
        amp_to_db(self.track_peak[track])
    }

    /// Peak of the summed stereo output from the most recently mixed
    /// block, in dBFS.
    pub fn master_level_db(&self) -> (f32, f32) {
        (amp_to_db(self.master_peak.0), amp_to_db(self.master_peak.1))
    }

    /// A track's pre-master stereo gain. The master is deliberately NOT
    /// folded in here any more (change 001, REQ-406): it rides its own
    /// ramp in `finish_mix`, so nothing the master does can reach a
    /// signal on its way to tape.
    fn target(&self, track: usize) -> (f32, f32) {
        if self.muted[track] {
            // Folded into the same smoothed target as the fader, so
            // mute/unmute rides the existing 5ms ramp instead of an
            // instant cut - no separate click-avoidance path needed.
            return (0.0, 0.0);
        }
        let amp = db_to_amp(self.fader_db[track]);
        let (l, r) = pan_gains(self.pan[track]);
        (amp * l, amp * r)
    }

    fn bus_target(&self) -> f32 {
        if self.bus_muted {
            0.0
        } else {
            db_to_amp(self.bus_fader_db)
        }
    }

    /// Phase 1 (change 001): sum the four tracks, pre-master, ticking
    /// each track's ramps **exactly once per sample** whatever else is
    /// going on.
    ///
    /// Writes two sums from the same per-sample scaled values:
    /// `mon_*` is the audible/monitor sum, which skips tracks flagged
    /// `set_excluded_from_sum`. `print`, when given, is the ungated
    /// sum: every track at full weight regardless of that flag, because
    /// exclusion is about audibility, not about what gets printed
    /// (REQ-406 vs REQ-408 need opposite things from the same tracks
    /// during a bounce, which is why there are two sums and not one).
    ///
    /// Both sums are overwritten, not accumulated into.
    pub fn sum_tracks(
        &mut self,
        inputs: &[&[f32]; NUM_TRACKS],
        mon_l: &mut [f32],
        mon_r: &mut [f32],
        mut print: Option<(&mut [f32], &mut [f32])>,
    ) {
        let len = mon_l.len();
        assert_eq!(mon_r.len(), len);
        mon_l.fill(0.0);
        mon_r.fill(0.0);
        if let Some((pl, pr)) = print.as_mut() {
            assert_eq!(pl.len(), len);
            assert_eq!(pr.len(), len);
            pl.fill(0.0);
            pr.fill(0.0);
        }
        if len == 0 {
            return;
        }
        for (t, input) in inputs.iter().enumerate() {
            assert_eq!(input.len(), len);
            let (tl, tr) = self.target(t);
            self.left[t].set_target(tl);
            self.right[t].set_target(tr);
            // Fader only, not master: lets the meters show track
            // balance independent of the overall volume knob. Muted
            // reads as silent too - the meter should match what's
            // actually contributing to the mix, not the fader value
            // muting is deliberately not disturbing. Exclusion does
            // NOT silence the meter (REQ-408): the whole point is
            // riding these faders while the bounce runs.
            let fader_amp = if self.muted[t] {
                0.0
            } else {
                db_to_amp(self.fader_db[t])
            };
            let excluded = self.excluded_from_sum[t];
            let mut peak = 0.0f32;
            for (n, &s) in input.iter().enumerate() {
                // Ticked unconditionally, before any gating: skipping
                // these for an excluded track would freeze its ramp for
                // the whole pass and snap it at punch-out - a real
                // click (REQ-602) whose duration would depend on block
                // size (REQ-203).
                let gl = self.left[t].tick();
                let gr = self.right[t].tick();
                let (sl, sr) = (s * gl, s * gr);
                if !excluded {
                    mon_l[n] += sl;
                    mon_r[n] += sr;
                }
                if let Some((pl, pr)) = print.as_mut() {
                    pl[n] += sl;
                    pr[n] += sr;
                }
                peak = peak.max(s.abs());
            }
            self.track_peak[t] = peak * fader_amp;
        }
    }

    /// Phase 2 (change 001): fold the bounce bus into the monitor sum
    /// at its own smoothed fader/mute (no pan - it is already stereo),
    /// apply the master ramp, and clamp. `out_*` arrive holding phase
    /// 1's monitor sum and leave holding the finished output - the only
    /// place `out_l`/`out_r` are written.
    ///
    /// `bus_gain`, when given, is this block's already-ticked gain from
    /// `tick_bus_gain` - a bounce pass needs the same per-sample values
    /// before the chain and after it, and the chain runs between the
    /// two phases, so it ticks once up front and hands the values here.
    /// When `None`, this ticks the ramp itself, once per sample,
    /// including when `bus` is `None`: a frozen ramp would break
    /// block-size invariance just as surely as a double-ticked one.
    pub fn finish_mix(
        &mut self,
        out_l: &mut [f32],
        out_r: &mut [f32],
        bus: Option<(&[f32], &[f32])>,
        bus_gain: Option<&[f32]>,
    ) {
        let len = out_l.len();
        assert_eq!(out_r.len(), len);
        if len == 0 {
            return;
        }
        if let Some((bl, br)) = bus {
            assert_eq!(bl.len(), len);
            assert_eq!(br.len(), len);
        }
        if bus_gain.is_none() {
            self.bus_gain.set_target(self.bus_target());
        }
        self.master.set_target(db_to_amp(self.master_db));
        for n in 0..len {
            // Either the caller already ticked this block's gain (a
            // bounce needs the same values pre-chain, and ticking here
            // too would advance the ramp twice per sample) or we tick
            // it ourselves - exactly once, either way.
            let bg = match bus_gain {
                Some(g) => g[n],
                None => self.bus_gain.tick(),
            };
            if let Some((bl, br)) = bus {
                out_l[n] += bl[n] * bg;
                out_r[n] += br[n] * bg;
            }
            let m = self.master.tick();
            // Hard safety ceiling on the summed output, not just the
            // offline WAV writer's own clamp (render.rs quantizes with
            // one too - this makes the live monitoring path match it
            // instead of being the one place nothing stops an extreme
            // value from reaching a real speaker or headphones). Four
            // tracks at up to +12dB of fader gain each plus the bus and
            // the master can genuinely sum past 0dBFS; this is the last
            // line of defense before that reaches hardware, not a
            // substitute for sane gain staging.
            out_l[n] = (out_l[n] * m).clamp(-1.0, 1.0);
            out_r[n] = (out_r[n] * m).clamp(-1.0, 1.0);
        }
        self.master_peak = (
            out_l.iter().fold(0.0f32, |acc, &s| acc.max(s.abs())),
            out_r.iter().fold(0.0f32, |acc, &s| acc.max(s.abs())),
        );
    }

    /// Mix one block of the four track signals into stereo out. All
    /// slices must share the same length. The ordinary playback path:
    /// both phases back to back with nothing between them, which is
    /// exactly what this did before it was split.
    pub fn mix_block(
        &mut self,
        inputs: &[&[f32]; NUM_TRACKS],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        self.sum_tracks(inputs, out_l, out_r, None);
        self.finish_mix(out_l, out_r, None, None);
    }

    /// Fill `out` with this block's bus gain, one ticked value per
    /// sample. For the bounce path, which needs the same values either
    /// side of the character chain; ordinary playback lets `finish_mix`
    /// tick for itself instead.
    pub fn tick_bus_gain(&mut self, out: &mut [f32]) {
        self.bus_gain.set_target(self.bus_target());
        for slot in out.iter_mut() {
            *slot = self.bus_gain.tick();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use porta_testkit::meter::rms_dbfs;
    use porta_testkit::signal::{silence, sine};
    use porta_testkit::{assert_no_clicks, assert_rms_near_db};

    fn mix_once(mixer: &mut Mixer, track: usize, signal: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let quiet = silence(signal.len());
        let mut inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
        inputs[track] = signal;
        let mut l = vec![0.0; signal.len()];
        let mut r = vec![0.0; signal.len()];
        mixer.mix_block(&inputs, &mut l, &mut r);
        (l, r)
    }

    #[test]
    fn center_pan_is_equal_power() {
        let mut m = Mixer::new();
        let s = sine(1000.0, 0.0, 48_000);
        let (l, r) = mix_once(&mut m, 0, &s);
        // 0 dBFS-peak sine has -3.01 dB RMS; center pan adds -3.01 per side.
        assert_rms_near_db!(&l, -6.02, 0.1);
        assert_rms_near_db!(&r, -6.02, 0.1);
    }

    #[test]
    fn hard_pan_silences_the_other_side() {
        let mut m = Mixer::new();
        m.set_pan(2, -1.0);
        let s = sine(1000.0, 0.0, 4800);
        // Let the 5ms ramp settle, then measure a clean block.
        mix_once(&mut m, 2, &s);
        let (l, r) = mix_once(&mut m, 2, &s);
        assert_rms_near_db!(&l, -3.01, 0.1);
        assert!(
            rms_dbfs(&r) < -80.0,
            "right should be silent, got {}",
            rms_dbfs(&r)
        );
    }

    #[test]
    fn fader_and_master_scale() {
        let mut m = Mixer::new();
        m.set_fader_db(1, -6.0);
        m.set_master_db(-4.0);
        let s = sine(1000.0, 0.0, 48_000);
        mix_once(&mut m, 1, &s); // settle ramps
        let (l, _) = mix_once(&mut m, 1, &s);
        // -3.01 (sine RMS) - 3.01 (center pan) - 6 (fader) - 4 (master)
        assert_rms_near_db!(&l, -16.02, 0.1);
    }

    #[test]
    fn fader_jump_does_not_click() {
        let mut m = Mixer::new();
        let s = sine(440.0, -6.0, 4800);
        let mut left = Vec::new();
        for block in 0..10 {
            if block == 5 {
                m.set_fader_db(0, -30.0); // hard jump mid-stream
            }
            let (l, _) = mix_once(&mut m, 0, &s);
            left.extend(l);
        }
        assert_no_clicks!(&left);
        // And the jump actually happened.
        assert!(rms_dbfs(&left[..4800]) - rms_dbfs(&left[43_200..]) > 20.0);
    }

    #[test]
    fn master_jump_does_not_click() {
        // The master used to ride each track's own ramp (it was folded
        // into `target`); change 001 gave it its own. That refactor
        // would have been free to drop the smoothing entirely and
        // nothing here would have caught it - this is the guard.
        let mut m = Mixer::new();
        let s = sine(440.0, -6.0, 4800);
        let mut left = Vec::new();
        for block in 0..10 {
            if block == 5 {
                m.set_master_db(-30.0); // hard jump mid-stream
            }
            let (l, _) = mix_once(&mut m, 0, &s);
            left.extend(l);
        }
        assert_no_clicks!(&left);
        assert!(rms_dbfs(&left[..4800]) - rms_dbfs(&left[43_200..]) > 20.0);
    }

    #[test]
    fn excluded_track_leaves_the_monitor_sum_but_not_the_print_or_its_meter() {
        // REQ-408's three-way split, the reason there are two sums.
        let mut m = Mixer::new();
        let s = sine(440.0, -6.0, 512);
        let quiet = silence(512);
        let inputs: [&[f32]; NUM_TRACKS] = [&s, &quiet, &quiet, &quiet];

        let (mut ml, mut mr) = (vec![0.0; 512], vec![0.0; 512]);
        let (mut pl, mut pr) = (vec![0.0; 512], vec![0.0; 512]);

        m.set_excluded_from_sum(0, true);
        m.sum_tracks(&inputs, &mut ml, &mut mr, Some((&mut pl, &mut pr)));

        assert!(
            ml.iter().all(|&x| x == 0.0),
            "an excluded track must be absent from the monitor sum"
        );
        assert!(
            pl.iter().any(|&x| x != 0.0),
            "an excluded track must still be at full weight in the print sum"
        );
        assert!(
            m.track_level_db(0) > -20.0,
            "an excluded track's meter must stay live, got {}",
            m.track_level_db(0)
        );

        // Un-excluded, the monitor sum matches the print sum exactly.
        m.set_excluded_from_sum(0, false);
        m.sum_tracks(&inputs, &mut ml, &mut mr, Some((&mut pl, &mut pr)));
        assert_eq!(ml, pl, "with nothing excluded the two sums must agree");
    }

    #[test]
    fn excluding_a_track_does_not_freeze_its_ramp() {
        // The ramps must keep ticking while excluded, or they'd snap to
        // their live value at punch-out - a click whose length depends
        // on block size (REQ-602/203). Move a fader while excluded,
        // then un-exclude: the gain must already be there, not ramping.
        let mut m = Mixer::new();
        let s = sine(440.0, -6.0, 4800);
        let quiet = silence(4800);
        let inputs: [&[f32]; NUM_TRACKS] = [&s, &quiet, &quiet, &quiet];
        let (mut l, mut r) = (vec![0.0; 4800], vec![0.0; 4800]);

        m.set_excluded_from_sum(0, true);
        m.set_fader_db(0, -20.0);
        // Long enough for the 5ms ramp to settle several times over.
        m.sum_tracks(&inputs, &mut l, &mut r, None);
        m.set_excluded_from_sum(0, false);
        m.sum_tracks(&inputs, &mut l, &mut r, None);

        // Already at -20dB from the first sample: no audible ramp-in.
        let head = rms_dbfs(&l[..240]);
        let tail = rms_dbfs(&l[4560..]);
        assert!(
            (head - tail).abs() < 1.0,
            "gain was still ramping after un-exclude: head {head:.1} vs tail {tail:.1}"
        );
    }

    #[test]
    fn split_phases_are_block_size_invariant() {
        // REQ-203 across the new two-phase path, at the awkward sizes
        // the task calls for (1 and 37 are deliberately not divisors).
        let render = |block: usize| {
            let mut m = Mixer::new();
            m.set_fader_db(1, -4.0);
            m.set_pan(1, -0.3);
            m.set_master_db(-2.0);
            let s = sine(440.0, -6.0, 2048);
            let quiet = silence(2048);
            let mut out = Vec::new();
            let mut done = 0;
            while done < 2048 {
                let n = block.min(2048 - done);
                let inputs: [&[f32]; NUM_TRACKS] = [
                    &s[done..done + n],
                    &s[done..done + n],
                    &quiet[..n],
                    &quiet[..n],
                ];
                let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
                m.mix_block(&inputs, &mut l, &mut r);
                out.extend(l);
                done += n;
            }
            out
        };
        let reference = render(512);
        for block in [1usize, 37, 64] {
            assert_eq!(
                render(block),
                reference,
                "block size {block} changed the render"
            );
        }
    }

    #[test]
    fn bus_sums_in_at_its_own_fader_and_mute() {
        // REQ-409 groundwork: the bus is part of the mix, at its own
        // gain, with no pan.
        let mut m = Mixer::new();
        let quiet = silence(4800);
        let inputs: [&[f32]; NUM_TRACKS] = [&quiet, &quiet, &quiet, &quiet];
        let bus = sine(440.0, -6.0, 4800);

        let run = |m: &mut Mixer| {
            let (mut l, mut r) = (vec![0.0; 4800], vec![0.0; 4800]);
            m.sum_tracks(&inputs, &mut l, &mut r, None);
            m.finish_mix(&mut l, &mut r, Some((&bus, &bus)), None);
            (rms_dbfs(&l[2400..]), rms_dbfs(&r[2400..]))
        };

        let (unity_l, unity_r) = run(&mut m);
        assert!(unity_l > -20.0, "bus must reach the output");
        assert!(
            (unity_l - unity_r).abs() < 0.01,
            "no pan: both channels equal"
        );

        m.set_bus_fader_db(-12.0);
        let (cut, _) = run(&mut m);
        assert!(
            (unity_l - cut - 12.0).abs() < 0.5,
            "bus fader should cut ~12dB, got {:.1}",
            unity_l - cut
        );

        m.set_bus_muted(true);
        let (muted, _) = run(&mut m);
        assert!(muted < -100.0, "muted bus must be silent, got {muted:.1}");
    }

    #[test]
    fn mute_silences_a_track_without_touching_its_fader() {
        let mut m = Mixer::new();
        m.set_fader_db(0, -6.0);
        m.set_muted(0, true);
        let s = sine(1000.0, 0.0, 4800);
        mix_once(&mut m, 0, &s); // settle the ramp
        let (l, r) = mix_once(&mut m, 0, &s);
        assert!(
            rms_dbfs(&l) < -80.0 && rms_dbfs(&r) < -80.0,
            "muted track should be silent, got l={} r={}",
            rms_dbfs(&l),
            rms_dbfs(&r)
        );
        assert_eq!(
            m.fader_db(0),
            -6.0,
            "mute must not disturb the fader value itself"
        );
        assert!(
            m.track_level_db(0) < -80.0,
            "meter should read silent while muted"
        );

        m.set_muted(0, false);
        mix_once(&mut m, 0, &s); // settle the unmute ramp
        let (l, _) = mix_once(&mut m, 0, &s);
        assert_rms_near_db!(&l, -3.01 - 3.01 - 6.0, 0.1);
    }

    #[test]
    fn output_never_exceeds_full_scale_even_with_hot_gain_staging() {
        // Four tracks, each already at 0dBFS peak, all faders pushed to
        // the UI's new maximum (+12dB): a real "gain was too high"
        // scenario, not a synthetic edge case. Unclamped this would
        // peak at roughly +12dB over full scale - loud enough to be a
        // real hazard through headphones, found 2026-08-21 from an
        // actual feedback/peak report while recording.
        let mut m = Mixer::new();
        for t in 0..NUM_TRACKS {
            m.set_fader_db(t, 12.0);
        }
        let s = sine(500.0, 0.0, 4800);
        let inputs: [&[f32]; NUM_TRACKS] = [&s, &s, &s, &s];
        let mut l = vec![0.0; 4800];
        let mut r = vec![0.0; 4800];
        m.mix_block(&inputs, &mut l, &mut r);
        m.mix_block(&inputs, &mut l, &mut r); // let the fader ramp settle
        let peak = l
            .iter()
            .chain(r.iter())
            .fold(0.0f32, |acc, &s| acc.max(s.abs()));
        assert!(peak <= 1.0, "output exceeded full scale: peak={peak}");
    }

    #[test]
    fn four_tracks_sum() {
        let mut m = Mixer::new();
        let s = sine(500.0, -12.0, 4800);
        let inputs: [&[f32]; NUM_TRACKS] = [&s, &s, &s, &s];
        let mut l = vec![0.0; 4800];
        let mut r = vec![0.0; 4800];
        m.mix_block(&inputs, &mut l, &mut r);
        m.mix_block(&inputs, &mut l, &mut r);
        // Four coherent copies = +12 dB over one track's contribution.
        assert_rms_near_db!(&l, -12.0 - 3.01 - 3.01 + 12.04, 0.15);
    }

    #[test]
    fn silent_track_meters_at_the_floor() {
        let mut m = Mixer::new();
        mix_once(&mut m, 0, &silence(4800));
        assert_eq!(m.track_level_db(0), FLOOR_DBFS);
        assert_eq!(m.track_level_db(1), FLOOR_DBFS);
        let (ml, mr) = m.master_level_db();
        assert_eq!(ml, FLOOR_DBFS);
        assert_eq!(mr, FLOOR_DBFS);
    }

    #[test]
    fn track_level_tracks_the_input_peak() {
        let mut m = Mixer::new();
        let s = sine(1000.0, -6.0, 4800); // -6 dBFS peak
        mix_once(&mut m, 1, &s);
        assert!(
            (m.track_level_db(1) - (-6.0)).abs() < 0.2,
            "got {} dB",
            m.track_level_db(1)
        );
        // Untouched tracks stay at the floor, not the same reading.
        assert_eq!(m.track_level_db(0), FLOOR_DBFS);
    }

    #[test]
    fn track_level_follows_the_fader_but_not_the_master() {
        let mut m = Mixer::new();
        let s = sine(1000.0, 0.0, 48_000); // let the 5ms ramp settle first
        mix_once(&mut m, 2, &s);
        let unity = mix_once_meter(&mut m, 2, &s);

        m.set_fader_db(2, -10.0);
        mix_once(&mut m, 2, &s); // settle the new fader ramp
        let faded = mix_once_meter(&mut m, 2, &s);
        assert!(
            (unity - faded - 10.0).abs() < 0.3,
            "fader -10dB should read -10dB lower on the meter, got unity={unity} faded={faded}"
        );

        // Master is a separate knob for the whole bus, not per track:
        // it must not move an individual track's meter.
        m.set_master_db(-20.0);
        mix_once(&mut m, 2, &s);
        let with_master_down = mix_once_meter(&mut m, 2, &s);
        assert!(
            (faded - with_master_down).abs() < 0.3,
            "master fader must not affect the per-track meter, got {faded} vs {with_master_down}"
        );
    }

    #[test]
    fn master_level_reflects_the_mix() {
        let mut m = Mixer::new();
        let s = sine(1000.0, 0.0, 48_000);
        mix_once(&mut m, 0, &s); // settle
        mix_once(&mut m, 0, &s);
        let (ml, mr) = m.master_level_db();
        // Center pan: -3.01 dB per side off a 0 dBFS-peak sine.
        assert!((ml - (-3.01)).abs() < 0.2, "left {ml} dB");
        assert!((mr - (-3.01)).abs() < 0.2, "right {mr} dB");

        m.set_master_db(-12.0);
        mix_once(&mut m, 0, &s); // settle the new master ramp
        mix_once(&mut m, 0, &s);
        let (ml2, _) = m.master_level_db();
        assert!(
            (ml - ml2 - 12.0).abs() < 0.3,
            "master fader should reach the master meter"
        );
    }

    /// Mix one block and return the track's post-fader meter reading.
    fn mix_once_meter(mixer: &mut Mixer, track: usize, signal: &[f32]) -> f32 {
        mix_once(mixer, track, signal);
        mixer.track_level_db(track)
    }
}
