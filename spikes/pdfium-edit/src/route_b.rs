//! Route B: write the font objects and content streams ourselves with lopdf.
//!
//! Type0 -> CIDFontType2 (Identity-H, /CIDToGIDMap /Identity), FontFile2 = subset,
//! /W from hmtx, ToUnicode from the shaper's clusters, glyphs placed in visual
//! order, optional /Span /ActualText and /ReversedChars marked content.

use lopdf::{Dictionary, Document, Object, Stream, dictionary};

use crate::Layout;
use crate::fontinfo::FontInfo;
use crate::pdfparts::{ActualText, Embed, ToUnicodePolicy, plan_segments, utf16be_hex};
use crate::shape::ShapedLine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Positioning {
    /// One `TJ` array per stretch, numbers inside it for every deviation from the
    /// font's own advances, `Ts` (text rise) for vertical GPOS offsets.
    Tj,
    /// Skia/Chrome style: an explicit `dx dy Td` before any glyph that does not sit
    /// exactly where the previous advance left the pen, then `<..> Tj`.
    Td,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub to_unicode: ToUnicodePolicy,
    pub actual_text: ActualText,
    pub positioning: Positioning,
    /// Wrap each RTL run in `/ReversedChars BMC .. EMC` like Skia does.
    pub reversed_chars: bool,
    /// Replace every /ActualText with a marker string, to see who honours it.
    pub probe: Option<Probe>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// "PROBE", whatever the span covers.
    Short,
    /// One "x" per glyph in the span.
    OnePerGlyph,
}

/// Where one line goes on a page and which font resource draws it.
pub struct Placement<'a> {
    pub size: f64,
    /// Line origin (left end of the visual line) and baseline, in page points.
    pub x: f64,
    pub baseline: f64,
    pub resource: &'a str,
}

/// Adds the Type0 font and everything under it; returns the Type0 dictionary's ID.
fn add_font(doc: &mut Document, font: &FontInfo, embed: &Embed) -> lopdf::ObjectId {
    let k = |v: i16| Object::Real(font.k(v as f64) as f32);
    // Subset tag: six capitals. A real writer derives it from the glyph set.
    let base_font = format!("MIRSAM+{}", font.name);
    let font_file = doc.add_object(Stream::new(
        dictionary! { "Length1" => embed.subset.len() as i64 },
        embed.subset.clone(),
    ));
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => Object::Name(base_font.clone().into_bytes()),
        "Flags" => 4, // symbolic: glyphs are addressed by ID, not by a standard encoding
        "FontBBox" => vec![k(font.bbox[0]), k(font.bbox[1]), k(font.bbox[2]), k(font.bbox[3])],
        "ItalicAngle" => Object::Real(font.italic_angle),
        "Ascent" => k(font.ascent),
        "Descent" => k(font.descent),
        "CapHeight" => k(font.cap_height),
        "StemV" => 80,
        "FontFile2" => font_file,
    });
    let widths: Vec<Object> = embed.widths.iter().map(|w| Object::Real(*w as f32)).collect();
    let cid_font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => Object::Name(base_font.clone().into_bytes()),
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "FontDescriptor" => descriptor,
        "DW" => 1000,
        "W" => vec![Object::Integer(0), Object::Array(widths)],
        "CIDToGIDMap" => "Identity",
    });
    let to_unicode = doc.add_object(Stream::new(Dictionary::new(), embed.to_unicode_cmap().into_bytes()));
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => Object::Name(base_font.into_bytes()),
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Reference(cid_font)],
        "ToUnicode" => to_unicode,
    })
}

pub fn write(
    font: &FontInfo,
    lines: &[ShapedLine],
    opt: Options,
    layout: &Layout,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let embed = Embed::build(font, lines, opt.to_unicode)?;
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let type0 = add_font(&mut doc, font, &embed);
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => type0 } });

    let mut kids = Vec::new();
    for line in lines {
        let place = Placement {
            size: layout.font_size as f64,
            x: layout.line_x(font, line),
            baseline: layout.baseline as f64,
            resource: "F1",
        };
        let content = line_content(font, &embed, line, opt, &place);
        let content_id = doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources,
        });
        kids.push(Object::Reference(page));
    }
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => kids.len() as i64,
            "Kids" => kids,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(layout.page_w), Object::Real(layout.page_h)],
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.compress();

    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

/// Adds one line to the first page of an existing PDF: parse it with lopdf, append
/// the font objects and a content stream, and write the whole file out again.
///
/// The existing content streams are left as they are. A `q` stream goes in front
/// of them and ours starts with `Q`, so whatever graphics state the page leaves
/// behind (Chrome leaves a flipped CTM) cannot reach the new text.
pub fn insert_into(
    existing: &[u8],
    password: Option<&str>,
    font: &FontInfo,
    line: &ShapedLine,
    opt: Options,
    place: &Placement,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut doc = match password {
        Some(password) => {
            Document::load_mem_with_options(existing, lopdf::LoadOptions::with_password(password))?
        }
        None => Document::load_mem(existing)?,
    };
    let page_id = *doc.get_pages().get(&1).ok_or("no first page")?;
    let embed = Embed::build(font, std::slice::from_ref(line), opt.to_unicode)?;
    let first_new_id = doc.max_id + 1;
    let type0 = add_font(&mut doc, font, &embed);

    // Resources may be inline, indirect or inherited; work on a resolved copy.
    let mut resources = {
        let (inline, inherited) = doc.get_page_resources(page_id)?;
        match (inline, inherited.first()) {
            (Some(dict), _) => dict.clone(),
            (None, Some(id)) => doc.get_dictionary(*id)?.clone(),
            (None, None) => Dictionary::new(),
        }
    };
    let mut fonts = match resources.get(b"Font") {
        Ok(Object::Dictionary(dict)) => dict.clone(),
        Ok(Object::Reference(id)) => doc.get_dictionary(*id)?.clone(),
        _ => Dictionary::new(),
    };
    if fonts.has(place.resource.as_bytes()) {
        return Err(format!("font resource /{} already exists on the page", place.resource).into());
    }
    fonts.set(place.resource, type0);
    resources.set("Font", fonts);

    let mut contents: Vec<Object> =
        vec![doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec())).into()];
    contents.extend(doc.get_page_contents(page_id).into_iter().map(Object::Reference));
    let ours = format!("Q\n{}", line_content(font, &embed, line, opt, place));
    contents.push(doc.add_object(Stream::new(Dictionary::new(), ours.into_bytes())).into());

    let page = doc.get_dictionary_mut(page_id)?;
    page.set("Resources", resources);
    page.set("Contents", contents);
    // Compress what we added; leave every existing stream exactly as it was read.
    for (id, object) in doc.objects.iter_mut() {
        if let (true, Object::Stream(stream)) = (id.0 >= first_new_id, object) {
            let _ = stream.compress();
        }
    }

    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

fn line_content(
    font: &FontInfo,
    embed: &Embed,
    line: &ShapedLine,
    opt: Options,
    place: &Placement,
) -> String {
    let size = place.size;
    let mut out = format!(
        "BT\n/{} {} Tf\n1 0 0 1 {} {} Tm\n",
        place.resource,
        num(size),
        num(place.x),
        num(place.baseline)
    );
    let mut w = Emitter::new(opt.positioning, size, font.upem as f64);
    let pens = line.pen_positions();
    let mut open_reversed: Option<usize> = None;

    for seg in plan_segments(line, embed, opt.actual_text) {
        if opt.reversed_chars {
            let want = (seg.rtl && seg.run != usize::MAX).then_some(seg.run);
            if open_reversed != want {
                w.flush(&mut out);
                if open_reversed.is_some() {
                    out += "EMC\n";
                }
                if want.is_some() {
                    out += "/ReversedChars BMC\n";
                }
                open_reversed = want;
            }
        }
        if let Some(text) = &seg.actual_text {
            let text = match opt.probe {
                None => text.clone(),
                Some(Probe::Short) => "PROBE".to_string(),
                Some(Probe::OnePerGlyph) => "x".repeat(seg.glyphs.len()),
            };
            w.flush(&mut out);
            out += &format!("/Span <</ActualText <{}>>> BDC\n", utf16be_hex(&text));
        }
        for i in seg.glyphs.clone() {
            let g = &line.glyphs[i];
            let cid = embed.cid(g.gid);
            w.glyph(
                &mut out,
                cid,
                (pens[i] + g.x_offset) as f64,
                g.y_offset as f64,
                embed.widths[cid as usize],
            );
        }
        if seg.actual_text.is_some() {
            w.flush(&mut out);
            out += "EMC\n";
        }
    }
    w.flush(&mut out);
    if open_reversed.is_some() {
        out += "EMC\n";
    }
    w.finish(&mut out);
    out += "ET\n";
    out
}

/// Turns "glyph at (x, y) font units from the line origin" into operators, tracking
/// where the viewer's own pen is after each glyph (it advances by the /W width).
struct Emitter {
    mode: Positioning,
    size: f64,
    upem: f64,
    /// Glyph IDs of the string being built.
    pending: String,
    in_array: bool,
    /// Tj mode: viewer pen x in 1/1000 em from the line origin, and current rise.
    cursor_k: f64,
    rise: f64,
    /// Td mode: text line origin and viewer pen, in points from the initial Tm.
    line_x: f64,
    line_y: f64,
    pen_x: f64,
}

impl Emitter {
    fn new(mode: Positioning, size: f64, upem: f64) -> Self {
        Self {
            mode,
            size,
            upem,
            pending: String::new(),
            in_array: false,
            cursor_k: 0.0,
            rise: 0.0,
            line_x: 0.0,
            line_y: 0.0,
            pen_x: 0.0,
        }
    }

    fn glyph(&mut self, out: &mut String, cid: u16, x: f64, y: f64, width_k: f64) {
        match self.mode {
            Positioning::Tj => {
                if y != self.rise {
                    self.flush(out);
                    *out += &format!("{} Ts\n", num(y / self.upem * self.size));
                    self.rise = y;
                }
                // A TJ number n moves the pen left by n/1000 em.
                let adjust = round3(self.cursor_k - x * 1000.0 / self.upem);
                if adjust != 0.0 {
                    self.close_string(out);
                    self.open_array(out);
                    *out += &format!("{} ", num(adjust));
                    self.cursor_k -= adjust;
                }
                self.open_array(out);
                self.pending += &format!("{cid:04X}");
                self.cursor_k += width_k;
            }
            Positioning::Td => {
                let (px, py) = (x / self.upem * self.size, y / self.upem * self.size);
                if (px - self.pen_x).abs() > 0.0005 || py != self.line_y {
                    self.flush(out);
                    *out += &format!("{} {} Td\n", num(px - self.line_x), num(py - self.line_y));
                    (self.line_x, self.line_y, self.pen_x) = (px, py, px);
                }
                self.pending += &format!("{cid:04X}");
                self.pen_x += width_k / 1000.0 * self.size;
            }
        }
    }

    fn open_array(&mut self, out: &mut String) {
        if !self.in_array {
            *out += "[";
            self.in_array = true;
        }
    }

    fn close_string(&mut self, out: &mut String) {
        if !self.pending.is_empty() {
            *out += &format!("<{}>", self.pending);
            self.pending.clear();
        }
    }

    /// Ends the current show operator (needed before any marked-content operator).
    fn flush(&mut self, out: &mut String) {
        match self.mode {
            Positioning::Tj => {
                self.close_string(out);
                if self.in_array {
                    *out += "] TJ\n";
                    self.in_array = false;
                }
            }
            Positioning::Td => {
                if !self.pending.is_empty() {
                    self.close_string(out);
                    *out += " Tj\n";
                }
            }
        }
    }

    fn finish(&mut self, out: &mut String) {
        self.flush(out);
        if self.mode == Positioning::Tj && self.rise != 0.0 {
            *out += "0 Ts\n";
        }
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

pub fn num(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.to_string() }
}
