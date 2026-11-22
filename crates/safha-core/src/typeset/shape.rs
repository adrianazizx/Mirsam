//! One harfrust call: a run of text in one font, script and direction, with
//! the text around it as context (so Arabic letters join across run edges).

use std::ops::Range;

use harfrust::{Buffer, BufferFlags, ClusterLevel, Direction, ShapeOptions, ShaperFont};

use super::{FontFace, TypesetError};

/// A shaped glyph in font units. Glyphs come out in visual order (left to
/// right), whatever the direction.
#[derive(Debug, Clone, Copy)]
pub(super) struct ShapedGlyph {
    pub id: u32,
    /// Byte offset in the full text where this glyph's cluster starts.
    pub cluster: usize,
    pub x_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

pub(super) struct ShapeRequest<'a> {
    pub text: &'a str,
    /// The run to shape, as a byte range of `text`.
    pub range: Range<usize>,
    /// Context limits: shaping looks at most this far for joining context
    /// (the bidi paragraph).
    pub context: Range<usize>,
    pub rtl: bool,
    pub script: harfrust::Script,
    pub language: Option<&'a harfrust::Language>,
    pub font: &'a FontFace,
}

pub(super) fn shape(request: &ShapeRequest<'_>) -> Result<Vec<ShapedGlyph>, TypesetError> {
    let ShapeRequest { text, range, context, rtl, script, language, font } = request;
    let mut buffer = Buffer::new();
    buffer.push_str(&text[range.clone()]);
    buffer.set_pre_context(&text[context.start..range.start]);
    buffer.set_post_context(&text[range.end..context.end]);
    buffer.set_direction(if *rtl { Direction::RightToLeft } else { Direction::LeftToRight });
    buffer.set_script(Some(*script));
    buffer.set_language(language.cloned());
    // One cluster per character where the font allows it: the PDF writer maps
    // glyphs back to text (ToUnicode), and finer clusters map more precisely.
    buffer.set_cluster_level(ClusterLevel::MonotoneCharacters);
    let mut flags = BufferFlags::empty();
    if range.start == context.start {
        flags |= BufferFlags::BEGINNING_OF_TEXT;
    }
    if range.end == context.end {
        flags |= BufferFlags::END_OF_TEXT;
    }
    buffer.set_flags(flags);

    let shaper = ShaperFont::new(font.shaper_font());
    harfrust::shape(&shaper, &mut buffer, ShapeOptions::new())
        .map_err(|e| TypesetError::Shape(format!("{e:?}")))?;

    Ok(buffer
        .glyph_infos()
        .iter()
        .zip(buffer.glyph_positions())
        .map(|(info, pos)| ShapedGlyph {
            id: info.glyph_id,
            cluster: range.start + info.cluster as usize,
            x_advance: pos.x_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
        })
        .collect())
}
