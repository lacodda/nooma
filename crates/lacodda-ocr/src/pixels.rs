//! A picture as the recognizers are given it: shades of grey on paper, at
//! the size its text reads best at.

/// A picture in shades of grey, a byte a pixel, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grey {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) luma: Vec<u8>,
}

impl Grey {
    /// Grey out of blue, green, red and an alpha not multiplied in, put on
    /// white paper. A picture cut out of a document is black text on
    /// nothing, and a recognizer, which does not look at the alpha, would
    /// read black on black.
    pub(crate) fn from_bgra(width: u32, height: u32, bgra: &[u8]) -> Self {
        let (pixels, _) = bgra.as_chunks::<4>();
        let luma = pixels
            .iter()
            .map(|pixel| {
                let (blue, green, red, alpha) = (pixel[0] as f32, pixel[1] as f32, pixel[2] as f32, pixel[3] as f32 / 255.0);
                let ink = 0.299 * red + 0.587 * green + 0.114 * blue;
                (ink * alpha + 255.0 * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Self { width, height, luma }
    }

    /// The picture at another size: each pixel a weighted average of those
    /// it covers, by a triangle as wide as one pixel of the larger of the
    /// two. It interpolates when the picture grows and averages when it
    /// shrinks, so thin strokes neither block up nor flicker away.
    pub(crate) fn resized(&self, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let across = weights(self.width, width);
        let down = weights(self.height, height);
        let (from_width, to_width) = (self.width as usize, width as usize);
        // Across first: every source row at the new width.
        let mut rows = vec![0f32; self.height as usize * to_width];
        for (y, row) in rows.chunks_exact_mut(to_width).enumerate() {
            let source = &self.luma[y * from_width..(y + 1) * from_width];
            for (out, (start, taps)) in row.iter_mut().zip(&across) {
                *out = taps.iter().enumerate().map(|(k, weight)| weight * f32::from(source[start + k])).sum();
            }
        }
        // Then down.
        let mut luma = vec![0u8; height as usize * to_width];
        for (y, (start, taps)) in down.iter().enumerate() {
            for x in 0..to_width {
                let value: f32 = taps.iter().enumerate().map(|(k, weight)| weight * rows[(start + k) * to_width + x]).sum();
                luma[y * to_width + x] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
        Self { width, height, luma }
    }
}

/// For each pixel of a row `to` long, the first pixel of a row `from` long it
/// takes from and the weights of those it takes.
fn weights(from: u32, to: u32) -> Vec<(usize, Vec<f32>)> {
    let scale = to as f64 / from as f64;
    // The triangle's half-width in source pixels: one, or one target pixel
    // when shrinking.
    let radius = if scale < 1.0 { 1.0 / scale } else { 1.0 };
    let last = from as usize - 1;
    (0..to)
        .map(|index| {
            let center = (index as f64 + 0.5) / scale - 0.5;
            let first = ((center - radius).floor().max(0.0) as usize).min(last);
            let end = ((center + radius).ceil().max(0.0) as usize).min(last);
            let mut taps: Vec<f32> = (first..=end)
                .map(|source| (1.0 - (source as f64 - center).abs() / radius).max(0.0) as f32)
                .collect();
            let sum: f32 = taps.iter().sum();
            if sum > 0.0 {
                taps.iter_mut().for_each(|weight| *weight /= sum);
                (first, taps)
            } else {
                // Past the edge: the edge pixel.
                (center.round().clamp(0.0, last as f64) as usize, vec![1.0])
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(width: u32, height: u32, at: impl Fn(u32, u32) -> u8) -> Grey {
        let luma = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| at(x, y)).collect();
        Grey { width, height, luma }
    }

    #[test]
    fn transparent_is_paper_and_ink_stays_ink() {
        let bgra = [0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 0, 128, 255, 255, 255, 255, 0, 0, 255, 255];
        let picture = Grey::from_bgra(5, 1, &bgra);
        assert_eq!(picture.luma[..4], [255, 0, 127, 255]);
        assert_eq!(picture.luma[4], 76, "red is the grey it looks");
    }

    #[test]
    fn a_flat_picture_stays_flat_at_any_size() {
        let flat = grey(7, 5, |_, _| 90);
        for (width, height) in [(14, 10), (3, 2), (19, 1), (1, 1)] {
            assert!(flat.resized(width, height).luma.iter().all(|&value| value == 90), "{width}x{height}");
        }
    }

    #[test]
    fn growing_keeps_an_edge_where_it_was() {
        let edge = grey(4, 1, |x, _| if x < 2 { 0 } else { 255 });
        let grown = edge.resized(8, 1);
        assert_eq!(grown.luma.len(), 8);
        assert!(
            grown.luma[..3].iter().all(|&v| v < 40) && grown.luma[5..].iter().all(|&v| v > 215),
            "{:?}",
            grown.luma
        );
        assert!(grown.luma.windows(2).all(|pair| pair[0] <= pair[1]), "no ringing: {:?}", grown.luma);
    }

    #[test]
    fn shrinking_averages_what_it_covers() {
        // Lines of ink one pixel apart become the grey they look like from
        // further away, not stripes or blank paper.
        let stripes = grey(40, 40, |x, _| if x % 2 == 0 { 0 } else { 255 });
        let shrunk = stripes.resized(10, 10);
        assert!(shrunk.luma.iter().all(|&value| (100..=155).contains(&value)), "{:?}", &shrunk.luma[..10]);
    }
}
