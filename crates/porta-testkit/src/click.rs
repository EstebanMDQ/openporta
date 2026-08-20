//! Click / discontinuity detection: the main quality instrument for an
//! agent that cannot listen. A click is a first-difference outlier
//! relative to the local derivative energy, so loud legitimate content
//! does not trigger it while a splice discontinuity does.

/// Detection window for local derivative RMS (10ms at 48kHz).
const WINDOW: usize = 480;
/// A delta must exceed this many times the local derivative RMS.
const RATIO: f32 = 8.0;
/// And exceed this absolute floor, so silence-adjacent noise is ignored.
const ABS_FLOOR: f32 = 0.02;

/// Sample indices where a discontinuity was detected.
pub fn find_clicks(signal: &[f32]) -> Vec<usize> {
    if signal.len() < 2 {
        return Vec::new();
    }
    let deltas: Vec<f32> = signal.windows(2).map(|w| w[1] - w[0]).collect();
    let mut clicks = Vec::new();
    for (i, &d) in deltas.iter().enumerate() {
        let lo = i.saturating_sub(WINDOW / 2);
        let hi = (i + WINDOW / 2).min(deltas.len());
        // Local RMS of the surrounding derivatives, excluding the candidate
        // itself so a lone spike cannot mask itself.
        let mut sum_sq = 0f32;
        let mut count = 0usize;
        for (j, &dj) in deltas[lo..hi].iter().enumerate() {
            if lo + j != i {
                sum_sq += dj * dj;
                count += 1;
            }
        }
        let local_rms = if count > 0 {
            (sum_sq / count as f32).sqrt()
        } else {
            0.0
        };
        if d.abs() > ABS_FLOOR && d.abs() > RATIO * local_rms.max(ABS_FLOOR / RATIO) {
            clicks.push(i + 1);
        }
    }
    clicks
}
