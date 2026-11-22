//! Milestone 3 spike (throwaway, devtools builds only; ADR 0022): what an
//! editor surface needs from the engine for one paragraph, timed.
//!
//! The UI half (`src/spikes/editorSurface.ts`) compares a `contenteditable`
//! overlay's caret with these stops, and draws these outlines to time typing
//! on our own surface.

use serde::{Deserialize, Serialize};

use safha_core::fonts::{FontFamily, faces_for};
use safha_core::typeset::{
    Align, BaseDirection, FontFace, LineHeight, ParagraphStyle, Span, caret_stops, glyph_outline, layout,
    subset_fonts, units_per_em,
};

use crate::error::{CommandError, CommandResult};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeRequest {
    text: String,
    family: FontFamily,
    size: f32,
    width: f32,
    justify: bool,
    /// Glyphs (font index, original id) the UI already holds outlines for.
    known: Vec<(usize, u32)>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeLine {
    top: f32,
    baseline: f32,
    height: f32,
    /// UTF-16 offsets.
    start: usize,
    end: usize,
    rtl: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeRun {
    line: usize,
    /// The face's PostScript name (the UI maps it to its CSS family).
    font: String,
    start: usize,
    end: usize,
    rtl: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeGlyph {
    font: usize,
    id: u32,
    /// Pen position from the box's left edge; baseline from its top, y down.
    x: f32,
    y: f32,
    size: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeCaret {
    offset: usize,
    line: usize,
    x: f32,
    split: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeOutline {
    font: usize,
    id: u32,
    units_per_em: u16,
    /// SVG path data in font units, y up.
    path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeLayout {
    height: f32,
    lines: Vec<SpikeLine>,
    runs: Vec<SpikeRun>,
    glyphs: Vec<SpikeGlyph>,
    carets: Vec<SpikeCaret>,
    outlines: Vec<SpikeOutline>,
    /// Engine time: layout, subsetting (for the outlines), outlines.
    layout_micros: u128,
    subset_micros: u128,
    outline_micros: u128,
}

#[tauri::command]
pub fn spike_editor_layout(request: SpikeRequest) -> CommandResult<SpikeLayout> {
    let text = &request.text;
    let utf16 = |byte: usize| text.get(..byte).map_or(0, |prefix| prefix.encode_utf16().count());
    let named = faces_for(request.family, text)?;
    let faces: Vec<FontFace> = named.iter().map(|(face, _)| face.clone()).collect();
    let style = ParagraphStyle {
        direction: BaseDirection::Auto,
        align: if request.justify { Align::Justify } else { Align::Start },
        width: Some(request.width),
        line_height: LineHeight::Normal,
    };

    let started = std::time::Instant::now();
    let laid = layout(text, &[Span::new(0..text.len(), 0, request.size)], &faces, &style)
        .map_err(|err| CommandError::new("text_failed", err.code()))?;
    let layout_micros = started.elapsed().as_micros();

    let started = std::time::Instant::now();
    let subsets =
        subset_fonts(text, &laid, &faces).map_err(|err| CommandError::new("text_failed", err.code()))?;
    let subset_micros = started.elapsed().as_micros();

    let started = std::time::Instant::now();
    let mut outlines = Vec::new();
    for subset in &subsets {
        let upem = units_per_em(&subset.data).unwrap_or(1000);
        for glyph in &subset.glyphs {
            if request.known.contains(&(subset.font, glyph.original)) {
                continue;
            }
            if let Some(path) = glyph_outline(&subset.data, u32::from(glyph.cid)) {
                outlines.push(SpikeOutline {
                    font: subset.font,
                    id: glyph.original,
                    units_per_em: upem,
                    path,
                });
            }
        }
    }
    let outline_micros = started.elapsed().as_micros();

    let mut lines = Vec::new();
    let mut runs = Vec::new();
    let mut glyphs = Vec::new();
    for (index, line) in laid.lines.iter().enumerate() {
        lines.push(SpikeLine {
            top: line.top,
            baseline: line.baseline,
            height: line.height,
            start: utf16(line.text_range.start),
            end: utf16(line.text_range.end),
            rtl: line.rtl,
        });
        for run in &line.runs {
            runs.push(SpikeRun {
                line: index,
                font: named.get(run.font).map_or_else(String::new, |(_, name)| name.clone()),
                start: utf16(run.text_range.start),
                end: utf16(run.text_range.end),
                rtl: run.rtl,
            });
            for glyph in &run.glyphs {
                glyphs.push(SpikeGlyph {
                    font: run.font,
                    id: glyph.id,
                    x: glyph.x,
                    y: line.baseline - glyph.y_offset,
                    size: run.size,
                });
            }
        }
    }
    let carets = caret_stops(text, &laid)
        .into_iter()
        .map(|stop| SpikeCaret { offset: utf16(stop.offset), line: stop.line, x: stop.x, split: stop.split })
        .collect();
    Ok(SpikeLayout {
        height: laid.height,
        lines,
        runs,
        glyphs,
        carets,
        outlines,
        layout_micros,
        subset_micros,
        outline_micros,
    })
}
