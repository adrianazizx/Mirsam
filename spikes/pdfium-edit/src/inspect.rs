//! Reads a generated PDF back with lopdf (independent of the code that wrote it)
//! to measure what actually ended up in the file.

use std::collections::BTreeMap;

use lopdf::content::Content;
use lopdf::{Document, Object};

#[derive(Debug, Default)]
pub struct Inspection {
    pub file_size: usize,
    /// Decoded size of every embedded font program (FontFile2 / FontFile3).
    pub font_programs: Vec<usize>,
    /// Charcode -> text, from the ToUnicode CMap(s).
    pub to_unicode: BTreeMap<u32, String>,
    pub has_to_unicode: bool,
    /// "Identity", "stream(N bytes)" or "absent".
    pub cid_to_gid: String,
    pub font_subtypes: Vec<String>,
    /// Per page: decoded content stream size and operator counts.
    pub pages: Vec<PageOps>,
}

#[derive(Debug, Default, Clone)]
pub struct PageOps {
    pub content_bytes: usize,
    pub show_ops: usize,
    pub text_objects: usize,
    pub marked_spans: usize,
    pub content: String,
}

pub fn inspect(bytes: &[u8]) -> Result<Inspection, Box<dyn std::error::Error>> {
    let doc = Document::load_mem(bytes)?;
    let mut out = Inspection { file_size: bytes.len(), cid_to_gid: "absent".into(), ..Default::default() };

    for object in doc.objects.values() {
        let Object::Dictionary(dict) = object else { continue };
        for key in [b"FontFile2".as_slice(), b"FontFile3", b"FontFile"] {
            if let Ok(Object::Stream(stream)) = dict.get_deref(key, &doc) {
                out.font_programs.push(decoded(stream).len());
            }
        }
        if dict.has_type(b"Font") {
            if let Ok(subtype) = dict.get(b"Subtype").and_then(Object::as_name) {
                out.font_subtypes.push(String::from_utf8_lossy(subtype).into_owned());
            }
            if let Ok(Object::Stream(stream)) = dict.get_deref(b"ToUnicode", &doc) {
                out.has_to_unicode = true;
                parse_to_unicode(&String::from_utf8_lossy(&decoded(stream)), &mut out.to_unicode);
            }
            match dict.get_deref(b"CIDToGIDMap", &doc) {
                Ok(Object::Name(name)) => out.cid_to_gid = String::from_utf8_lossy(name).into_owned(),
                Ok(Object::Stream(stream)) => {
                    out.cid_to_gid = format!("stream({} bytes)", decoded(stream).len())
                }
                _ => {}
            }
        }
    }

    for page_id in doc.get_pages().values() {
        let content = doc.get_page_content(*page_id);
        let mut page = PageOps {
            content_bytes: content.len(),
            content: String::from_utf8_lossy(&content).into_owned(),
            ..Default::default()
        };
        for op in Content::decode(&content)?.operations {
            match op.operator.as_str() {
                "Tj" | "TJ" | "'" | "\"" => page.show_ops += 1,
                "BT" => page.text_objects += 1,
                "BDC" | "BMC" => page.marked_spans += 1,
                _ => {}
            }
        }
        out.pages.push(page);
    }
    Ok(out)
}

fn decoded(stream: &lopdf::Stream) -> Vec<u8> {
    stream.decompressed_content().unwrap_or_else(|_| stream.content.clone())
}

fn hex_units(hex: &str) -> Vec<u16> {
    let digits: Vec<u32> = hex.chars().filter_map(|c| c.to_digit(16)).collect();
    digits.chunks(4).map(|c| c.iter().fold(0u32, |acc, d| acc * 16 + d) as u16).collect()
}

/// Minimal bfchar / bfrange reader: enough for the CMaps PDFium and we produce.
fn parse_to_unicode(cmap: &str, out: &mut BTreeMap<u32, String>) {
    // Tokens: <hex>, [, ], and bare words.
    let mut tokens: Vec<String> = Vec::new();
    let mut chars = cmap.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' if chars.peek() != Some(&'<') => {
                let hex: String = chars.by_ref().take_while(|c| *c != '>').collect();
                tokens.push(format!("<{hex}"));
            }
            '[' | ']' => tokens.push(c.to_string()),
            c if c.is_whitespace() || c == '<' || c == '>' => {}
            c => {
                let mut word = c.to_string();
                while let Some(n) = chars.peek() {
                    if n.is_whitespace() || "<>[]".contains(*n) {
                        break;
                    }
                    word.push(*n);
                    chars.next();
                }
                tokens.push(word);
            }
        }
    }

    let code = |t: &str| u32::from_str_radix(t.trim_start_matches('<'), 16).unwrap_or(0);
    let text = |units: &[u16]| String::from_utf16_lossy(units);
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "beginbfchar" => {
                i += 1;
                while i + 1 < tokens.len() && tokens[i] != "endbfchar" {
                    out.insert(code(&tokens[i]), text(&hex_units(&tokens[i + 1][1..])));
                    i += 2;
                }
            }
            "beginbfrange" => {
                i += 1;
                while i + 2 < tokens.len() && tokens[i] != "endbfrange" {
                    let (lo, hi) = (code(&tokens[i]), code(&tokens[i + 1]));
                    if tokens[i + 2] == "[" {
                        let mut j = i + 3;
                        let mut c = lo;
                        while j < tokens.len() && tokens[j] != "]" {
                            out.insert(c, text(&hex_units(&tokens[j][1..])));
                            c += 1;
                            j += 1;
                        }
                        i = j + 1;
                    } else {
                        let base = hex_units(&tokens[i + 2][1..]);
                        for c in lo..=hi {
                            let mut units = base.clone();
                            if let Some(last) = units.last_mut() {
                                *last = last.wrapping_add((c - lo) as u16);
                            }
                            out.insert(c, text(&units));
                        }
                        i += 3;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
}
