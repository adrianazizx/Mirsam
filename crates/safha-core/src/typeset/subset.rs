//! Font subsets for embedding: each font of a layout cut down to the glyphs the
//! layout uses (spec §6.3: "an embedded subset of the font"), with a variable
//! font instanced to the face's variation, since PDF has no variable fonts.
//!
//! The subset is a TrueType program without a `cmap` (`subsetter` removes it),
//! meant for a CIDFontType2 with `/CIDToGIDMap /Identity`: a glyph's new id *is*
//! its CID. What each glyph stands for comes from the layout's clusters, for
//! the writer's ToUnicode map.

use std::collections::BTreeMap;

use harfrust::{GlyphId, ShaperFont};
use subsetter::{GlyphRemapper, Tag};

use super::{FontFace, TextLayout, TypesetError};

/// One font of a layout, ready to embed.
#[derive(Debug, Clone)]
pub struct FontSubset {
    /// Index of the font in the font list the layout was made with.
    pub font: usize,
    /// The subset font program (TrueType, static).
    pub data: Vec<u8>,
    /// Glyphs kept, in CID order; CID 0 is `.notdef`.
    pub glyphs: Vec<SubsetGlyph>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubsetGlyph {
    /// The glyph's id in the subset, which is also its CID.
    pub cid: u16,
    /// The glyph's id in the original font, as the layout uses it.
    pub original: u32,
    /// Advance in thousandths of an em, at the face's variation (the PDF `/W` entry).
    pub width: f32,
    /// The text the glyph stands for when it alone draws a cluster (a letter,
    /// a ligature). `None` for `.notdef`, for glyphs that only ever draw part of
    /// a cluster (a letter's separate dots, a mark pair drawn as two glyphs),
    /// and the first cluster's text wins when one glyph draws different text
    /// in different places.
    pub text: Option<String>,
}

impl FontSubset {
    /// The CID of a glyph the layout uses.
    pub fn cid(&self, original: u32) -> Option<u16> {
        self.glyphs.iter().find(|g| g.original == original).map(|g| g.cid)
    }
}

/// Subsets every font `layout` uses. Fonts it does not use get no subset.
pub fn subset_fonts(
    text: &str,
    layout: &TextLayout,
    fonts: &[FontFace],
) -> Result<Vec<FontSubset>, TypesetError> {
    // Per font: original glyph id -> text, in order of first use.
    let mut used: BTreeMap<usize, Vec<(u32, Option<String>)>> = BTreeMap::new();
    for run in layout.runs() {
        let glyphs = used.entry(run.font).or_default();
        for cluster in &run.clusters {
            let single = cluster.glyphs.len() == 1;
            for glyph in &run.glyphs[cluster.glyphs.clone()] {
                // The letters, though the font drew presentation forms (ADR 0026).
                let meaning = single.then(|| super::logical_text(&text[cluster.text.clone()]));
                match glyphs.iter_mut().find(|(id, _)| *id == glyph.id) {
                    Some((_, existing)) => {
                        if existing.is_none() {
                            *existing = meaning;
                        }
                    }
                    None => glyphs.push((glyph.id, meaning)),
                }
            }
        }
    }

    used.into_iter()
        .map(|(font, glyphs)| {
            let face = &fonts[font];
            let mut remapper = GlyphRemapper::new();
            for (id, _) in &glyphs {
                let id = u16::try_from(*id)
                    .map_err(|_| TypesetError::Subset(format!("glyph id {id} out of range")))?;
                remapper.remap(id);
            }
            let variations: Vec<(Tag, f32)> =
                face.variations().map(|(tag, value)| (Tag::new(&tag), value)).collect();
            // Instances a variable font at the face's variation (its default
            // instance when there is none); a static font is subset as it is.
            let data = subsetter::subset_with_variations(face.data(), face.index(), &variations, &remapper)
                .map_err(|e| TypesetError::Subset(format!("{e:?}")))?;

            let shaper = ShaperFont::new(face.shaper_font());
            let upem = f32::from(face.units_per_em());
            let meaning: BTreeMap<u32, Option<String>> = glyphs.into_iter().collect();
            let kept = remapper
                .remapped_gids()
                .enumerate()
                .map(|(cid, original)| {
                    let original = u32::from(original);
                    SubsetGlyph {
                        cid: u16::try_from(cid).unwrap_or(u16::MAX),
                        original,
                        width: shaper.glyph_h_advance(GlyphId::new(original)) as f32 * 1000.0 / upem,
                        text: meaning.get(&original).cloned().flatten(),
                    }
                })
                .collect();
            Ok(FontSubset { font, data, glyphs: kept })
        })
        .collect()
}
