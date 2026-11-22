//! Line breaking and positioning: the part of [`layout`] after itemization.

use std::ops::Range;

use icu_segmenter::options::LineBreakOptions;
use icu_segmenter::{GraphemeClusterSegmenter, LineSegmenter};
use unicode_bidi::{BidiInfo, Level, ParagraphInfo};

use super::itemize::{Item, itemize};
use super::shape::{ShapeRequest, ShapedGlyph, shape};
use super::{
    Align, BaseDirection, Cluster, FontFace, GlyphRun, Line, LineHeight, ParagraphStyle, PositionedGlyph,
    Span, TextLayout, TypesetError,
};

/// Lays out `text`, styled by `spans` (logical order, contiguous, covering the
/// whole text), with `fonts` as the font list the spans index into and that
/// fallback searches.
///
/// Paragraph separators (`\n`, `\r\n`, U+2029) start a new bidi paragraph,
/// whose direction is detected again when `style.direction` is `Auto`; U+2028
/// breaks the line inside a paragraph. Empty text gives one empty line, so an
/// editor always has a caret position and line height.
pub fn layout(
    text: &str,
    spans: &[Span],
    fonts: &[FontFace],
    style: &ParagraphStyle,
) -> Result<TextLayout, TypesetError> {
    validate(text, spans, fonts)?;
    let languages: Vec<Option<harfrust::Language>> =
        spans.iter().map(|s| s.language.as_deref().and_then(|l| l.parse().ok())).collect();
    let ctx = Ctx { text, spans, fonts, languages: &languages };

    let default_level = match style.direction {
        BaseDirection::Auto => None,
        BaseDirection::Ltr => Some(Level::ltr()),
        BaseDirection::Rtl => Some(Level::rtl()),
    };
    let bidi = BidiInfo::new(text, default_level);

    let mut drafts: Vec<Draft> = Vec::new();
    if text.is_empty() {
        drafts.push(ctx.empty_line(0, style.direction == BaseDirection::Rtl));
    }
    for para in &bidi.paragraphs {
        let items = itemize(text, para.range.clone(), &bidi.levels, spans, fonts);
        let advances = ctx.measure(&items, &para.range)?;
        let ranges = break_lines(text, para.range.clone(), &advances, style.width);
        let count = ranges.len();
        for (i, (range, hard_end)) in ranges.into_iter().enumerate() {
            let mut draft = ctx.position_line(&bidi, para, &items, range)?;
            // Justification skips a paragraph's last line and lines ended by a
            // forced break, as word processors do.
            draft.stretchable = !hard_end && i + 1 < count;
            drafts.push(draft);
        }
    }
    // Text ending in a line break has an empty last line, where an editor's
    // caret goes; it keeps the previous paragraph's direction.
    if text.chars().next_back().is_some_and(is_hard_break) {
        let rtl = drafts.last().is_some_and(|d| d.line.rtl);
        drafts.push(ctx.empty_line(text.len(), rtl));
    }

    let width = match style.width {
        Some(w) if w.is_finite() => w.max(0.0),
        _ => drafts.iter().map(|d| d.content_width()).fold(0.0, f32::max),
    };
    let mut top = 0.0;
    let mut lines = Vec::with_capacity(drafts.len());
    for draft in drafts {
        let gap = draft.gap;
        let mut line = draft.finish(style.align, width, text);
        let normal = line.ascent + line.descent + gap;
        let height = match style.line_height {
            LineHeight::Normal => normal,
            LineHeight::Multiple(m) if m.is_finite() && m > 0.0 => normal * m,
            LineHeight::Points(p) if p.is_finite() && p > 0.0 => p,
            _ => normal,
        };
        // Half the leading above the text, half below (as CSS does).
        line.top = top;
        line.baseline = top + (height - (line.ascent + line.descent)) / 2.0 + line.ascent;
        line.height = height;
        top += height;
        lines.push(line);
    }
    Ok(TextLayout { lines, width, height: top })
}

fn validate(text: &str, spans: &[Span], fonts: &[FontFace]) -> Result<(), TypesetError> {
    let err = |m: String| Err(TypesetError::Spans(m));
    if spans.is_empty() {
        return err("no spans".into());
    }
    let mut expected = 0;
    for (i, span) in spans.iter().enumerate() {
        if span.range.start != expected || span.range.end < span.range.start {
            return err(format!("span {i} ({:?}) does not continue at byte {expected}", span.range));
        }
        if !text.is_char_boundary(span.range.start) || !text.is_char_boundary(span.range.end) {
            return err(format!("span {i} ({:?}) splits a character", span.range));
        }
        if span.font >= fonts.len() {
            return err(format!("span {i} uses font {} of {}", span.font, fonts.len()));
        }
        if !(span.size.is_finite() && span.size > 0.0) {
            return err(format!("span {i} has size {}", span.size));
        }
        expected = span.range.end;
    }
    if expected != text.len() {
        return err(format!("spans end at byte {expected}, text at {}", text.len()));
    }
    Ok(())
}

/// Characters that end a line by themselves (UAX #14 classes BK, CR, LF, NL).
/// They are never shaped or drawn.
pub(super) fn is_hard_break(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Whitespace that hangs past the end of a line instead of counting toward
/// its width. No-break spaces do not hang: they glue words together.
fn hangs(ch: char) -> bool {
    ch.is_whitespace() && !matches!(ch, '\u{00A0}' | '\u{2007}' | '\u{202F}')
}

/// Spaces that justification may widen.
fn stretches(ch: char) -> bool {
    matches!(ch, ' ' | '\u{00A0}' | '\u{3000}' | '\u{2000}'..='\u{200A}')
}

struct Ctx<'a> {
    text: &'a str,
    spans: &'a [Span],
    fonts: &'a [FontFace],
    languages: &'a [Option<harfrust::Language>],
}

impl Ctx<'_> {
    fn scale(&self, item_span: usize, font: usize) -> f32 {
        self.spans[item_span].size / f32::from(self.fonts[font].units_per_em())
    }

    fn shape_piece(
        &self,
        item: &Item,
        range: Range<usize>,
        context: &Range<usize>,
        rtl: bool,
    ) -> Result<Vec<ShapedGlyph>, TypesetError> {
        shape(&ShapeRequest {
            text: self.text,
            range,
            context: context.clone(),
            rtl,
            script: item.script,
            language: self.languages[item.span].as_ref(),
            font: &self.fonts[item.font],
        })
    }

    /// Advance (points) per byte of the paragraph, each glyph counted at the
    /// byte where its cluster starts. Items are shaped whole, in their own
    /// direction: good enough to choose breaks; lines are reshaped exactly.
    fn measure(&self, items: &[Item], para: &Range<usize>) -> Result<Vec<f32>, TypesetError> {
        let mut advances = vec![0.0f32; para.len()];
        for item in items {
            let scale = self.scale(item.span, item.font);
            for glyph in self.shape_piece(item, item.range.clone(), para, item.level.is_rtl())? {
                advances[glyph.cluster - para.start] += glyph.x_advance as f32 * scale;
            }
        }
        Ok(advances)
    }

    fn empty_line(&self, at: usize, rtl: bool) -> Draft {
        let span = self.spans.iter().position(|s| s.range.contains(&at)).unwrap_or(self.spans.len() - 1);
        let (font, size) = (self.spans[span].font, self.spans[span].size);
        let (ascent, descent) = self.metrics(font, size);
        let face = &self.fonts[font];
        Draft {
            line: Line {
                text_range: at..at,
                rtl,
                top: 0.0,
                baseline: 0.0,
                height: 0.0,
                ascent,
                descent,
                x: 0.0,
                width: 0.0,
                runs: Vec::new(),
            },
            total: 0.0,
            trailing: 0.0,
            stretchable: false,
            gap: face.line_metrics().gap * size / f32::from(face.units_per_em()),
        }
    }

    /// Ascent and descent in points.
    fn metrics(&self, font: usize, size: f32) -> (f32, f32) {
        let face = &self.fonts[font];
        let scale = size / f32::from(face.units_per_em());
        let m = face.line_metrics();
        (m.ascent * scale, m.descent * scale)
    }

    /// Builds one line: its bidi runs in visual order (rule L1 applied by
    /// unicode-bidi for this line), each run's items reshaped for exactly the
    /// part on this line, glyphs placed from x = 0.
    fn position_line(
        &self,
        bidi: &BidiInfo<'_>,
        para: &ParagraphInfo,
        items: &[Item],
        range: Range<usize>,
    ) -> Result<Draft, TypesetError> {
        let rtl = para.level.is_rtl();
        // Drop the break character itself; nothing of it is drawn.
        let mut content_end = range.end;
        while content_end > range.start
            && self.text[..content_end].chars().next_back().is_some_and(is_hard_break)
        {
            content_end -= self.text[..content_end].chars().next_back().map_or(1, char::len_utf8);
        }
        if content_end == range.start {
            let mut draft = self.empty_line(range.start, rtl);
            draft.line.text_range = range;
            return Ok(draft);
        }

        // Where the hanging whitespace starts, logically.
        let mut trailing_start = content_end;
        while trailing_start > range.start
            && self.text[..trailing_start].chars().next_back().is_some_and(hangs)
        {
            trailing_start -= self.text[..trailing_start].chars().next_back().map_or(1, char::len_utf8);
        }

        let (levels, runs) = bidi.visual_runs(para, range.start..content_end);
        let mut out_runs: Vec<GlyphRun> = Vec::new();
        let mut pen = 0.0f32;
        let mut trailing = 0.0f32;
        let (mut ascent, mut descent) = (0.0f32, 0.0f32);
        let mut gap = 0.0f32;
        let mut best_line = 0.0f32;
        for run in runs {
            let run_rtl = levels[run.start].is_rtl();
            let mut pieces: Vec<(&Item, Range<usize>)> = items
                .iter()
                .filter(|item| item.range.start < run.end && run.start < item.range.end)
                .map(|item| (item, item.range.start.max(run.start)..item.range.end.min(run.end)))
                .collect();
            if run_rtl {
                pieces.reverse();
            }
            for (item, piece) in pieces {
                let scale = self.scale(item.span, item.font);
                let size = self.spans[item.span].size;
                let shaped = self.shape_piece(item, piece.clone(), &para.range, run_rtl)?;
                let (a, d) = self.metrics(item.font, size);
                ascent = ascent.max(a);
                descent = descent.max(d);
                // The line gap of whichever font sets the tallest line box.
                let item_gap = self.fonts[item.font].line_metrics().gap * scale;
                if a + d + item_gap > best_line {
                    best_line = a + d + item_gap;
                    gap = item_gap;
                }

                let clusters = clusters_of(&shaped, &piece, run_rtl);
                let mut glyphs = Vec::with_capacity(shaped.len());
                for (index, cluster) in clusters.iter().enumerate() {
                    for g in &shaped[cluster.glyphs.clone()] {
                        let advance = g.x_advance as f32 * scale;
                        glyphs.push(PositionedGlyph {
                            id: g.id,
                            x: pen + g.x_offset as f32 * scale,
                            y_offset: g.y_offset as f32 * scale,
                            advance,
                            cluster: index,
                        });
                        if cluster.text.start >= trailing_start {
                            trailing += advance;
                        }
                        pen += advance;
                    }
                }
                out_runs.push(GlyphRun {
                    font: item.font,
                    size,
                    rtl: run_rtl,
                    script: item.script.tag().to_be_bytes(),
                    text_range: piece,
                    glyphs,
                    clusters,
                });
            }
        }
        Ok(Draft {
            line: Line {
                text_range: range,
                rtl,
                top: 0.0,
                baseline: 0.0,
                height: 0.0,
                ascent,
                descent,
                x: 0.0,
                width: pen - trailing,
                runs: out_runs,
            },
            total: pen,
            trailing,
            stretchable: false,
            gap,
        })
    }
}

/// Groups a piece's glyphs (visual order) into clusters and gives each the
/// text it stands for: from its cluster offset to the next cluster's offset in
/// logical order — which is the next group for LTR and the previous for RTL.
fn clusters_of(glyphs: &[ShapedGlyph], piece: &Range<usize>, rtl: bool) -> Vec<Cluster> {
    let mut groups: Vec<(usize, Range<usize>)> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        match groups.last_mut() {
            Some((start, range)) if *start == g.cluster => range.end = i + 1,
            _ => groups.push((g.cluster, i..i + 1)),
        }
    }
    let n = groups.len();
    (0..n)
        .map(|k| {
            let start = groups[k].0;
            let end = if rtl {
                if k == 0 { piece.end } else { groups[k - 1].0 }
            } else if k + 1 < n {
                groups[k + 1].0
            } else {
                piece.end
            };
            Cluster { text: start..end.max(start), glyphs: groups[k].1.clone() }
        })
        .collect()
}

/// A positioned line before alignment: glyphs start at x = 0.
struct Draft {
    line: Line,
    /// Pen advance of everything on the line, hanging whitespace included.
    total: f32,
    /// Advance of the hanging whitespace.
    trailing: f32,
    stretchable: bool,
    /// Line gap (points) that `LineHeight::Normal` adds.
    gap: f32,
}

impl Draft {
    fn content_width(&self) -> f32 {
        self.total - self.trailing
    }

    fn finish(mut self, align: Align, width: f32, text: &str) -> Line {
        let rtl = self.line.rtl;
        if align == Align::Justify && self.stretchable {
            self.justify(width, text);
        }
        let content = self.content_width();
        // Hanging whitespace sits at the paragraph's end: right for LTR, left for RTL.
        let content_left = if rtl { self.trailing } else { 0.0 };
        let left = match (align, rtl) {
            (Align::Left, _) | (Align::Start | Align::Justify, false) | (Align::End, true) => 0.0,
            (Align::Right, _) | (Align::Start | Align::Justify, true) | (Align::End, false) => {
                width - content
            }
            (Align::Center, _) => (width - content) / 2.0,
        };
        let dx = left - content_left;
        for run in &mut self.line.runs {
            for glyph in &mut run.glyphs {
                glyph.x += dx;
            }
        }
        self.line.x = left;
        self.line.width = content;
        self.line
    }

    /// Widens the line's inner spaces so its content fills `width`.
    fn justify(&mut self, width: f32, text: &str) {
        let content = self.content_width();
        let trailing_start = self.trailing_start(text);
        let is_space = |run: &GlyphRun, g: &PositionedGlyph| {
            let range = &run.clusters[g.cluster].text;
            range.start < trailing_start && !range.is_empty() && text[range.clone()].chars().all(stretches)
        };
        let count = self
            .line
            .runs
            .iter()
            .flat_map(|r| r.glyphs.iter().map(move |g| (r, g)))
            .filter(|(r, g)| is_space(r, g))
            .count();
        if count == 0 || width.partial_cmp(&content) != Some(std::cmp::Ordering::Greater) {
            return;
        }
        let extra = (width - content) / count as f32;
        let mut shift = 0.0;
        for run in &mut self.line.runs {
            let flags: Vec<bool> = run.glyphs.iter().map(|g| is_space(run, g)).collect();
            for (glyph, space) in run.glyphs.iter_mut().zip(flags) {
                glyph.x += shift;
                if space {
                    glyph.advance += extra;
                    shift += extra;
                }
            }
        }
        self.total += extra * count as f32;
    }

    fn trailing_start(&self, text: &str) -> usize {
        let range = &self.line.text_range;
        let mut at = range.end;
        while at > range.start && text[..at].chars().next_back().is_some_and(|c| hangs(c) || is_hard_break(c))
        {
            at -= text[..at].chars().next_back().map_or(1, char::len_utf8);
        }
        at
    }
}

/// Splits one paragraph into lines: (byte range, ended by a hard break).
/// Greedy: each line takes as many break opportunities as fit.
fn break_lines(
    text: &str,
    para: Range<usize>,
    advances: &[f32],
    width: Option<f32>,
) -> Vec<(Range<usize>, bool)> {
    let p = &text[para.clone()];
    let mut prefix = Vec::with_capacity(advances.len() + 1);
    prefix.push(0.0f32);
    for a in advances {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + a);
    }
    // Width of [start, end), hanging whitespace and break characters excluded.
    let fit = |start: usize, end: usize| -> f32 {
        let mut e = end;
        while e > start && p[..e].chars().next_back().is_some_and(|c| hangs(c) || is_hard_break(c)) {
            e -= p[..e].chars().next_back().map_or(1, char::len_utf8);
        }
        prefix[e] - prefix[start]
    };
    let max = width.filter(|w| w.is_finite());

    let breaks: Vec<(usize, bool)> = LineSegmenter::new_for_non_complex_scripts(LineBreakOptions::default())
        .segment_str(p)
        .filter(|&at| at > 0)
        .map(|at| (at, p[..at].chars().next_back().is_some_and(is_hard_break)))
        .collect();
    let graphemes: Vec<usize> = GraphemeClusterSegmenter::new().segment_str(p).filter(|&g| g > 0).collect();

    let mut lines = Vec::new();
    let mut start = 0;
    let mut candidate: Option<usize> = None;
    let mut i = 0;
    while i < breaks.len() {
        let (at, hard) = breaks[i];
        let fits = max.is_none_or(|m| fit(start, at) <= m + 1e-3);
        if fits {
            if hard || at == p.len() {
                lines.push((para.start + start..para.start + at, hard));
                start = at;
                candidate = None;
            } else {
                candidate = Some(at);
            }
            i += 1;
            continue;
        }
        let m = max.unwrap_or(f32::INFINITY);
        let end = match candidate.take() {
            Some(c) => c,
            // One word wider than the line: break it between grapheme clusters,
            // as many as fit, at least one.
            None => {
                let inside = graphemes.iter().copied().filter(|&g| g > start && g < at);
                let fitting = inside.clone().take_while(|&g| fit(start, g) <= m + 1e-3).last();
                match fitting.or_else(|| graphemes.iter().copied().find(|&g| g > start)) {
                    Some(g) if g < at => g,
                    _ => at,
                }
            }
        };
        let hard = end == at && hard;
        lines.push((para.start + start..para.start + end, hard));
        start = end;
        if end == at {
            i += 1;
        }
    }
    if start < p.len() || lines.is_empty() {
        lines.push((para.start + start..para.end, false));
    }
    lines
}
