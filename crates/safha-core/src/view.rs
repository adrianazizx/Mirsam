//! From PDF page space to the page as it is displayed.
//!
//! Text geometry ([`crate::text`]) is in PDF page space: points, y up, relative
//! to the page's own origin (which need not be its corner), before the page's
//! `/Rotate`. A viewer draws the page rotated, with y down from its top-left
//! corner. [`ViewTransform`] maps one to the other exactly as PDFium does when
//! it renders, so a highlight lands on the glyph it belongs to on rotated pages
//! and on pages whose crop box does not start at the origin.

use pdfium_render::prelude::{PdfPage, PdfPoints, PdfRenderConfig};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::text::Rect;

/// A rectangle on the displayed page: points from its top-left corner, y down.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// An affine map `view = (a·x + c·y + e, b·x + d·y + f)` from page space to view space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

/// Device pixels per point used when sampling PDFium's page-to-device mapping,
/// which returns whole pixels: the transform is exact to 1/64 pt.
const SAMPLE_SCALE: f32 = 64.0;
/// Distance between the sampled points, in points.
const SAMPLE_SPAN: f32 = 1000.0;

impl ViewTransform {
    /// For an unrotated page whose lower-left corner is the origin and which is
    /// `height` points tall.
    pub fn unrotated(height: f32) -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: -1.0, e: 0.0, f: height }
    }

    /// Reads the mapping PDFium renders `page` with: three points through
    /// `FPDF_PageToDevice` determine the affine map.
    pub(crate) fn of_page(page: &PdfPage<'_>) -> Result<Self> {
        let width = page.width().value;
        let height = page.height().value;
        let device = |side: f32| -> Result<i32> {
            let pixels = (side * SAMPLE_SCALE).round();
            if !(1.0..=1.0e8).contains(&pixels) {
                return Err(Error::Pdfium("page size is out of range".into()));
            }
            Ok(pixels as i32)
        };
        let config = PdfRenderConfig::new().set_fixed_size(device(width)?, device(height)?);
        let sample = |x: f32, y: f32| -> Result<(f32, f32)> {
            let (dx, dy) = page.points_to_pixels(PdfPoints::new(x), PdfPoints::new(y), &config)?;
            Ok((dx as f32 / SAMPLE_SCALE, dy as f32 / SAMPLE_SCALE))
        };
        let (e, f) = sample(0.0, 0.0)?;
        let (x1, y1) = sample(SAMPLE_SPAN, 0.0)?;
        let (x2, y2) = sample(0.0, SAMPLE_SPAN)?;
        let transform = Self {
            a: (x1 - e) / SAMPLE_SPAN,
            b: (y1 - f) / SAMPLE_SPAN,
            c: (x2 - e) / SAMPLE_SPAN,
            d: (y2 - f) / SAMPLE_SPAN,
            e,
            f,
        };
        Ok(transform.snapped())
    }

    /// Page rotations are multiples of 90°, so the linear part is a signed
    /// permutation; rounding removes the sampling error from it.
    fn snapped(self) -> Self {
        let snap = |v: f32| v.round();
        Self { a: snap(self.a), b: snap(self.b), c: snap(self.c), d: snap(self.d), ..self }
    }

    /// Page space is displayed as it is, only moved and flipped upright: the
    /// page is not turned (`/Rotate` 0).
    pub fn upright(&self) -> bool {
        self.b == 0.0 && self.c == 0.0 && self.a > 0.0 && self.d < 0.0
    }

    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// The page-space point displayed at (`x`, `y`): the inverse map.
    pub fn page_point(&self, x: f32, y: f32) -> (f32, f32) {
        // The linear part is a signed permutation (determinant ±1), so this
        // is exact; a degenerate map yields non-finite points, which callers
        // reject.
        let det = self.a * self.d - self.b * self.c;
        let (dx, dy) = (x - self.e, y - self.f);
        ((self.d * dx - self.c * dy) / det, (self.a * dy - self.b * dx) / det)
    }

    /// The displayed bounding box of a page-space rectangle.
    pub fn rect(&self, rect: Rect) -> ViewRect {
        let corners = [
            self.point(rect.left, rect.bottom),
            self.point(rect.right, rect.bottom),
            self.point(rect.left, rect.top),
            self.point(rect.right, rect.top),
        ];
        let (mut x0, mut y0, mut x1, mut y1) =
            (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for (x, y) in corners {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        ViewRect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
    }
}

#[cfg(test)]
impl ViewTransform {
    /// A page turned a quarter: page (x, y) is displayed at (y, x).
    pub(crate) fn of_rotation_for_tests() -> Self {
        Self { a: 0.0, b: 1.0, c: 1.0, d: 0.0, e: 0.0, f: 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unrotated_page_flips_y() {
        let transform = ViewTransform::unrotated(800.0);
        let rect = transform.rect(Rect { left: 10.0, bottom: 700.0, right: 30.0, top: 712.0 });
        assert_eq!(rect, ViewRect { x: 10.0, y: 88.0, width: 20.0, height: 12.0 });
    }

    #[test]
    fn page_points_invert_the_map() {
        let turned = ViewTransform { a: 0.0, b: 1.0, c: -1.0, d: 0.0, e: 800.0, f: -40.0 };
        for transform in [ViewTransform::unrotated(800.0), turned] {
            let (vx, vy) = transform.point(123.0, 456.0);
            assert_eq!(transform.page_point(vx, vy), (123.0, 456.0));
        }
    }

    #[test]
    fn a_quarter_turn_swaps_axes() {
        // /Rotate 90 on a 600 x 800 page: the page's bottom-left corner is
        // displayed at the top-left, and its left edge runs along the top.
        let transform = ViewTransform { a: 0.0, b: 1.0, c: 1.0, d: 0.0, e: 0.0, f: 0.0 };
        let rect = transform.rect(Rect { left: 10.0, bottom: 20.0, right: 30.0, top: 25.0 });
        assert_eq!(rect, ViewRect { x: 20.0, y: 10.0, width: 5.0, height: 20.0 });
    }
}
