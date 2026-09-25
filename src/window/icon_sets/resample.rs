//! Choosing a stored icon size for the tree's icon box and resampling it to the box's exact size
//! (icon sets spec §6): area averaging on premultiplied BGRA, done once per icon and size.

use super::material::SIZES;

/// The stored size to draw a `px`-square box from: `px` itself when stored, else the next size
/// up, else the largest.
pub(crate) fn pick_size(px: u32) -> u32 {
    SIZES
        .iter()
        .copied()
        .find(|&size| size >= px)
        .unwrap_or(SIZES[SIZES.len() - 1])
}

/// For each of `to` output pixels, the input pixels it covers and their weights (summing to 1).
fn taps(from: u32, to: u32) -> Vec<Vec<(usize, f32)>> {
    let scale = from as f32 / to as f32;
    (0..to)
        .map(|out| {
            let start = out as f32 * scale;
            let end = start + scale;
            let mut taps = Vec::new();
            let mut source = start.floor() as usize;
            while (source as f32) < end && source < from as usize {
                let overlap = end.min(source as f32 + 1.0) - start.max(source as f32);
                if overlap > 0.0 {
                    taps.push((source, overlap / scale));
                }
                source += 1;
            }
            taps
        })
        .collect()
}

/// `pixels` (`from`-square premultiplied BGRA, top-down) resampled to a `to` square. Averaging
/// premultiplied channels keeps each colour channel at or below its alpha.
pub(crate) fn resample(pixels: &[u8], from: u32, to: u32) -> Vec<u8> {
    if from == to {
        return pixels.to_vec();
    }
    let (from_n, to_n) = (from as usize, to as usize);
    let taps = taps(from, to);
    // Rows first: `from` rows of `to` columns.
    let mut wide = vec![0_f32; from_n * to_n * 4];
    for y in 0..from_n {
        for (x, column) in taps.iter().enumerate() {
            for &(source, weight) in column {
                for channel in 0..4 {
                    wide[(y * to_n + x) * 4 + channel] +=
                        f32::from(pixels[(y * from_n + source) * 4 + channel]) * weight;
                }
            }
        }
    }
    let mut out = vec![0_u8; to_n * to_n * 4];
    for (y, row) in taps.iter().enumerate() {
        for x in 0..to_n {
            for channel in 0..4 {
                let value: f32 = row
                    .iter()
                    .map(|&(source, weight)| wide[(source * to_n + x) * 4 + channel] * weight)
                    .sum();
                out[(y * to_n + x) * 4 + channel] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::icon_sets::material::{MaterialIcon, pixels};

    #[test]
    fn a_box_size_uses_its_stored_size_else_the_next_one_up_else_the_largest() {
        // Break caught: 175% (28 px) drawn from 24 (upscaled, soft) or 300%+ finding nothing.
        assert_eq!(pick_size(16), 16);
        assert_eq!(pick_size(20), 20);
        assert_eq!(pick_size(28), 32);
        assert_eq!(pick_size(36), 48);
        assert_eq!(pick_size(64), 48);
        assert_eq!(pick_size(1), 16);
    }

    #[test]
    fn resampling_keeps_solid_areas_averages_edges_and_stays_premultiplied() {
        // Break caught: an opaque icon given see-through seams, an edge dropped instead of
        // blended, or channels above alpha (spec §6).
        let solid: Vec<u8> = [40, 80, 120, 255].repeat(32 * 32);
        for to in [20, 28, 40, 64] {
            let out = resample(&solid, 32, to);
            assert_eq!(out.len(), (to * to * 4) as usize);
            assert!(
                out.as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| p == &[40, 80, 120, 255]),
                "{to}"
            );
        }
        // Left column opaque white, right column clear: one pixel, half covered.
        let half = [
            255, 255, 255, 255, 0, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0,
        ];
        assert_eq!(resample(&half, 2, 1), vec![128, 128, 128, 128]);
        for icon in MaterialIcon::ALL {
            let out = resample(pixels(icon, 32).unwrap(), 32, 28);
            assert!(
                out.as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]),
                "{icon:?}"
            );
        }
    }
}
