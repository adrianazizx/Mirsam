//! The text layout engine (ADR 0020): English, Arabic and mixed paragraphs,
//! shaped with the OFL fonts in `assets/fonts/`.

use std::path::PathBuf;

use safha_core::typeset::{
    Align, BaseDirection, FontFace, GlyphRun, Line, LineHeight, ParagraphStyle, Span, TextLayout, layout,
};

fn font(file: &str) -> FontFace {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file);
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    FontFace::from_bytes(data, 0).unwrap_or_else(|e| panic!("{file}: {e}"))
}

fn amiri() -> FontFace {
    font("Amiri-Regular.ttf")
}

const SIZE: f32 = 18.0;

fn lay(text: &str, fonts: &[FontFace], style: &ParagraphStyle) -> TextLayout {
    let spans = [Span::new(0..text.len(), 0, SIZE)];
    let out = layout(text, &spans, fonts, style).unwrap_or_else(|e| panic!("layout {text:?}: {e}"));
    check_invariants(text, &out, style);
    out
}

fn unbounded() -> ParagraphStyle {
    ParagraphStyle::default()
}

fn width(w: f32) -> ParagraphStyle {
    ParagraphStyle { width: Some(w), ..ParagraphStyle::default() }
}

/// Cluster texts of a line, left to right as drawn.
fn visual_clusters<'a>(text: &'a str, line: &Line) -> Vec<&'a str> {
    line.runs.iter().flat_map(|r| r.clusters.iter().map(|c| &text[c.text.clone()])).collect()
}

/// The text of each run, left to right.
fn visual_runs<'a>(text: &'a str, line: &Line) -> Vec<(&'a str, bool)> {
    line.runs.iter().map(|r| (&text[r.text_range.clone()], r.rtl)).collect()
}

fn line_text<'a>(text: &'a str, line: &Line) -> &'a str {
    &text[line.text_range.clone()]
}

fn is_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}' | '\u{000B}' | '\u{000C}' | '\u{0085}')
}

/// What must hold for any layout:
/// - lines cover the text in order, without gaps;
/// - each run's clusters cover its text exactly once, and every glyph is in a cluster;
/// - every drawn character of a line is in exactly one run;
/// - runs are laid left to right without overlapping;
/// - lines fit the box, unless a single grapheme is wider than it;
/// - baselines go down the page.
fn check_invariants(text: &str, out: &TextLayout, style: &ParagraphStyle) {
    let mut at = 0;
    for line in &out.lines {
        assert_eq!(line.text_range.start, at, "lines must be contiguous: {:?}", out.lines);
        at = line.text_range.end;

        let mut covered: Vec<usize> = Vec::new();
        let mut previous_end = f32::NEG_INFINITY;
        for run in &line.runs {
            let mut cluster_bytes: Vec<usize> = run.clusters.iter().flat_map(|c| c.text.clone()).collect();
            cluster_bytes.sort_unstable();
            assert_eq!(
                cluster_bytes,
                run.text_range.clone().collect::<Vec<_>>(),
                "clusters must cover run {run:?}"
            );
            let glyphs_in_clusters: usize = run.clusters.iter().map(|c| c.glyphs.len()).sum();
            assert_eq!(glyphs_in_clusters, run.glyphs.len());
            for (i, c) in run.clusters.iter().enumerate() {
                assert!(run.glyphs[c.glyphs.clone()].iter().all(|g| g.cluster == i));
            }
            covered.extend(run.text_range.clone());

            let (start, end) = pen_extent(run);
            assert!(
                start >= previous_end - 0.01,
                "runs overlap at x={start} (previous ended at {previous_end})"
            );
            previous_end = end;
        }
        covered.sort_unstable();
        let drawn: Vec<usize> = line
            .text_range
            .clone()
            .filter(|&i| text.is_char_boundary(i))
            .flat_map(|i| {
                let c = text[i..].chars().next().unwrap_or(' ');
                if is_break(c) { Vec::new() } else { (i..i + c.len_utf8()).collect() }
            })
            .collect();
        assert_eq!(covered, drawn, "line {:?} must draw each character once", line_text(text, line));

        if let Some(w) = style.width {
            let one_grapheme = line_text(text, line).trim().chars().filter(|c| !is_mark(*c)).count() <= 1;
            assert!(
                line.width <= w + 0.01 || one_grapheme,
                "line {:?} is {} wide, box {w}",
                line_text(text, line),
                line.width
            );
        }
        assert!(line.height > 0.0 && line.baseline > line.top && line.baseline < line.top + line.height);
    }
    assert_eq!(at, text.len(), "lines must reach the end of the text");
    assert!(out.lines.windows(2).all(|w| w[1].baseline > w[0].baseline));
}

fn is_mark(c: char) -> bool {
    ('\u{064B}'..='\u{065F}').contains(&c) || c == '\u{0670}'
}

/// Pen start and end of a run (glyph x minus nothing: offsets are small; use
/// advances from the first glyph's pen).
fn pen_extent(run: &GlyphRun) -> (f32, f32) {
    let start = run.glyphs.first().map_or(0.0, |g| g.x);
    let total: f32 = run.glyphs.iter().map(|g| g.advance).sum();
    // The first glyph's x includes its offset; marks have zero advance, so the
    // run's pen start is best estimated from the first spacing glyph.
    let pen_start = run.glyphs.iter().find(|g| g.advance > 0.0).map_or(start, |g| {
        let before: f32 = run.glyphs.iter().take_while(|h| !std::ptr::eq(*h, g)).map(|h| h.advance).sum();
        g.x - before
    });
    (pen_start, pen_start + total)
}

// ---------------------------------------------------------------- English (LTR)

#[test]
fn english_line_is_left_to_right_with_ligatures() {
    let text = "The office staff finished the final draft efficiently.";
    let out = lay(text, &[amiri()], &unbounded());
    assert_eq!(out.lines.len(), 1);
    let line = &out.lines[0];
    assert!(!line.rtl);
    assert_eq!(line.x, 0.0, "start alignment is the left edge for LTR");
    assert!(line.runs.iter().all(|r| !r.rtl));
    // Amiri's ligatures: ffi, ff, fi, ffi again; each one glyph standing for several letters.
    let clusters = visual_clusters(text, line);
    for ligature in ["ffi", "ff", "fi"] {
        assert!(clusters.contains(&ligature), "expected a {ligature} ligature in {clusters:?}");
    }
    // Logical order is visual order: cluster offsets increase left to right.
    let starts: Vec<usize> = line.runs.iter().flat_map(|r| r.clusters.iter().map(|c| c.text.start)).collect();
    assert!(starts.windows(2).all(|w| w[0] < w[1]));
    // Glyphs are placed left to right.
    let xs: Vec<f32> = line.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| g.x)).collect();
    assert!(xs.windows(2).all(|w| w[0] <= w[1]));
    assert!((out.width - line.width).abs() < 0.01);
}

#[test]
fn english_paragraph_wraps_at_spaces() {
    let text = "Revenue rose 12.5% to $4,500,000 in Q1 2026, while costs fell by 3%.";
    let out = lay(text, &[font("IBMPlexSans-VF.ttf")], &width(160.0));
    assert!(out.lines.len() >= 3, "{} lines", out.lines.len());
    for line in &out.lines {
        let t = line_text(text, line);
        // Breaks are at spaces (UAX #14), never inside "$4,500,000" or "12.5%".
        assert!(line.text_range.end == text.len() || t.ends_with(' '), "line {t:?} ends mid-word");
        assert_eq!(line.x, 0.0);
    }
    assert!(out.lines.iter().any(|l| line_text(text, l).contains("$4,500,000")));
    assert!(out.lines.iter().any(|l| line_text(text, l).contains("12.5%")));
}

#[test]
fn variable_font_instance_changes_widths() {
    let regular = font("IBMPlexSans-VF.ttf");
    let bold = regular.with_variations(&[(*b"wght", 700.0)]).unwrap_or_else(|e| panic!("{e}"));
    let text = "Quarterly Report";
    let w_regular = lay(text, &[regular], &unbounded()).width;
    let w_bold = lay(text, &[bold], &unbounded()).width;
    assert!(w_bold > w_regular * 1.02, "bold {w_bold} vs regular {w_regular}");
}

// ---------------------------------------------------------------- Arabic (RTL)

/// Glyph ids of each letter of `word` shaped alone (isolated forms).
fn isolated_glyphs(word: &str, f: &FontFace) -> Vec<u32> {
    word.chars()
        .map(|c| {
            let s = c.to_string();
            let out = lay(&s, std::slice::from_ref(f), &unbounded());
            out.lines[0].runs[0].glyphs[0].id
        })
        .collect()
}

#[test]
fn arabic_word_joins_and_reads_right_to_left() {
    let f = amiri();
    let text = "سلم";
    let out = lay(text, std::slice::from_ref(&f), &unbounded());
    let line = &out.lines[0];
    assert!(line.rtl, "first strong character is Arabic");
    let run = &line.runs[0];
    assert!(run.rtl);
    assert_eq!(&run.script, b"Arab");
    // Visual order is the reverse of logical: the last letter is leftmost.
    assert_eq!(visual_clusters(text, line), vec!["م", "ل", "س"]);
    // Every letter takes a joined form, not its isolated one.
    let joined: Vec<u32> = run.glyphs.iter().rev().map(|g| g.id).collect();
    let isolated = isolated_glyphs(text, &f);
    for (i, (j, iso)) in joined.iter().zip(&isolated).enumerate() {
        assert_ne!(j, iso, "letter {i} of {text} is not joined");
    }
}

#[test]
fn arabic_line_aligns_right_by_default() {
    let text = "السلام عليكم";
    let out = lay(text, &[amiri()], &width(300.0));
    let line = &out.lines[0];
    assert!((line.x + line.width - 300.0).abs() < 0.01, "x {} width {}", line.x, line.width);
    let left = lay(text, &[amiri()], &ParagraphStyle { align: Align::Left, ..width(300.0) });
    assert_eq!(left.lines[0].x, 0.0);
}

#[test]
fn lam_alef_forms_a_ligature_where_the_font_has_one() {
    // IBM Plex Sans Arabic draws lam-alef as one glyph.
    let text = "لا";
    let out = lay(text, &[font("IBMPlexSansArabic-Regular.ttf")], &unbounded());
    let run = &out.lines[0].runs[0];
    assert_eq!(run.glyphs.len(), 1, "{run:?}");
    assert_eq!(run.clusters, vec![safha_core::typeset::Cluster { text: 0..text.len(), glyphs: 0..1 }]);
}

#[test]
fn vocalized_marks_are_positioned_on_their_letters() {
    let text = "قَالَ الْمُعَلِّمُ";
    let out = lay(text, &[amiri()], &unbounded());
    let line = &out.lines[0];
    let marks: Vec<_> = line
        .runs
        .iter()
        .flat_map(|r| r.glyphs.iter().map(move |g| (r, g)))
        .filter(|(r, g)| text[r.clusters[g.cluster].text.clone()].chars().all(is_mark))
        .collect();
    // Every mark of the input is drawn, by glyphs that do not advance the pen.
    let mark_count = text.chars().filter(|c| is_mark(*c)).count();
    let mut drawn: Vec<usize> = marks.iter().flat_map(|(r, g)| r.clusters[g.cluster].text.clone()).collect();
    drawn.sort_unstable();
    drawn.dedup();
    assert_eq!(drawn.len(), mark_count * 2, "every mark (2 bytes each) has a glyph");
    assert!(marks.iter().all(|(_, g)| g.advance.abs() < 0.01), "{marks:?}");
    // Each mark is placed over a letter: within the span of the spacing glyphs
    // next to it, not left at the pen position of the line start.
    let line_end = line.x + line.width;
    assert!(marks.iter().all(|(_, g)| g.x > line.x - SIZE && g.x < line_end), "{marks:?}");
    // Noto Naskh moves marks vertically too (GPOS mark anchors).
    let naskh = lay(text, &[font("NotoNaskhArabic-VF.ttf")], &unbounded());
    assert!(naskh.runs().flat_map(|r| r.glyphs.iter()).any(|g| g.advance == 0.0 && g.y_offset != 0.0));
}

#[test]
fn arabic_paragraph_wraps_right_to_left() {
    let text = "لا بأس؛ لأن الأولاد الآن في الملعب، فلا داعي للقلق.";
    let out = lay(text, &[amiri()], &width(140.0));
    assert!(out.lines.len() >= 3);
    for line in &out.lines {
        assert!(line.rtl);
        assert!((line.x + line.width - 140.0).abs() < 0.01, "RTL lines end at the right edge");
        // Inside each line the logically first character is rightmost.
        let clusters = &line.runs.last().map(|r| r.clusters.clone()).unwrap_or_default();
        let rightmost = clusters.last().map(|c| c.text.start);
        assert_eq!(rightmost, Some(line.text_range.start), "line {:?}", line_text(text, line));
    }
    // The first line holds the start of the text.
    assert!(line_text(text, &out.lines[0]).starts_with("لا بأس"));
}

// ---------------------------------------------------------------- Mixed

#[test]
fn english_product_name_and_numbers_inside_arabic() {
    let text = "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال.";
    let out = lay(text, &[amiri()], &unbounded());
    let line = &out.lines[0];
    assert!(line.rtl);
    let runs = visual_runs(text, line);
    // Left to right as drawn: the end of the sentence is on the left.
    let ltr: Vec<&str> = runs.iter().filter(|(_, rtl)| !rtl).map(|(t, _)| *t).collect();
    assert_eq!(ltr, vec!["4,500", "iPhone 15 Pro", "3"], "LTR runs left to right: {runs:?}");
    let all = runs.iter().map(|(t, _)| *t).collect::<String>();
    let pos = |needle: &str| all.find(needle).unwrap_or_else(|| panic!("{needle} missing from {all:?}"));
    // Reading right to left: اشتريت, 3, أجهزة, iPhone 15 Pro, بسعر, 4,500, ريال.
    assert!(pos("ريال") < pos("4,500") && pos("4,500") < pos("بسعر") && pos("بسعر") < pos("iPhone"));
    assert!(pos("Pro") < pos("أجهزة") && pos("أجهزة") < pos("3") && pos("3") < pos("اشتريت"));
    // The period ends the RTL sentence: leftmost.
    assert_eq!(visual_clusters(text, line)[0], ".");
    // Inside "iPhone 15 Pro" the letters run left to right.
    let iphone = line
        .runs
        .iter()
        .find(|r| &text[r.text_range.clone()] == "iPhone 15 Pro")
        .map(|r| visual_clusters_of(text, r));
    assert_eq!(iphone.as_deref().map(|c| c.concat()), Some("iPhone 15 Pro".to_string()));
}

fn visual_clusters_of<'a>(text: &'a str, run: &GlyphRun) -> Vec<&'a str> {
    run.clusters.iter().map(|c| &text[c.text.clone()]).collect()
}

#[test]
fn digits_inside_arabic_read_left_to_right() {
    for (text, number) in [("عام 2026 مضى", "2026"), ("عام ٢٠٢٦ مضى", "٢٠٢٦")] {
        let out = lay(text, &[amiri()], &unbounded());
        let line = &out.lines[0];
        let run = line.runs.iter().find(|r| &text[r.text_range.clone()] == number);
        let run = run.unwrap_or_else(|| panic!("{number} is not its own run: {:?}", visual_runs(text, line)));
        assert!(!run.rtl, "{number} is laid left to right");
        assert_eq!(visual_clusters_of(text, run).concat(), number);
        // And it sits between the two Arabic words: left of عام, right of مضى.
        let order: Vec<&str> =
            visual_runs(text, line).into_iter().map(|(t, _)| t.trim()).filter(|t| !t.is_empty()).collect();
        assert_eq!(order, vec!["مضى", number, "عام"], "{text}");
    }
}

#[test]
fn brackets_are_mirrored_in_right_to_left_text() {
    let f = amiri();
    let text = "(شاملة الضريبة)";
    let out = lay(text, std::slice::from_ref(&f), &unbounded());
    let line = &out.lines[0];
    let clusters = visual_clusters(text, line);
    // Logically "(" comes first, so it is drawn rightmost, and ")" leftmost.
    assert_eq!(clusters.first(), Some(&")"));
    assert_eq!(clusters.last(), Some(&"("));
    // In RTL the shaper mirrors: the logical "(" is drawn with the glyph of ")".
    let glyph_of = |s: &str, rtl: bool| {
        let style = ParagraphStyle {
            direction: if rtl { BaseDirection::Rtl } else { BaseDirection::Ltr },
            ..unbounded()
        };
        lay(s, std::slice::from_ref(&f), &style).lines[0].runs[0].glyphs[0].id
    };
    let rightmost = line.runs.last().and_then(|r| r.glyphs.last()).map(|g| g.id);
    assert_eq!(rightmost, Some(glyph_of(")", false)));
}

#[test]
fn arabic_phrase_inside_english_paragraph() {
    let text = "The contract (العقد) is signed.";
    let out = lay(text, &[amiri()], &unbounded());
    let line = &out.lines[0];
    assert!(!line.rtl);
    let runs = visual_runs(text, line);
    let arabic = runs.iter().find(|(t, _)| t.contains("العقد")).map(|(_, rtl)| *rtl);
    assert_eq!(arabic, Some(true));
    // The brackets belong to the English sentence: "(" left of the Arabic word.
    let clusters = visual_clusters(text, line).concat();
    assert!(clusters.starts_with("The contract ("), "{clusters}");
}

#[test]
fn mixed_paragraph_wraps_with_each_line_reordered() {
    let text = "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال (شاملة الضريبة).";
    let out = lay(text, &[amiri()], &width(150.0));
    assert!(out.lines.len() >= 3);
    // "iPhone 15 Pro" may be split across lines, but every LTR piece keeps its
    // letters in order.
    for line in &out.lines {
        for run in line.runs.iter().filter(|r| !r.rtl) {
            let starts: Vec<usize> = run.clusters.iter().map(|c| c.text.start).collect();
            assert!(starts.windows(2).all(|w| w[0] < w[1]));
        }
        for run in line.runs.iter().filter(|r| r.rtl) {
            let starts: Vec<usize> = run.clusters.iter().map(|c| c.text.start).collect();
            assert!(starts.windows(2).all(|w| w[0] > w[1]));
        }
    }
}

#[test]
fn direction_is_detected_per_paragraph() {
    let cases = [("Hello مرحبا", false), ("مرحبا Hello", true), ("2026 مرحبا", true), ("123", false)];
    for (text, rtl) in cases {
        assert_eq!(lay(text, &[amiri()], &unbounded()).lines[0].rtl, rtl, "{text}");
    }
    // Each paragraph after a newline decides again.
    let out = lay("Hello\nمرحبا", &[amiri()], &unbounded());
    assert_eq!(out.lines.iter().map(|l| l.rtl).collect::<Vec<_>>(), vec![false, true]);
    // An explicit direction overrides detection.
    let forced = lay("Hello", &[amiri()], &ParagraphStyle { direction: BaseDirection::Rtl, ..unbounded() });
    assert!(forced.lines[0].rtl);
}

#[test]
fn fallback_font_draws_what_the_first_font_lacks() {
    // IBM Plex Sans (Latin) has no Arabic; Noto Naskh Arabic has.
    let fonts = [font("IBMPlexSans-VF.ttf"), font("NotoNaskhArabic-VF.ttf")];
    let text = "Hello مرحبا بكم office";
    let out = lay(text, &fonts, &unbounded());
    for run in &out.lines[0].runs {
        let t = &text[run.text_range.clone()];
        if t.chars().any(|c| c.is_ascii_alphabetic()) {
            assert_eq!(run.font, 0, "{t}");
        }
        if t.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)) {
            assert_eq!(run.font, 1, "{t}");
        }
    }
    // No .notdef: glyph 0 is never used.
    assert!(out.runs().all(|r| r.glyphs.iter().all(|g| g.id != 0)));
}

// ---------------------------------------------------------------- Lines

#[test]
fn hard_line_breaks_and_empty_text() {
    let text = "first\u{2028}second";
    let out = lay(text, &[amiri()], &unbounded());
    assert_eq!(out.lines.len(), 2);
    assert_eq!(line_text(text, &out.lines[0]), "first\u{2028}");

    let out = lay("a\n\nb\n", &[amiri()], &unbounded());
    assert_eq!(out.lines.len(), 4, "a, the empty line, b, and the empty last line");
    assert!(out.lines[1].runs.is_empty() && out.lines[3].runs.is_empty());

    let empty =
        layout("", &[Span::new(0..0, 0, SIZE)], &[amiri()], &unbounded()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(empty.lines.len(), 1, "an empty paragraph still has a line for the caret");
    assert!(empty.height > 0.0);
}

#[test]
fn overlong_word_breaks_between_graphemes() {
    let text = "Supercalifragilistic";
    let out = lay(text, &[amiri()], &width(40.0));
    assert!(out.lines.len() > 2);
    assert!(out.lines.iter().all(|l| l.width <= 40.01));
    // An Arabic word with marks never separates a mark from its letter.
    let text = "الْمُعَلِّمُونَ";
    let out = lay(text, &[amiri()], &width(25.0));
    assert!(out.lines.len() > 1);
    for line in &out.lines {
        let first = line_text(text, line).chars().next();
        assert!(first.is_some_and(|c| !is_mark(c)), "line starts with a mark: {:?}", line_text(text, line));
    }
}

#[test]
fn justified_lines_fill_the_width_except_the_last() {
    let text = "The office staff finished the final draft efficiently; the affluent client found it difficult to fault.";
    let style = ParagraphStyle { align: Align::Justify, ..width(200.0) };
    let out = lay(text, &[amiri()], &style);
    let (last, rest) = out.lines.split_last().unwrap_or_else(|| panic!("no lines"));
    assert!(!rest.is_empty());
    for line in rest {
        assert!((line.width - 200.0).abs() < 0.01, "line {:?} is {}", line_text(text, line), line.width);
        assert_eq!(line.x, 0.0);
    }
    assert!(last.width < 200.0);

    // Arabic justifies too, flush on both sides.
    let text = "لا بأس؛ لأن الأولاد الآن في الملعب، فلا داعي للقلق.";
    let out = lay(text, &[amiri()], &ParagraphStyle { align: Align::Justify, ..width(140.0) });
    let first = &out.lines[0];
    assert!((first.x).abs() < 0.01 && (first.width - 140.0).abs() < 0.01, "{first:?}");
}

#[test]
fn line_height_options() {
    let text = "one\u{2028}two";
    let normal = lay(text, &[amiri()], &unbounded());
    let gap = normal.lines[1].baseline - normal.lines[0].baseline;
    assert!(gap > SIZE, "Amiri's normal line is taller than its size: {gap}");
    let fixed =
        lay(text, &[amiri()], &ParagraphStyle { line_height: LineHeight::Points(24.0), ..unbounded() });
    assert!((fixed.lines[1].baseline - fixed.lines[0].baseline - 24.0).abs() < 0.01);
    let double =
        lay(text, &[amiri()], &ParagraphStyle { line_height: LineHeight::Multiple(2.0), ..unbounded() });
    assert!((double.lines[1].baseline - double.lines[0].baseline - 2.0 * gap).abs() < 0.01);
}

#[test]
fn spans_set_size_and_font_per_stretch() {
    let text = "Big small";
    let fonts = [amiri(), font("IBMPlexSans-VF.ttf")];
    let spans = [Span::new(0..4, 0, 36.0), Span::new(4..9, 1, 12.0)];
    let out = layout(text, &spans, &fonts, &unbounded()).unwrap_or_else(|e| panic!("{e}"));
    check_invariants(text, &out, &unbounded());
    let runs = &out.lines[0].runs;
    assert!(runs.iter().any(|r| r.font == 0 && r.size == 36.0));
    assert!(runs.iter().any(|r| r.font == 1 && r.size == 12.0));

    // Spans must cover the text exactly.
    for bad in [vec![Span::new(0..4, 0, 12.0)], vec![Span::new(0..9, 5, 12.0)], vec![Span::new(0..9, 0, 0.0)]]
    {
        assert!(layout(text, &bad, &fonts, &unbounded()).is_err(), "{bad:?}");
    }
}

// ---------------------------------------------------------------- Font matching

use safha_core::typeset::{FontRequest, MatchKind, match_font};

const BUNDLED: [&str; 7] = [
    "Amiri-Regular.ttf",
    "Amiri-Bold.ttf",
    "IBMPlexSans-VF.ttf",
    "IBMPlexSansArabic-Regular.ttf",
    "IBMPlexSansArabic-SemiBold.ttf",
    "NotoNaskhArabic-VF.ttf",
    "NotoSansArabic-VF.ttf",
];

fn bundled() -> Vec<FontFace> {
    BUNDLED.iter().map(|f| font(f)).collect()
}

const ARABIC: &str = "السلام عليكم ورحمة الله";

fn pick(name: &str, text: &str, fonts: &[FontFace]) -> (usize, MatchKind, bool) {
    let request = FontRequest::from_pdf(name, None, None, None, None);
    let m = match_font(&request, text, fonts).unwrap_or_else(|| panic!("no match for {name}"));
    (m.index, m.kind, m.covers_text)
}

#[test]
fn matching_prefers_the_same_family_and_style() {
    let fonts = bundled();
    assert_eq!(pick("ABCDEF+Amiri-Bold", ARABIC, &fonts), (1, MatchKind::Exact, true));
    assert_eq!(pick("Amiri-Regular", ARABIC, &fonts), (0, MatchKind::Exact, true));
    // No bold italic Amiri here: the bold, flagged as another style.
    assert_eq!(pick("Amiri-BoldItalic", ARABIC, &fonts), (1, MatchKind::SameFamily, true));
    // Bold asked, SemiBold is the nearest weight of the family.
    assert_eq!(pick("IBMPlexSansArabic-Bold", ARABIC, &fonts), (4, MatchKind::SameFamily, true));
}

#[test]
fn matching_substitutes_by_classification() {
    let fonts = bundled();
    // Windows' Naskh faces become a serif Naskh here; its sans faces a sans Arabic.
    for name in ["TraditionalArabic", "ABCDEF+SimplifiedArabic-Bold", "TimesNewRomanPSMT"] {
        let (index, kind, covers) = pick(name, ARABIC, &fonts);
        assert_eq!(kind, MatchKind::Substitute, "{name}");
        assert!(covers, "{name}");
        assert_eq!(fonts[index].traits().serif, Some(true), "{name} matched {}", BUNDLED[index]);
    }
    for name in ["Tahoma", "Arial,Bold", "SegoeUI"] {
        let (index, _, covers) = pick(name, ARABIC, &fonts);
        assert!(covers);
        assert_eq!(fonts[index].traits().serif, Some(false), "{name} matched {}", BUNDLED[index]);
    }
    // "Arial,Bold" keeps the weight: the SemiBold sans, not a regular.
    assert_eq!(pick("Arial,Bold", ARABIC, &fonts).0, 4);
}

#[test]
fn matching_never_picks_a_font_that_cannot_draw_the_text() {
    let fonts = bundled();
    // IBM Plex Sans has no Arabic: its Arabic relative is chosen instead.
    let (index, kind, covers) = pick("IBMPlexSans-Regular", ARABIC, &fonts);
    assert_eq!((index, kind, covers), (3, MatchKind::Substitute, true));
    // For English it is the exact match.
    assert_eq!(pick("IBMPlexSans-Regular", "Quarterly Report", &fonts), (2, MatchKind::Exact, true));
    // The matched font sets the whole paragraph without .notdef.
    let text = "اشتريت 3 أجهزة iPhone 15 Pro";
    let (index, _, covers) = pick("ABCDEF+Mystery", text, &fonts);
    assert!(covers);
    let out = lay(text, std::slice::from_ref(&fonts[index]), &unbounded());
    assert!(out.runs().all(|r| r.font == 0 && r.glyphs.iter().all(|g| g.id != 0)));
}

#[test]
fn matching_uses_descriptor_weight_slant_and_variable_instances() {
    let fonts = bundled();
    // An unknown family, FontWeight 600 and the Serif flag clear: weight decides.
    let request = FontRequest::from_pdf("ABCDEF+Unknown", None, Some(600.0), Some(32), Some(0.0));
    assert_eq!(match_font(&request, ARABIC, &fonts).map(|m| m.index), Some(4));
    // A bold instance of a variable font matches a bold request exactly.
    let plex = font("IBMPlexSans-VF.ttf");
    let candidates =
        vec![plex.clone(), plex.with_variations(&[(*b"wght", 700.0)]).unwrap_or_else(|e| panic!("{e}"))];
    assert_eq!(pick("IBMPlexSans-Bold", "Quarterly Report", &candidates), (1, MatchKind::Exact, true));
}
