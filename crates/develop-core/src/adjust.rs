//! What every slider in the Develop panel is described by (its range, default and the look of
//! its track), and the tone curves: a curve through a few points turned into a smooth table.

/// What the track behind a slider shows, so a setting reads at a glance (a warm-to-cool track
/// for temperature, say).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    Plain,
    /// Dark to light.
    Light,
    /// Blue to amber.
    Temperature,
    /// Green to magenta.
    Tint,
    /// Gray to vivid.
    Saturation,
    /// Every hue around the wheel.
    Hue,
}

/// One numeric setting: its name, range, default and how it is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slider {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub decimals: usize,
    pub unit: &'static str,
    pub track: Track,
}

/// A curve through points (x, y in 0–1), for Curves and the tone curve.
pub type CurvePoints = Vec<[f32; 2]>;

/// The straight line every curve starts as.
pub fn identity_curve() -> CurvePoints {
    vec![[0.0, 0.0], [1.0, 1.0]]
}

/// A 256-entry table for a curve, interpolated smoothly through its points without
/// overshooting (monotone cubic, as Photoshop's Curves feel).
pub fn curve_table(points: &[[f32; 2]]) -> [f32; 256] {
    let mut sorted: Vec<[f32; 2]> = points.iter().copied().filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));
    sorted.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-4);
    let mut table = [0.0f32; 256];
    if sorted.len() < 2 {
        for (index, value) in table.iter_mut().enumerate() {
            *value = index as f32 / 255.0;
        }
        return table;
    }
    let count = sorted.len();
    // Slopes between points, then tangents limited so the curve never overshoots (Fritsch–Carlson).
    let deltas: Vec<f32> = sorted.windows(2).map(|pair| (pair[1][1] - pair[0][1]) / (pair[1][0] - pair[0][0]).max(1e-6)).collect();
    let mut tangents = vec![0.0f32; count];
    tangents[0] = deltas[0];
    tangents[count - 1] = deltas[count - 2];
    for index in 1..count - 1 {
        tangents[index] = if deltas[index - 1] * deltas[index] <= 0.0 { 0.0 } else { (deltas[index - 1] + deltas[index]) * 0.5 };
    }
    for index in 0..count - 1 {
        if deltas[index].abs() < 1e-9 {
            tangents[index] = 0.0;
            tangents[index + 1] = 0.0;
            continue;
        }
        let a = tangents[index] / deltas[index];
        let b = tangents[index + 1] / deltas[index];
        let length = a.hypot(b);
        if length > 3.0 {
            let factor = 3.0 / length;
            tangents[index] = factor * a * deltas[index];
            tangents[index + 1] = factor * b * deltas[index];
        }
    }
    for (index, value) in table.iter_mut().enumerate() {
        let x = index as f32 / 255.0;
        *value = if x <= sorted[0][0] {
            sorted[0][1]
        } else if x >= sorted[count - 1][0] {
            sorted[count - 1][1]
        } else {
            let segment = sorted.windows(2).position(|pair| x <= pair[1][0]).unwrap_or(count - 2);
            let [x0, y0] = sorted[segment];
            let [x1, y1] = sorted[segment + 1];
            let h = (x1 - x0).max(1e-6);
            let t = (x - x0) / h;
            let (t2, t3) = (t * t, t * t * t);
            (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * tangents[segment] + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * tangents[segment + 1]
        }
        .clamp(0.0, 1.0);
    }
    table
}
