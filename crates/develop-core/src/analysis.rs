//! Measuring pictures: the histogram shown above the Develop panel, which Auto also reads.

use rayon::prelude::*;

use crate::bitmap::Bitmap;

/// How many pixels have each value, per channel, counting only pixels that aren't transparent.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    pub red: [u32; 256],
    pub green: [u32; 256],
    pub blue: [u32; 256],
    pub luma: [u32; 256],
    pub total: u32,
}

impl Histogram {
    /// Counts a bitmap's pixels (straight values, so half-transparent edges count true to color).
    /// Big pictures are sampled on a grid, which keeps it quick and changes little.
    pub fn of(bitmap: &Bitmap) -> Histogram {
        let pixels = bitmap.width as usize * bitmap.height as usize;
        let step = ((pixels / 2_000_000).max(1) as f32).sqrt().ceil() as usize;
        let width = bitmap.width as usize;
        let rows: Vec<usize> = (0..bitmap.height as usize).step_by(step).collect();
        // Each task counts a band of rows into its own tally on the heap (a tally is 4 KB, too
        // big to pass around on worker threads' stacks), and the tallies are added up after.
        let parts: Vec<Box<Histogram>> = rows
            .par_chunks(64)
            .map(|band| {
                let mut part = Box::new(Histogram::empty());
                for &y in band {
                    for x in (0..width).step_by(step) {
                        let index = (y * width + x) * 4;
                        let alpha = bitmap.data[index + 3];
                        if alpha == 0 {
                            continue;
                        }
                        let straight = |channel: u8| ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as usize;
                        let (r, g, b) = (straight(bitmap.data[index]), straight(bitmap.data[index + 1]), straight(bitmap.data[index + 2]));
                        part.red[r] += 1;
                        part.green[g] += 1;
                        part.blue[b] += 1;
                        part.luma[(r * 54 + g * 183 + b * 19) >> 8] += 1;
                        part.total += 1;
                    }
                }
                part
            })
            .collect();
        let mut total = Histogram::empty();
        for part in parts {
            for value in 0..256 {
                total.red[value] += part.red[value];
                total.green[value] += part.green[value];
                total.blue[value] += part.blue[value];
                total.luma[value] += part.luma[value];
            }
            total.total += part.total;
        }
        total
    }

    pub fn empty() -> Histogram {
        Histogram { red: [0; 256], green: [0; 256], blue: [0; 256], luma: [0; 256], total: 0 }
    }

    /// The value (0–1) below which `fraction` of the pixels' brightness falls.
    pub fn percentile_luma(&self, fraction: f32) -> f32 {
        percentile(&self.luma, self.total, fraction)
    }
}

fn percentile(counts: &[u32; 256], total: u32, fraction: f32) -> f32 {
    if total == 0 {
        return if fraction < 0.5 { 0.0 } else { 1.0 };
    }
    let wanted = (total as f32 * fraction.clamp(0.0, 1.0)) as u64;
    let mut seen = 0u64;
    for (value, &count) in counts.iter().enumerate() {
        seen += u64::from(count);
        if seen > wanted {
            return value as f32 / 255.0;
        }
    }
    1.0
}
