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
    left: [Smoothed; NUM_TRACKS],
    right: [Smoothed; NUM_TRACKS],
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
            left: [Smoothed::settled(l); NUM_TRACKS],
            right: [Smoothed::settled(r); NUM_TRACKS],
            track_peak: [0.0; NUM_TRACKS],
            master_peak: (0.0, 0.0),
        }
    }

    pub fn set_fader_db(&mut self, track: usize, db: f32) {
        self.fader_db[track] = db;
    }

    pub fn set_pan(&mut self, track: usize, pan: f32) {
        self.pan[track] = pan.clamp(-1.0, 1.0);
    }

    pub fn set_master_db(&mut self, db: f32) {
        self.master_db = db;
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

    fn target(&self, track: usize) -> (f32, f32) {
        let amp = db_to_amp(self.fader_db[track]) * db_to_amp(self.master_db);
        let (l, r) = pan_gains(self.pan[track]);
        (amp * l, amp * r)
    }

    /// Mix one block of the four track signals into stereo out. All
    /// slices must share the same length.
    pub fn mix_block(
        &mut self,
        inputs: &[&[f32]; NUM_TRACKS],
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        let len = out_l.len();
        assert_eq!(out_r.len(), len);
        out_l.fill(0.0);
        out_r.fill(0.0);
        if len == 0 {
            return;
        }
        for (t, input) in inputs.iter().enumerate() {
            assert_eq!(input.len(), len);
            let (tl, tr) = self.target(t);
            self.left[t].set_target(tl);
            self.right[t].set_target(tr);
            let fader_amp = db_to_amp(self.fader_db[t]);
            let mut peak = 0.0f32;
            for (n, &s) in input.iter().enumerate() {
                out_l[n] += s * self.left[t].tick();
                out_r[n] += s * self.right[t].tick();
                peak = peak.max(s.abs());
            }
            // Fader only, not master: lets the meters show track
            // balance independent of the overall volume knob.
            self.track_peak[t] = peak * fader_amp;
        }
        // Hard safety ceiling on the summed output, not just the
        // offline WAV writer's own clamp (render.rs quantizes with one
        // too - this makes the live monitoring path match it instead
        // of being the one place nothing stops an extreme value from
        // reaching a real speaker or headphones). Four tracks at up to
        // +12dB of fader gain each plus the master can genuinely sum
        // past 0dBFS; this is the last line of defense before that
        // reaches hardware, not a substitute for sane gain staging.
        for s in out_l.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        for s in out_r.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        self.master_peak = (
            out_l.iter().fold(0.0f32, |acc, &s| acc.max(s.abs())),
            out_r.iter().fold(0.0f32, |acc, &s| acc.max(s.abs())),
        );
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
