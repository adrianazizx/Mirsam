//! Pieces both routes need once they place glyph IDs themselves: the font subset,
//! the glyph -> Unicode assignment behind the ToUnicode CMap, and the plan of
//! which glyph ranges get an /ActualText span.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use subsetter::GlyphRemapper;
use unicode_bidi::{BidiClass, bidi_class};

use crate::fontinfo::FontInfo;
use crate::shape::{Cluster, ShapedLine};

/// Everything derived from "these glyphs of this font are used".
pub struct Embed {
    /// Subset font program (glyph IDs are renumbered by the subsetter).
    pub subset: Vec<u8>,
    /// Original glyph ID -> CID. With Identity-H and /CIDToGIDMap /Identity the CID
    /// *is* the subset's new glyph ID, so content streams must use the remapped IDs.
    pub cid_of: HashMap<u16, u16>,
    pub n_cids: u16,
    /// Original glyph ID -> text, the source of the ToUnicode CMap.
    pub unicode_of: HashMap<u16, String>,
    /// Advance per CID in 1/1000 em (the /W array).
    pub widths: Vec<f64>,
}

impl Embed {
    pub fn build(
        font: &FontInfo,
        lines: &[ShapedLine],
        policy: ToUnicodePolicy,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut remapper = GlyphRemapper::new(); // .notdef is always glyph 0
        let mut cid_of = HashMap::new();
        cid_of.insert(0u16, 0u16);
        for g in lines.iter().flat_map(|l| &l.glyphs) {
            cid_of.entry(g.gid).or_insert_with(|| remapper.remap(g.gid));
        }
        let subset = subsetter::subset(&font.data, 0, &remapper).map_err(|e| format!("subset: {e}"))?;
        let n_cids = remapper.num_gids();

        let mut widths = vec![0.0; n_cids as usize];
        for (&gid, &cid) in &cid_of {
            widths[cid as usize] = font.k(font.advance(gid) as f64);
        }

        Ok(Self { subset, cid_of, n_cids, unicode_of: assign_unicode(font, lines, policy), widths })
    }

    pub fn cid(&self, gid: u16) -> u16 {
        self.cid_of[&gid]
    }

    /// The text a ToUnicode-only extractor would produce for a cluster.
    pub fn tounicode_text(&self, line: &ShapedLine, c: &Cluster) -> String {
        line.logical_glyphs(c)
            .iter()
            .filter_map(|&i| self.unicode_of.get(&line.glyphs[i].gid).map(String::as_str))
            .collect()
    }

    /// True when ToUnicode alone cannot reproduce the cluster's source text.
    pub fn needs_actual_text(&self, line: &ShapedLine, c: &Cluster) -> bool {
        self.tounicode_text(line, c) != line.cluster_text(c)
    }

    pub fn to_unicode_cmap(&self) -> String {
        let by_cid: BTreeMap<u16, &str> =
            self.unicode_of.iter().map(|(gid, s)| (self.cid_of[gid], s.as_str())).collect();
        let entries: Vec<(u16, &str)> = by_cid.into_iter().collect();
        let mut out = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        for chunk in entries.chunks(100) {
            out += &format!("{} beginbfchar\n", chunk.len());
            for (cid, text) in chunk {
                let hex: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
                out += &format!("<{cid:04X}> <{hex}>\n");
            }
            out += "endbfchar\n";
        }
        out += "endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToUnicodePolicy {
    /// Map each glyph to the source text it stands for in this document (logical
    /// order for ligatures, the typed bracket for a mirrored one).
    Source,
    /// What PDFium's `FPDFText_LoadFont` and Skia do: a glyph the font's cmap
    /// reaches maps to that code point (a presentation form for an encoded
    /// contextual/ligature glyph, the *drawn* bracket for a mirrored one); only
    /// glyphs outside the cmap fall back to `Source`.
    Cmap,
}

/// Decide what each used glyph maps back to.
///
/// 1. (`Cmap` policy only) the font's own cmap.
/// 2. A cluster with one glyph maps that glyph to the whole cluster text; this is
///    what makes ligatures (one glyph, several characters) extractable.
/// 3. In a cluster with several glyphs, a glyph whose cmap code point occurs in
///    the cluster text takes it; glyphs still unmapped share out what is left:
///    1:1 in logical order when the counts agree, otherwise the first takes all.
///
/// A glyph can only have one entry, so a glyph that stands for different text in
/// different places (or a cluster whose glyph order differs from its character
/// order) cannot round-trip through ToUnicode; `needs_actual_text` flags those.
fn assign_unicode(font: &FontInfo, lines: &[ShapedLine], policy: ToUnicodePolicy) -> HashMap<u16, String> {
    let mut map: HashMap<u16, String> = HashMap::new();
    if policy == ToUnicodePolicy::Cmap {
        for g in lines.iter().flat_map(|l| &l.glyphs) {
            if let Some(cp) =
                font.rev_cmap.get(&g.gid).and_then(|v| v.first()).and_then(|cp| char::from_u32(*cp))
            {
                map.insert(g.gid, cp.to_string());
            }
        }
    }
    for line in lines {
        for c in line.clusters.iter().filter(|c| c.glyphs.len() == 1) {
            map.entry(line.glyphs[c.glyphs.start].gid).or_insert_with(|| line.cluster_text(c).to_string());
        }
    }
    for line in lines {
        for c in line.clusters.iter().filter(|c| c.glyphs.len() > 1) {
            let mut rest = line.cluster_text(c).to_string();
            let take = |rest: &mut String, s: &str| match rest.find(s) {
                Some(at) => {
                    rest.replace_range(at..at + s.len(), "");
                    true
                }
                None => false,
            };
            let mut unmapped = Vec::new();
            for i in line.logical_glyphs(c) {
                let gid = line.glyphs[i].gid;
                if let Some(s) = map.get(&gid) {
                    take(&mut rest, s);
                    continue;
                }
                let own = font
                    .rev_cmap
                    .get(&gid)
                    .into_iter()
                    .flatten()
                    .filter_map(|cp| char::from_u32(*cp))
                    .find(|ch| rest.contains(*ch));
                match own {
                    Some(ch) => {
                        take(&mut rest, &ch.to_string());
                        map.insert(gid, ch.to_string());
                    }
                    None if !unmapped.contains(&gid) => unmapped.push(gid),
                    None => {}
                }
            }
            if rest.is_empty() {
                continue;
            }
            if unmapped.len() == rest.chars().count() {
                for (gid, ch) in unmapped.iter().zip(rest.chars()) {
                    map.insert(*gid, ch.to_string());
                }
            } else if let Some(first) = unmapped.first() {
                map.insert(*first, rest);
            }
        }
    }
    map
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActualText {
    /// ToUnicode only.
    None,
    /// A span around each cluster that ToUnicode cannot reproduce.
    Cluster,
    /// A span around every cluster, needed or not (probes only).
    EveryCluster,
    /// A span around every base-plus-marks group, and around any other cluster
    /// ToUnicode cannot reproduce.
    Grapheme,
    /// A span around every word of an RTL run (and LTR words that need one).
    Word,
    /// One span around the whole line.
    Line,
    /// `Grapheme`, bent to how PDFium's extractor treats /ActualText inside RTL
    /// text (it reverses runs of RTL letters and mirrors brackets as if the text
    /// were in visual order): ligatures and mirrored brackets get a span whose
    /// text is pre-reversed / pre-mirrored. Deliberately *not* what the PDF spec
    /// means by ActualText; here to measure what it would take.
    PdfiumTuned,
}

/// A stretch of consecutive glyphs (visual order) emitted as a unit.
#[derive(Debug, Clone)]
pub struct Segment {
    pub glyphs: Range<usize>,
    pub actual_text: Option<String>,
    pub run: usize,
    pub rtl: bool,
}

fn is_mark(c: char) -> bool {
    bidi_class(c) == BidiClass::NSM
}

/// The string PDFium's extractor turns back into `text` when it meets it as
/// /ActualText inside right-to-left content.
pub fn pdfium_visual(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let strong_rtl = |c: char| matches!(bidi_class(c), BidiClass::R | BidiClass::AL);
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let mut j = i;
        while j < chars.len() && strong_rtl(chars[j]) {
            j += 1;
        }
        if j > i {
            out.extend(chars[i..j].iter().rev());
            i = j;
        } else {
            out.push(match chars[i] {
                '(' => ')',
                ')' => '(',
                '[' => ']',
                ']' => '[',
                '{' => '}',
                '}' => '{',
                '<' => '>',
                '>' => '<',
                '«' => '»',
                '»' => '«',
                other => other,
            });
            i += 1;
        }
    }
    out.into_iter().collect()
}

pub fn plan_segments(line: &ShapedLine, embed: &Embed, mode: ActualText) -> Vec<Segment> {
    if mode == ActualText::Line {
        return vec![Segment {
            glyphs: 0..line.glyphs.len(),
            actual_text: Some(line.text.clone()),
            run: usize::MAX,
            rtl: line.base_rtl,
        }];
    }
    let mut out: Vec<Segment> = Vec::new();
    let all = |c: &Cluster, test: fn(char) -> bool| line.cluster_text(c).chars().all(test);
    let is_space = |c: &Cluster| all(c, char::is_whitespace);
    let is_mark_cluster = |c: &Cluster| all(c, is_mark);
    let text_of = |group: &[Cluster]| {
        let lo = group.iter().map(|w| w.text.start).min().unwrap_or(0);
        let hi = group.iter().map(|w| w.text.end).max().unwrap_or(0);
        line.text[lo..hi].to_string()
    };
    let mut i = 0;
    while i < line.clusters.len() {
        let c = &line.clusters[i];
        let same_run = |j: usize| j < line.clusters.len() && line.clusters[j].run == c.run;
        let mut j = i + 1;
        let actual_text = match mode {
            ActualText::Cluster if embed.needs_actual_text(line, c) => Some(line.cluster_text(c).to_string()),
            ActualText::EveryCluster => Some(line.cluster_text(c).to_string()),
            ActualText::Grapheme | ActualText::PdfiumTuned => {
                // Visual order: an RTL base comes after its marks, an LTR base before.
                if c.rtl {
                    while is_mark_cluster(&line.clusters[j - 1]) && same_run(j) {
                        j += 1;
                    }
                } else {
                    while same_run(j) && is_mark_cluster(&line.clusters[j]) {
                        j += 1;
                    }
                }
                let group = &line.clusters[i..j];
                let text = text_of(group);
                let tuned = if mode == ActualText::PdfiumTuned && c.rtl {
                    pdfium_visual(&text)
                } else {
                    text.clone()
                };
                let needed = group.len() > 1
                    || tuned != text
                    || group.iter().any(|w| embed.needs_actual_text(line, w));
                needed.then_some(tuned)
            }
            ActualText::Word if !is_space(c) => {
                while same_run(j) && !is_space(&line.clusters[j]) {
                    j += 1;
                }
                let word = &line.clusters[i..j];
                let needed = c.rtl || word.iter().any(|w| embed.needs_actual_text(line, w));
                needed.then(|| text_of(word))
            }
            _ => None,
        };
        let glyphs = c.glyphs.start..line.clusters[j - 1].glyphs.end;
        match out.last_mut() {
            // Plain neighbours in the same run merge into one segment.
            Some(prev) if actual_text.is_none() && prev.actual_text.is_none() && prev.run == c.run => {
                prev.glyphs.end = glyphs.end;
            }
            _ => out.push(Segment { glyphs, actual_text, run: c.run, rtl: c.rtl }),
        }
        i = j;
    }
    out
}

/// PDF text string: UTF-16BE with BOM, as hex digits (no angle brackets).
pub fn utf16be_hex(s: &str) -> String {
    let mut out = String::from("FEFF");
    for u in s.encode_utf16() {
        out += &format!("{u:04X}");
    }
    out
}
