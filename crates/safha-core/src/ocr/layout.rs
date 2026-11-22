//! Page assessment and text-line strips, without a detector (spec §8 steps 2–4, ADR 0033).
//!
//! - **Ink:** Otsu's threshold, then isolated specks removed. A pixel stays if its 5x5
//!   window holds at least 9 ink pixels; strokes do, the specks of faxes and
//!   photocopies do not.
//! - **Skew:** the angle (±4°, in 0.25° steps) whose row profile is sharpest.
//! - **Lines:** bands of the horizontal ink profile of the deskewed page. The threshold
//!   adapts to the page's speck level, rows over 70% ink (background around the
//!   paper, shadows) are skipped, and bands with little ink are dropped.
//!
//! The strips are for re-reading at full resolution (`reconcile`), not word boxes. On
//! a curved page several lines can share a strip; the model reads multi-line crops too.

use super::image::OcrImage;

/// What the page looks like, and where its lines are.
#[derive(Clone, Debug, PartialEq)]
pub struct PageLayout {
    /// Degrees the page was rotated by to level its lines (positive: rotated
    /// clockwise). [`deskew`] applies it.
    pub skew_degrees: f32,
    /// Text-line strips in the deskewed page, top to bottom.
    pub lines: Vec<LineBox>,
    /// Share of the page that is ink after speck removal.
    pub ink_fraction: f32,
    /// Share of dark pixels removed as specks: a noise measure for assessment.
    pub speck_fraction: f32,
}

/// A strip of the (deskewed) page: `x0..x1`, `y0..y1` in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineBox {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// Assesses the page and finds its line strips. The boxes refer to
/// `deskew(image, layout.skew_degrees)`.
pub fn analyze(image: &OcrImage) -> PageLayout {
    let gray = luma(image);
    let (w, h) = (image.width() as usize, image.height() as usize);
    let (raw, ink) = ink_mask(&gray, w, h);
    let raw_count = raw.iter().filter(|&&b| b).count();
    let ink_count = ink.iter().filter(|&&b| b).count();
    let skew = skew_angle(&ink, w, h);
    let (lines, ink_fraction) = if skew.abs() >= 0.25 {
        let level = deskew(image, skew);
        let gray = luma(&level);
        let (_, ink) = ink_mask(&gray, w, h);
        (line_strips(&ink, w, h), ink.iter().filter(|&&b| b).count() as f32 / (w * h) as f32)
    } else {
        (line_strips(&ink, w, h), ink_count as f32 / (w * h) as f32)
    };
    PageLayout {
        skew_degrees: skew,
        lines,
        ink_fraction,
        speck_fraction: if raw_count == 0 { 0.0 } else { (raw_count - ink_count) as f32 / raw_count as f32 },
    }
}

/// `image` rotated by `degrees` about its centre (positive: clockwise), same size,
/// white where the rotation uncovers the canvas. Bilinear.
pub fn deskew(image: &OcrImage, degrees: f32) -> OcrImage {
    if degrees == 0.0 {
        return image.clone();
    }
    let (w, h) = (image.width() as usize, image.height() as usize);
    let (sin, cos) = f64::from(degrees).to_radians().sin_cos();
    let (cx, cy) = ((w as f64 - 1.0) / 2.0, (h as f64 - 1.0) / 2.0);
    let src = image.rgb();
    let mut out = vec![255u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            // Inverse map: the source point that lands on (x, y).
            let (dx, dy) = (x as f64 - cx, y as f64 - cy);
            let sx = cos * dx + sin * dy + cx;
            let sy = -sin * dx + cos * dy + cy;
            if sx < 0.0 || sy < 0.0 || sx > (w - 1) as f64 || sy > (h - 1) as f64 {
                continue;
            }
            let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (fx, fy) = (sx - x0 as f64, sy - y0 as f64);
            for c in 0..3 {
                let p = |xx: usize, yy: usize| f64::from(src[(yy * w + xx) * 3 + c]);
                let v = p(x0, y0) * (1.0 - fx) * (1.0 - fy)
                    + p(x1, y0) * fx * (1.0 - fy)
                    + p(x0, y1) * (1.0 - fx) * fy
                    + p(x1, y1) * fx * fy;
                out[(y * w + x) * 3 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    // Same dimensions as the input, so this cannot fail.
    OcrImage::from_rgb(image.width(), image.height(), out).unwrap_or_else(|_| image.clone())
}

/// Pillow's `convert("L")`.
fn luma(image: &OcrImage) -> Vec<u8> {
    image
        .rgb()
        .as_chunks::<3>()
        .0
        .iter()
        .map(|[r, g, b]| {
            ((u32::from(*r) * 19595 + u32::from(*g) * 38470 + u32::from(*b) * 7471 + 0x8000) >> 16) as u8
        })
        .collect()
}

fn otsu(gray: &[u8]) -> u8 {
    let mut hist = [0f64; 256];
    for &v in gray {
        hist[v as usize] += 1.0;
    }
    let total = gray.len() as f64;
    let sum_all: f64 = hist.iter().enumerate().map(|(i, &n)| i as f64 * n).sum();
    let (mut w0, mut sum0, mut best, mut threshold) = (0.0, 0.0, -1.0, 128u8);
    for (t, &count) in hist.iter().enumerate().take(255) {
        w0 += count;
        sum0 += t as f64 * count;
        let w1 = total - w0;
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }
        let (m0, m1) = (sum0 / w0, (sum_all - sum0) / w1);
        let between = w0 * w1 * (m0 - m1) * (m0 - m1);
        if between > best {
            best = between;
            threshold = (t + 1) as u8;
        }
    }
    threshold
}

/// (dark pixels, ink without isolated specks), row-major.
fn ink_mask(gray: &[u8], w: usize, h: usize) -> (Vec<bool>, Vec<bool>) {
    let t = otsu(gray);
    let raw: Vec<bool> = gray.iter().map(|&v| v < t).collect();
    // Integral image for 5x5 window counts.
    let stride = w + 1;
    let mut integral = vec![0u32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += u32::from(raw[y * w + x]);
            integral[(y + 1) * stride + x + 1] = integral[y * stride + x + 1] + row;
        }
    }
    let count = |x0: usize, y0: usize, x1: usize, y1: usize| {
        integral[y1 * stride + x1] + integral[y0 * stride + x0]
            - integral[y0 * stride + x1]
            - integral[y1 * stride + x0]
    };
    let mut ink = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if raw[y * w + x] {
                let (x0, y0) = (x.saturating_sub(2), y.saturating_sub(2));
                let (x1, y1) = ((x + 3).min(w), (y + 3).min(h));
                ink[y * w + x] = count(x0, y0, x1, y1) >= 9;
            }
        }
    }
    (raw, ink)
}

fn skew_angle(ink: &[bool], w: usize, h: usize) -> f32 {
    let points: Vec<(usize, usize)> =
        (0..h).flat_map(|y| (0..w).filter(move |&x| ink[y * w + x]).map(move |x| (x, y))).collect();
    if points.is_empty() {
        return 0.0;
    }
    let step = points.len().div_ceil(400_000);
    let mut best = (f64::MIN, 0.0f32);
    let mut profile = vec![0f64; h];
    for k in -16..=16 {
        let angle = k as f32 * 0.25;
        let t = f64::from(angle).to_radians().tan();
        profile.iter_mut().for_each(|v| *v = 0.0);
        let mut n = 0.0;
        for &(x, y) in points.iter().step_by(step) {
            let row = (y as f64 + x as f64 * t).round();
            if row >= 0.0 && (row as usize) < h {
                profile[row as usize] += 1.0;
                n += 1.0;
            }
        }
        let mean = n / h as f64;
        let variance = profile.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / h as f64;
        if variance > best.0 + 1e-9 {
            best = (variance, angle);
        }
    }
    best.1
}

fn percentile(values: &[u32], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    // numpy's default (linear interpolation).
    let rank = p / 100.0 * (sorted.len() - 1) as f64;
    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
    f64::from(sorted[lo]) + (f64::from(sorted[hi]) - f64::from(sorted[lo])) * (rank - lo as f64)
}

fn line_strips(ink: &[bool], w: usize, h: usize) -> Vec<LineBox> {
    // Ignore page edges and columns that are mostly ink (scanner shadows).
    let mut usable = vec![true; w];
    for (x, slot) in usable.iter_mut().enumerate() {
        let column = (0..h).filter(|&y| ink[y * w + x]).count();
        *slot = column * 2 < h && x >= w * 2 / 100 && x < w * 98 / 100;
    }
    let usable_width = usable.iter().filter(|&&u| u).count().max(1);
    let rows: Vec<u32> =
        (0..h).map(|y| (0..w).filter(|&x| usable[x] && ink[y * w + x]).count() as u32).collect();
    // Rows mostly covered in ink are background around the paper or a shadow.
    let valid: Vec<bool> = rows.iter().map(|&r| (r as f64) < 0.7 * usable_width as f64).collect();
    let valid_rows: Vec<u32> = rows.iter().zip(&valid).filter(|(_, v)| **v).map(|(r, _)| *r).collect();
    if valid_rows.is_empty() {
        return Vec::new();
    }
    let strong = percentile(&valid_rows, 95.0);
    let floor = percentile(&valid_rows, 30.0);
    let threshold = 2f64.max((0.002 * w as f64).floor()).max(floor + 0.15 * (strong - floor));
    let on: Vec<bool> = rows.iter().zip(&valid).map(|(&r, &v)| v && f64::from(r) > threshold).collect();

    let mut bands: Vec<(usize, usize)> = Vec::new();
    let mut y = 0;
    while y < h {
        if on[y] {
            let start = y;
            while y < h && on[y] {
                y += 1;
            }
            bands.push((start, y));
        } else {
            y += 1;
        }
    }
    if bands.is_empty() {
        return Vec::new();
    }
    // Bands of a few specks hold little ink; text lines hold far more.
    let mass: Vec<u32> = bands.iter().map(|&(a, b)| rows[a..b].iter().sum()).collect();
    let big = percentile(&mass, 75.0);
    let bands: Vec<(usize, usize)> =
        bands.into_iter().zip(&mass).filter(|(_, m)| f64::from(**m) >= 0.1 * big).map(|(b, _)| b).collect();
    let mut heights: Vec<usize> = bands.iter().map(|(a, b)| b - a).collect();
    heights.sort_unstable();
    let median = heights.get(heights.len() / 2).copied().unwrap_or(0) as f64;
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in bands.into_iter().filter(|(a, b)| (b - a) as f64 >= 0.3 * median) {
        match merged.last_mut() {
            Some(last) if ((a - last.1) as f64) < 0.35 * median => last.1 = b,
            _ => merged.push((a, b)),
        }
    }
    merged
        .into_iter()
        .filter_map(|(y0, y1)| {
            let lh = y1 - y0;
            let min_count = 1f64.max(0.03 * lh as f64);
            let cols: Vec<usize> = (0..w)
                .filter(|&x| usable[x] && ((y0..y1).filter(|&yy| ink[yy * w + x]).count() as f64) > min_count)
                .collect();
            let (first, last) = (*cols.first()?, *cols.last()?);
            let (pad_x, pad_y) = (lh * 6 / 10, lh * 35 / 100);
            Some(LineBox {
                x0: first.saturating_sub(pad_x) as u32,
                y0: y0.saturating_sub(pad_y) as u32,
                x1: (last + 1 + pad_x).min(w) as u32,
                y1: (y1 + pad_y).min(h) as u32,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white page with dark bars standing in for text lines, optionally tilted and
    /// sprinkled with specks.
    fn page(lines: &[(usize, usize)], tilt: f64, specks: bool) -> OcrImage {
        let (w, h) = (600usize, 400usize);
        let mut rgb = vec![255u8; w * h * 3];
        for &(top, height) in lines {
            for x in 60..540 {
                // "letters" 6 px wide with 6 px gaps, and a word gap every 80 px
                if x % 80 < 12 || x % 12 < 6 {
                    continue;
                }
                let shift = ((x as f64 - 300.0) * tilt.to_radians().tan()).round() as i64;
                for y in top..top + height {
                    let yy = y as i64 + shift;
                    if (0..h as i64).contains(&yy) {
                        let i = (yy as usize * w + x) * 3;
                        rgb[i..i + 3].copy_from_slice(&[20, 20, 20]);
                    }
                }
            }
        }
        if specks {
            let mut state = 12345u64;
            for _ in 0..3000 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let (x, y) = ((state >> 33) as usize % w, (state >> 13) as usize % h);
                let i = (y * w + x) * 3;
                rgb[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        OcrImage::from_rgb(w as u32, h as u32, rgb).expect("valid")
    }

    #[test]
    fn finds_level_lines() {
        let layout = analyze(&page(&[(50, 20), (110, 20), (170, 20), (300, 20)], 0.0, false));
        assert_eq!(layout.skew_degrees, 0.0);
        assert_eq!(layout.lines.len(), 4);
        for (line, top) in layout.lines.iter().zip([50, 110, 170, 300]) {
            assert!(line.y0 <= top && line.y1 >= top + 20, "{line:?}");
            assert!(line.x0 <= 60 && line.x1 >= 540, "{line:?}");
        }
    }

    #[test]
    fn specks_are_not_lines() {
        let layout = analyze(&page(&[(80, 18), (160, 18), (240, 18)], 0.0, true));
        assert_eq!(layout.lines.len(), 3, "{:?}", layout.lines);
        assert!(layout.speck_fraction > 0.05);
    }

    #[test]
    fn measures_and_removes_skew() {
        let tilted = page(&[(80, 14), (150, 14), (220, 14), (290, 14)], 2.0, false);
        let layout = analyze(&tilted);
        assert!((layout.skew_degrees.abs() - 2.0).abs() <= 0.25, "{}", layout.skew_degrees);
        assert_eq!(layout.lines.len(), 4, "{:?}", layout.lines);
        let level = deskew(&tilted, layout.skew_degrees);
        assert!(analyze(&level).skew_degrees.abs() <= 0.25);
    }

    #[test]
    fn a_blank_page_has_no_lines() {
        let blank = OcrImage::from_rgb(50, 40, vec![255; 50 * 40 * 3]).expect("valid");
        let layout = analyze(&blank);
        assert!(layout.lines.is_empty());
        assert_eq!(layout.skew_degrees, 0.0);
    }
}
