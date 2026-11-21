//! Logical string -> glyphs in visual (left-to-right) order: unicode-bidi for the
//! run order, harfrust for shaping each run.
//!
//! Spike shortcuts a real text engine may not take: one font per line (no
//! fallback), runs are split by bidi level only (no script itemisation), a single
//! line (no line breaking), and no justification / kashida.

use std::ops::Range;

use harfrust::{Buffer, ClusterLevel, Direction, ShapeOptions, ShaperFont};
use unicode_bidi::BidiInfo;

use crate::fontinfo::FontInfo;

#[derive(Debug, Clone)]
pub struct Glyph {
    pub gid: u16,
    /// Font units. The pen moves by `x_advance` after the glyph; the glyph itself
    /// is drawn at pen + (`x_offset`, `y_offset`).
    pub x_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    /// Index into `ShapedLine::clusters`.
    pub cluster: usize,
}

/// A minimal glyphs<->characters unit as reported by the shaper.
#[derive(Debug, Clone)]
pub struct Cluster {
    /// Byte range of the source characters in `ShapedLine::text`.
    pub text: Range<usize>,
    /// Glyph range in `ShapedLine::glyphs` (visual order).
    pub glyphs: Range<usize>,
    pub rtl: bool,
    /// Index of the bidi run (visual order) this cluster belongs to.
    pub run: usize,
}

#[derive(Debug, Clone)]
pub struct ShapedLine {
    pub text: String,
    pub base_rtl: bool,
    /// Visual order, left to right.
    pub glyphs: Vec<Glyph>,
    /// Visual order, left to right.
    pub clusters: Vec<Cluster>,
    /// Sum of advances, font units.
    pub width: i32,
    pub run_count: usize,
}

impl ShapedLine {
    pub fn cluster_text(&self, c: &Cluster) -> &str {
        &self.text[c.text.clone()]
    }

    /// Glyph indices of a cluster in logical order (reversed for RTL clusters).
    pub fn logical_glyphs(&self, c: &Cluster) -> Vec<usize> {
        if c.rtl { c.glyphs.clone().rev().collect() } else { c.glyphs.clone().collect() }
    }

    /// Pen x (font units) at which each glyph's advance starts.
    pub fn pen_positions(&self) -> Vec<i32> {
        let mut x = 0;
        self.glyphs
            .iter()
            .map(|g| {
                let at = x;
                x += g.x_advance;
                at
            })
            .collect()
    }
}

pub fn shape_line(font: &FontInfo, text: &str) -> Result<ShapedLine, Box<dyn std::error::Error>> {
    let bidi = BidiInfo::new(text, None);
    let para = bidi.paragraphs.first().ok_or("empty text")?;
    let base_rtl = para.level.is_rtl();
    let (levels, runs) = bidi.visual_runs(para, para.range.clone());

    let shaper = ShaperFont::new(&font.shaper);
    let mut glyphs = Vec::new();
    let mut clusters: Vec<Cluster> = Vec::new();

    for (run_idx, run) in runs.iter().enumerate() {
        let rtl = levels[run.start].is_rtl();
        let mut buf = Buffer::new();
        buf.push_str(&text[run.clone()]);
        buf.set_pre_context(&text[..run.start]);
        buf.set_post_context(&text[run.end..]);
        buf.set_direction(if rtl { Direction::RightToLeft } else { Direction::LeftToRight });
        buf.guess_segment_properties();
        // Keep marks in their own clusters where the font allows it: the PDF needs a
        // glyph -> Unicode mapping that is as fine-grained as possible.
        buf.set_cluster_level(ClusterLevel::MonotoneCharacters);
        harfrust::shape(&shaper, &mut buf, ShapeOptions::new()).map_err(|e| format!("shape: {e:?}"))?;

        let infos = buf.glyph_infos();
        let positions = buf.glyph_positions();
        // Output is in visual order: cluster values ascend for LTR, descend for RTL.
        let mut i = 0;
        while i < infos.len() {
            let mut j = i + 1;
            while j < infos.len() && infos[j].cluster == infos[i].cluster {
                j += 1;
            }
            let start = run.start + infos[i].cluster as usize;
            // The cluster's text ends where the logically-next cluster starts.
            let end = if rtl {
                if i == 0 { run.end } else { run.start + infos[i - 1].cluster as usize }
            } else if j < infos.len() {
                run.start + infos[j].cluster as usize
            } else {
                run.end
            };
            let first_glyph = glyphs.len();
            for k in i..j {
                glyphs.push(Glyph {
                    gid: u16::try_from(infos[k].glyph_id)?,
                    x_advance: positions[k].x_advance,
                    x_offset: positions[k].x_offset,
                    y_offset: positions[k].y_offset,
                    cluster: clusters.len(),
                });
            }
            clusters.push(Cluster { text: start..end, glyphs: first_glyph..glyphs.len(), rtl, run: run_idx });
            i = j;
        }
    }

    let width = glyphs.iter().map(|g| g.x_advance).sum();
    Ok(ShapedLine { text: text.to_string(), base_rtl, glyphs, clusters, width, run_count: runs.len() })
}
