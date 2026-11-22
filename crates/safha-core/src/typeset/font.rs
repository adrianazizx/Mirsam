//! A font the layout engine can shape with: the parsed face (at a variation
//! instance, if any) plus the few metrics layout needs.

use std::sync::Arc;

use harfrust::Tag;

use super::TypesetError;

/// One font face, ready to shape with.
///
/// Cheap to clone: the font data is shared. A variable font is shaped and
/// measured at the instance chosen with [`FontFace::with_variations`]; the
/// default instance otherwise.
#[derive(Clone)]
pub struct FontFace {
    data: Arc<Vec<u8>>,
    index: u32,
    font: harfrust::Font,
    variations: Vec<(Tag, f32)>,
    units_per_em: u16,
    line: LineMetrics,
}

/// Vertical metrics in font units: `ascent` above the baseline and `descent`
/// below it, both positive, and the font's recommended extra `gap` between lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub gap: f32,
}

impl FontFace {
    /// Parses face `index` of a font file (0 for a single-face file).
    pub fn from_bytes(data: Vec<u8>, index: u32) -> Result<Self, TypesetError> {
        let data = Arc::new(data);
        let font = harfrust::Font::new(Arc::clone(&data) as Arc<dyn AsRef<[u8]> + Send + Sync>, index)
            .ok_or(TypesetError::FontParse)?;
        Self::from_parts(data, index, font, Vec::new())
    }

    /// The same face at a variation instance, e.g. `[(*b"wght", 700.0)]`.
    /// Axes the font does not have are ignored; values are clamped by the font.
    pub fn with_variations(&self, variations: &[([u8; 4], f32)]) -> Result<Self, TypesetError> {
        let variations: Vec<(Tag, f32)> = variations.iter().map(|(t, v)| (Tag::new(t), *v)).collect();
        let font = self.font.instance_builder().variations(variations.iter().copied()).build();
        Self::from_parts(Arc::clone(&self.data), self.index, font, variations)
    }

    fn from_parts(
        data: Arc<Vec<u8>>,
        index: u32,
        font: harfrust::Font,
        variations: Vec<(Tag, f32)>,
    ) -> Result<Self, TypesetError> {
        let metrics = font.metrics();
        let units_per_em = metrics.units_per_em;
        if units_per_em == 0 {
            return Err(TypesetError::FontParse);
        }
        // Browsers' choice: the typographic line when the font asks for it,
        // otherwise hhea, falling back to whichever exists.
        let line_box = if metrics.use_typo_metrics {
            metrics.typo_line.or(metrics.hhea_line)
        } else {
            metrics.hhea_line.or(metrics.typo_line)
        };
        let line = match line_box {
            Some(b) => LineMetrics {
                ascent: b.ascender.to_f32().max(0.0),
                descent: (-b.descender.to_f32()).max(0.0),
                gap: b.line_gap.to_f32().max(0.0),
            },
            None => LineMetrics {
                ascent: metrics.bounds.y_max.to_f32().max(0.0),
                descent: (-metrics.bounds.y_min.to_f32()).max(0.0),
                gap: 0.0,
            },
        };
        Ok(FontFace { data, index, font, variations, units_per_em, line })
    }

    /// The raw font file, for embedding and subsetting.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Face index inside the font file.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The variation instance this face is shaped at (empty: the default).
    pub fn variations(&self) -> impl Iterator<Item = ([u8; 4], f32)> + '_ {
        self.variations.iter().map(|(t, v)| (t.to_be_bytes(), *v))
    }

    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    pub fn line_metrics(&self) -> LineMetrics {
        self.line
    }

    /// Whether the font's character map has a glyph for `ch`.
    pub fn covers(&self, ch: char) -> bool {
        self.font.charmap().map_unicode(ch).is_some()
    }

    /// Advance width of `ch`'s nominal glyph in thousandths of an em, at this
    /// face's variation; `None` if the font lacks it.
    pub fn advance(&self, ch: char) -> Option<f32> {
        let shaper = harfrust::ShaperFont::new(&self.font);
        let glyph = shaper.nominal_glyph(u32::from(ch))?;
        Some(shaper.glyph_h_advance(glyph) as f32 * 1000.0 / f32::from(self.units_per_em))
    }

    pub(super) fn shaper_font(&self) -> &harfrust::Font {
        &self.font
    }

    /// Minimum and maximum of a variable font's `wght` axis.
    pub fn weight_axis(&self) -> Option<(f32, f32)> {
        let axis = self.font.axes().get_by_tag(Tag::new(b"wght"))?;
        Some((axis.min_value(), axis.max_value()))
    }

    pub(super) fn variation_list(&self) -> &[(Tag, f32)] {
        &self.variations
    }
}

impl std::fmt::Debug for FontFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontFace")
            .field("bytes", &self.data.len())
            .field("index", &self.index)
            .field("units_per_em", &self.units_per_em)
            .field("variations", &self.variations)
            .finish()
    }
}
