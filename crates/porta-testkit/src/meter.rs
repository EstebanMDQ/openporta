//! Level meters. All results in dBFS; silence clamps to `FLOOR_DBFS`
//! instead of negative infinity so assertions stay finite.

/// Lowest level any meter reports.
pub const FLOOR_DBFS: f32 = -160.0;

fn to_dbfs(linear: f32) -> f32 {
    if linear <= 0.0 {
        FLOOR_DBFS
    } else {
        (20.0 * linear.log10()).max(FLOOR_DBFS)
    }
}

pub fn rms_dbfs(signal: &[f32]) -> f32 {
    if signal.is_empty() {
        return FLOOR_DBFS;
    }
    let sum_sq: f64 = signal.iter().map(|&s| (s as f64) * (s as f64)).sum();
    to_dbfs((sum_sq / signal.len() as f64).sqrt() as f32)
}

pub fn peak_dbfs(signal: &[f32]) -> f32 {
    let peak = signal.iter().fold(0f32, |acc, &s| acc.max(s.abs()));
    to_dbfs(peak)
}

/// Windowed RMS in dBFS, one value per full window (remainder dropped).
pub fn rms_envelope(signal: &[f32], window: usize) -> Vec<f32> {
    assert!(window > 0, "window must be non-zero");
    signal.chunks_exact(window).map(rms_dbfs).collect()
}
