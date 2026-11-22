//! Widget appearance streams (ADR 0045): a field's value drawn the way
//! the widget asks (its background, border, text size and colour,
//! alignment), with text shaped by the text engine.
//!
//! - **Text and drop-downs:** one line centred vertically, or wrapped lines
//!   from the top for a multi-line field; a comb field puts one character in
//!   each of its cells. A size of 0 in `/DA` means "fit": the text is made as
//!   large as the box allows (at most 12 points) and shrunk to fit its width.
//! - **Alignment (`/Q`):** 0 (the default) starts the text where its own
//!   direction starts, so Arabic sits at the right; 1 centres; 2 aligns right.
//! - **Lists:** the choices from the top index, the chosen ones highlighted.
//! - **Check boxes and radio buttons** without appearances of their own: a
//!   tick, cross, or dot drawn as paths, the style its `/MK /CA` names.

use crate::annotate::{Appearance, MarkKind, mark_in, num};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, for_text};
use crate::pdf::text::draw_text;
use crate::pdf::{Dict, Object};
use crate::typeset::{Align, BaseDirection, FontFace, LineHeight, ParagraphStyle, Span, TextLayout, layout};

/// The largest size a fitted ("auto", `/DA` size 0) text is set at.
const AUTO_MAX: f32 = 12.0;
const AUTO_MIN: f32 = 4.0;
/// The highlight behind a list's chosen rows (what other viewers use).
const SELECTED: [f32; 3] = [0.6, 0.75, 0.85];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonStyle {
    Check,
    Radio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Border {
    Solid,
    Dashed,
    Beveled,
    Underline,
}

/// How a widget is drawn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Look {
    rect: [f32; 4],
    /// `/MK /R`: the widget's own quarter turns.
    rotation: i64,
    /// The appearance's box (the widget's, turned by `rotation`).
    width: f32,
    height: f32,
    background: Option<[f32; 3]>,
    border: Option<[f32; 3]>,
    border_width: f32,
    border_style: Border,
    /// 0: fit the box.
    size: f32,
    color: [f32; 3],
    quadding: i64,
    /// A button's mark, from its `/MK /CA` (ZapfDingbats letters).
    mark: Option<MarkKind>,
}

/// A colour array (`/MK /BG`, `/MK /BC`): gray, RGB, or CMYK; none when empty.
fn color(object: Option<&Object>) -> Option<[f32; 3]> {
    let values: Vec<f32> = object?.as_array()?.iter().filter_map(crate::annotate::number).collect();
    let clamp = |v: f32| v.clamp(0.0, 1.0);
    match values.as_slice() {
        [gray] => Some([clamp(*gray); 3]),
        [r, g, b] => Some([clamp(*r), clamp(*g), clamp(*b)]),
        [c, m, y, k] => Some([c, m, y].map(|v| clamp((1.0 - v) * (1.0 - k)))),
        _ => None,
    }
}

/// Text size and colour from a `/DA` string; size 0 means "fit".
fn parse_da(da: &[u8]) -> (f32, [f32; 3]) {
    let text = String::from_utf8_lossy(da);
    let words: Vec<&str> = text.split_whitespace().collect();
    let number = |i: Option<usize>| i.and_then(|i| words.get(i)).and_then(|w| w.parse::<f32>().ok());
    let mut size = 0.0;
    let mut rgb = [0.0; 3];
    for (i, word) in words.iter().enumerate() {
        let back = |n: usize| number(i.checked_sub(n));
        match *word {
            "Tf" => size = back(1).filter(|s| s.is_finite() && *s >= 0.0).unwrap_or(0.0),
            "g" => {
                if let Some(gray) = back(1) {
                    rgb = [gray; 3];
                }
            }
            "rg" => {
                if let (Some(r), Some(g), Some(b)) = (back(3), back(2), back(1)) {
                    rgb = [r, g, b];
                }
            }
            "k" => {
                if let (Some(c), Some(m), Some(y), Some(k)) = (back(4), back(3), back(2), back(1)) {
                    rgb = [c, m, y].map(|v| (1.0 - v) * (1.0 - k));
                }
            }
            _ => {}
        }
    }
    (size.min(400.0), rgb.map(|c| c.clamp(0.0, 1.0)))
}

impl Look {
    pub(crate) fn new(
        rect: [f32; 4],
        da: &[u8],
        quadding: Option<i64>,
        mk: &Dict,
        bs: Option<&Dict>,
        border: Option<&Object>,
        caption: Option<&[u8]>,
    ) -> Self {
        let rotation = mk.get(b"R").and_then(Object::as_int).unwrap_or(0).rem_euclid(360) / 90 * 90;
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        let (width, height) = if rotation % 180 == 90 { (h, w) } else { (w, h) };
        let (size, color_rgb) = parse_da(da);
        // `/BS /W`, else `/Border [h v w]`, else 1.
        let border_width = bs
            .and_then(|bs| bs.get(b"W"))
            .and_then(crate::annotate::number)
            .or_else(|| {
                border.and_then(Object::as_array).and_then(|a| a.get(2)).and_then(crate::annotate::number)
            })
            .unwrap_or(1.0)
            .clamp(0.0, 12.0);
        let border_style = match bs.and_then(|bs| bs.get(b"S")).and_then(Object::as_name) {
            Some(b"D") => Border::Dashed,
            Some(b"B" | b"I") => Border::Beveled,
            Some(b"U") => Border::Underline,
            _ => Border::Solid,
        };
        let mark = caption.and_then(|ca| ca.first()).map(|letter| match letter {
            b'8' => MarkKind::Cross,
            b'l' | b'n' | b'u' => MarkKind::Dot,
            _ => MarkKind::Check,
        });
        Look {
            rect,
            rotation,
            width,
            height,
            background: color(mk.get(b"BG")),
            border: color(mk.get(b"BC")),
            border_width,
            border_style,
            size,
            color: color_rgb,
            quadding: quadding.unwrap_or(0),
            mark,
        }
    }

    /// The `/Matrix` for the widget's own turn (ISO 32000-2, 12.5.5).
    fn matrix(&self) -> Option<[f32; 6]> {
        let [left, bottom, right, top] = self.rect;
        let (w, h) = (right - left, top - bottom);
        match self.rotation {
            90 => Some([0.0, 1.0, -1.0, 0.0, w, 0.0]),
            180 => Some([-1.0, 0.0, 0.0, -1.0, w, h]),
            270 => Some([0.0, -1.0, 1.0, 0.0, 0.0, h]),
            _ => None,
        }
    }

    fn appearance(&self, content: String) -> Appearance {
        Appearance {
            rect: self.rect,
            bbox: [0.0, 0.0, self.width, self.height],
            matrix: self.matrix(),
            resources: Dict::default(),
            content: content.into_bytes(),
            fonts: Vec::new(),
            images: Vec::new(),
        }
    }

    fn drawn_border(&self) -> f32 {
        if self.border.is_some() { self.border_width } else { 0.0 }
    }

    /// Space between the box's edge and its text.
    fn padding(&self) -> f32 {
        let border = self.drawn_border();
        2.0 + if self.border_style == Border::Beveled { 2.0 * border } else { border }
    }

    /// Background and border, as a rectangle or (`round`) a circle.
    fn frame(&self, round: bool) -> String {
        let (w, h) = (self.width, self.height);
        let mut out = String::from("q\n");
        let rgb = |[r, g, b]: [f32; 3]| format!("{} {} {}", num(r), num(g), num(b));
        if let Some(background) = self.background {
            let path = if round {
                circle(w / 2.0, h / 2.0, w.min(h) / 2.0)
            } else {
                format!("0 0 {} {} re", num(w), num(h))
            };
            out.push_str(&format!("{} rg\n{path} f\n", rgb(background)));
        }
        let bw = self.drawn_border();
        if let (Some(border), true) = (self.border, bw > 0.0) {
            out.push_str(&format!("{} RG {} w\n", rgb(border), num(bw)));
            if self.border_style == Border::Dashed {
                out.push_str("[3] 0 d\n");
            }
            let half = bw / 2.0;
            if round {
                out.push_str(&format!("{} S\n", circle(w / 2.0, h / 2.0, w.min(h) / 2.0 - half)));
            } else if self.border_style == Border::Underline {
                out.push_str(&format!("0 {} m {} {} l S\n", num(half), num(w), num(half)));
            } else {
                out.push_str(&format!("{} {} {} {} re S\n", num(half), num(half), num(w - bw), num(h - bw)));
            }
        }
        out.push_str("Q\n");
        out
    }

    /// The text clipped inside the border, marked as the field's variable
    /// text (`/Tx BMC`) as other writers do.
    fn with_text(&self, frame: String, text: &str) -> String {
        let bw = self.drawn_border();
        format!(
            "{frame}/Tx BMC\nq\n{} {} {} {} re W n\n{text}Q\nEMC\n",
            num(bw),
            num(bw),
            num((self.width - 2.0 * bw).max(0.0)),
            num((self.height - 2.0 * bw).max(0.0))
        )
    }

    /// A text field's or drop-down's appearance showing `text`. `comb`: the
    /// number of cells, one character each.
    pub(crate) fn text(
        &self,
        text: &str,
        multiline: bool,
        comb: Option<usize>,
        password: bool,
    ) -> Result<Appearance> {
        let frame = self.frame(false);
        if text.is_empty() {
            return Ok(self.appearance(self.with_text(frame, "")));
        }
        let shown: String = if password { "•".repeat(text.chars().count()) } else { text.to_string() };
        let fonts = for_text(FontFamily::Sans, &shown)?;
        let faces: Vec<FontFace> = fonts.iter().map(|f| f.face.clone()).collect();
        let pad = self.padding();
        let inner_w = (self.width - 2.0 * pad).max(1.0);
        let inner_h = (self.height - 2.0 * pad).max(1.0);
        let lay = |size: f32, width: Option<f32>, align: Align| -> Result<TextLayout> {
            let style = ParagraphStyle {
                direction: BaseDirection::Auto,
                align,
                width,
                line_height: LineHeight::Normal,
            };
            layout(&shown, &[Span::new(0..shown.len(), 0, size)], &faces, &style)
                .map_err(|err| Error::Typeset(err.code()))
        };

        let (laid, left, top) = if multiline {
            let align = match self.quadding {
                1 => Align::Center,
                2 => Align::Right,
                _ => Align::Start,
            };
            let size = if self.size > 0.0 { self.size } else { AUTO_MAX };
            (lay(size, Some(inner_w), align)?, pad, self.height - pad)
        } else {
            let cells = comb.filter(|n| *n > 0);
            let size = if self.size > 0.0 {
                self.size
            } else {
                // Fit: as tall as the box allows, then as narrow as it needs.
                let probe = lay(AUTO_MAX, None, Align::Start)?;
                let mut size = AUTO_MAX * (inner_h / probe.height.max(0.01)).min(1.0);
                let room = match cells {
                    Some(n) => {
                        self.width / n as f32
                            * probe
                                .lines
                                .first()
                                .map_or(1, |l| l.runs.iter().map(|r| r.clusters.len()).sum::<usize>())
                                as f32
                    }
                    None => inner_w,
                };
                let wide = probe.width * size / AUTO_MAX;
                if wide > room {
                    size *= room / wide;
                }
                size.max(AUTO_MIN)
            };
            let mut laid = lay(size, None, Align::Start)?;
            let top = self.height - pad - ((inner_h - laid.height) / 2.0).max(0.0);
            match cells {
                Some(n) => {
                    spread_into_cells(&mut laid, n, self.width);
                    (laid, 0.0, top)
                }
                None => {
                    let rtl = laid.lines.first().is_some_and(|line| line.rtl);
                    let wide = laid.width;
                    let offset = match self.quadding {
                        1 => (inner_w - wide) / 2.0,
                        2 => inner_w - wide,
                        _ if rtl => inner_w - wide,
                        _ => 0.0,
                    };
                    // Too wide: the start of the text shows.
                    let offset = if wide > inner_w { if rtl { inner_w - wide } else { 0.0 } } else { offset };
                    (laid, pad + offset, top)
                }
            }
        };
        let drawing = draw_text(&shown, &laid, &fonts, (left, top), self.color, "F")?;
        let content = String::from_utf8_lossy(&drawing.content).into_owned();
        let mut appearance = self.appearance(self.with_text(frame, &content));
        appearance.fonts = drawing.fonts;
        Ok(appearance)
    }

    /// A list box's appearance: `labels` from `top`, the `selected` ones highlighted.
    pub(crate) fn list(&self, labels: &[&str], selected: &[usize], top: usize) -> Result<Appearance> {
        let frame = self.frame(false);
        let top = top.min(labels.len().saturating_sub(1));
        let shown = &labels[top.min(labels.len())..];
        if shown.is_empty() {
            return Ok(self.appearance(self.with_text(frame, "")));
        }
        // One paragraph per choice; choices are one line each (breaks become spaces).
        let lines: Vec<String> = shown.iter().map(|label| label.replace(['\r', '\n'], " ")).collect();
        let text = lines.join("\n");
        let mut starts = Vec::with_capacity(lines.len());
        let mut at = 0;
        for line in &lines {
            starts.push(at);
            at += line.len() + 1;
        }
        let fonts = for_text(FontFamily::Sans, &text)?;
        let faces: Vec<FontFace> = fonts.iter().map(|f| f.face.clone()).collect();
        let pad = self.padding();
        let inner_w = (self.width - 2.0 * pad).max(1.0);
        let size = if self.size > 0.0 { self.size } else { AUTO_MAX };
        let style = ParagraphStyle {
            direction: BaseDirection::Auto,
            align: Align::Start,
            width: Some(inner_w),
            line_height: LineHeight::Normal,
        };
        let laid = layout(&text, &[Span::new(0..text.len(), 0, size)], &faces, &style)
            .map_err(|err| Error::Typeset(err.code()))?;
        let text_top = self.height - pad;
        let mut highlight = String::new();
        let bw = self.drawn_border();
        for line in &laid.lines {
            let choice = starts.iter().rposition(|start| *start <= line.text_range.start).unwrap_or(0) + top;
            if selected.contains(&choice) {
                let [r, g, b] = SELECTED;
                highlight.push_str(&format!(
                    "{} {} {} rg {} {} {} {} re f\n",
                    num(r),
                    num(g),
                    num(b),
                    num(bw),
                    num(text_top - line.top - line.height),
                    num(self.width - 2.0 * bw),
                    num(line.height)
                ));
            }
        }
        let drawing = draw_text(&text, &laid, &fonts, (pad, text_top), self.color, "F")?;
        let content = format!("{highlight}{}", String::from_utf8_lossy(&drawing.content));
        let mut appearance = self.appearance(self.with_text(frame, &content));
        appearance.fonts = drawing.fonts;
        Ok(appearance)
    }

    /// A check box's or radio button's "on" and "off" appearances.
    pub(crate) fn button(&self, style: ButtonStyle) -> (Appearance, Appearance) {
        let round = style == ButtonStyle::Radio;
        let frame = self.frame(round);
        let mark = self.mark.unwrap_or(if round { MarkKind::Dot } else { MarkKind::Check });
        let scale = if mark == MarkKind::Dot { 1.0 } else { 0.8 };
        let on = format!("{frame}{}", mark_in(mark, self.color, (self.width, self.height), scale));
        (self.appearance(on), self.appearance(frame))
    }
}

/// A circle as four Bézier quarters.
fn circle(cx: f32, cy: f32, r: f32) -> String {
    let k = r * 0.5523;
    let n = num;
    format!(
        "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c",
        n(cx + r),
        n(cy),
        n(cx + r),
        n(cy + k),
        n(cx + k),
        n(cy + r),
        n(cx),
        n(cy + r),
        n(cx - k),
        n(cy + r),
        n(cx - r),
        n(cy + k),
        n(cx - r),
        n(cy),
        n(cx - r),
        n(cy - k),
        n(cx - k),
        n(cy - r),
        n(cx),
        n(cy - r),
        n(cx + k),
        n(cy - r),
        n(cx + r),
        n(cy - k),
        n(cx + r),
        n(cy),
    )
}

/// Moves each cluster of the first line to the centre of its own cell, `n`
/// cells across `width`, in visual order; right-to-left text fills the
/// cells from the right.
fn spread_into_cells(laid: &mut TextLayout, n: usize, width: f32) {
    let Some(line) = laid.lines.first_mut() else { return };
    let count: usize = line.runs.iter().map(|run| run.clusters.len()).sum();
    let first = if line.rtl { n.saturating_sub(count) } else { 0 };
    let cell = width / n as f32;
    let mut k = 0;
    for run in &mut line.runs {
        for cluster in run.clusters.clone() {
            let glyphs = &mut run.glyphs[cluster.glyphs.clone()];
            let left = glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            let right = glyphs.iter().map(|g| g.x + g.advance).fold(f32::NEG_INFINITY, f32::max);
            if left.is_finite() && right.is_finite() {
                let target = (first + k) as f32 * cell + cell / 2.0;
                let delta = target - (left + right) / 2.0;
                for glyph in glyphs {
                    glyph.x += delta;
                }
            }
            k += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(rect: [f32; 4], da: &str, q: Option<i64>) -> Look {
        let mk = Dict(vec![
            (b"BC".to_vec(), Object::Array(vec![Object::Int(0)])),
            (b"BG".to_vec(), Object::Array(vec![Object::Int(1)])),
        ]);
        Look::new(rect, da.as_bytes(), q, &mk, None, None, None)
    }

    fn content(appearance: &Appearance) -> String {
        String::from_utf8_lossy(&appearance.content).into_owned()
    }

    #[test]
    fn the_default_appearance_is_read() {
        assert_eq!(parse_da(b"/Helv 0 Tf 0 g"), (0.0, [0.0; 3]));
        assert_eq!(parse_da(b"/F1 9 Tf 1 0 0 rg"), (9.0, [1.0, 0.0, 0.0]));
        assert_eq!(parse_da(b"/F1 10 Tf 0 0 0 1 k"), (10.0, [0.0; 3]));
    }

    #[test]
    fn arabic_text_starts_at_the_right_and_english_at_the_left() {
        let field = look([0.0, 0.0, 200.0, 20.0], "/Helv 10 Tf 0 g", None);
        let x_of_first = |appearance: &Appearance| -> f32 {
            let text = content(appearance);
            let tm = text.split("Tm").next().expect("a glyph");
            tm.split_whitespace().rev().nth(1).and_then(|x| x.parse().ok()).expect("x")
        };
        let english = field.text("Name", false, None, false).expect("drawn");
        let arabic = field.text("الاسم", false, None, false).expect("drawn");
        assert!(x_of_first(&english) < 10.0, "{}", content(&english));
        assert!(x_of_first(&arabic) > 150.0, "{}", content(&arabic));
        assert_eq!(arabic.fonts[0].base_font.split('+').nth(1), Some("IBMPlexSansArabic"));
        assert!(content(&arabic).contains("/Tx BMC"));
        // Centred on request.
        let centred = look([0.0, 0.0, 200.0, 20.0], "/Helv 10 Tf 0 g", Some(1))
            .text("الاسم", false, None, false)
            .expect("drawn");
        let x = x_of_first(&centred);
        assert!(x > 60.0 && x < 140.0, "{x}");
    }

    #[test]
    fn fitted_text_fills_the_height_but_stays_inside_the_width() {
        let short = look([0.0, 0.0, 300.0, 14.0], "/Helv 0 Tf 0 g", None)
            .text("Hi", false, None, false)
            .expect("drawn");
        let size = |appearance: &Appearance| -> f32 {
            let text = content(appearance);
            let at = text.find(" Tf").expect("Tf");
            text[..at].split_whitespace().last().and_then(|s| s.parse().ok()).expect("size")
        };
        assert!(size(&short) < 12.0 && size(&short) > 6.0, "{}", size(&short));
        let long = look([0.0, 0.0, 60.0, 30.0], "/Helv 0 Tf 0 g", None)
            .text("A very long name that cannot fit", false, None, false)
            .expect("drawn");
        assert!(size(&long) < 5.0, "{}", size(&long));
    }

    #[test]
    fn comb_fields_put_one_character_in_each_cell() {
        let field = look([0.0, 0.0, 100.0, 20.0], "/Helv 10 Tf 0 g", None);
        let drawn = content(&field.text("1234", false, Some(10), false).expect("drawn"));
        let xs: Vec<f32> = drawn
            .lines()
            .filter(|line| line.ends_with("Tj"))
            .filter_map(|line| line.split_whitespace().nth(4)?.parse().ok())
            .collect();
        assert_eq!(xs.len(), 4);
        // Ten-point cells: each glyph starts a little before its cell's centre.
        for (i, x) in xs.iter().enumerate() {
            let centre = 10.0 * i as f32 + 5.0;
            assert!((x - centre).abs() < 4.0, "glyph {i} at {x}");
        }
    }

    #[test]
    fn passwords_show_bullets_and_lists_highlight_their_choices() {
        let field = look([0.0, 0.0, 100.0, 20.0], "/Helv 10 Tf 0 g", None);
        let secret = field.text("secret", false, None, true).expect("drawn");
        assert_eq!(secret.fonts[0].text.iter().map(|(_, t)| t.as_str()).collect::<String>(), "•");
        let list = look([0.0, 0.0, 120.0, 60.0], "/Helv 10 Tf 0 g", None)
            .list(&["الرياض", "Jeddah", "Dammam"], &[1], 0)
            .expect("drawn");
        assert_eq!(content(&list).matches("0.6 0.75 0.85 rg").count(), 1);
    }

    #[test]
    fn buttons_get_a_mark_when_on() {
        let field = look([0.0, 0.0, 12.0, 12.0], "/ZaDb 0 Tf 0 g", None);
        let (on, off) = field.button(ButtonStyle::Check);
        assert!(content(&on).contains(" l S") && !content(&off).contains(" l S"));
        let (on, _) = field.button(ButtonStyle::Radio);
        assert!(content(&on).contains(" c f"));
        let turned = Look::new(
            [0.0, 0.0, 20.0, 100.0],
            b"/Helv 10 Tf 0 g",
            None,
            &Dict(vec![(b"R".to_vec(), Object::Int(90))]),
            None,
            None,
            None,
        );
        let appearance = turned.text("x", false, None, false).expect("drawn");
        assert_eq!(appearance.bbox, [0.0, 0.0, 100.0, 20.0]);
        assert_eq!(appearance.matrix, Some([0.0, 1.0, -1.0, 0.0, 20.0, 0.0]));
    }
}
