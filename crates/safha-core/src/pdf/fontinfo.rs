//! What editing needs to know about a font a page already uses (M3, ADR 0023):
//! how its codes read as Unicode, how wide they are, and whether new text can
//! be written with it.
//!
//! Read from the font dictionary, never from the font program: `/ToUnicode`
//! (or the simple font's encoding), `/Widths` or `/W`, and the descriptor's
//! flags and name. Supported: simple fonts (one byte per code) and Type 0
//! fonts with an Identity encoding (two bytes). Anything else is read as
//! unusable, and text in it is not edited.

use std::collections::HashMap;

use super::crypt::Crypt;
use super::{Dict, File, Object, decode_stream};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontKind {
    Simple,
    /// Type 0 with an Identity-H or Identity-V encoding: two bytes per code.
    Identity,
    /// Type 3, or an encoding this reader does not follow.
    Unsupported,
}

#[derive(Debug, Clone)]
pub(crate) struct FontInfo {
    /// `/BaseFont` without its subset tag.
    pub(crate) name: String,
    pub(crate) kind: FontKind,
    pub(crate) vertical: bool,
    pub(crate) embedded: bool,
    pub(crate) serif: bool,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) fixed: bool,
    first_char: u32,
    widths: Vec<f64>,
    cid_widths: HashMap<u32, f64>,
    default_width: f64,
    unicode: HashMap<u32, String>,
    /// Text → code, for the codes that stand for exactly one character.
    reverse: HashMap<char, u32>,
    /// Where the embedded font program is (`/FontFile2`, or `/FontFile3`),
    /// when it has one (ADR 0026).
    program: Option<Object>,
}

impl FontInfo {
    /// Reads a font dictionary; `None` if it is not one.
    pub(crate) fn read(file: &File<'_>, dict: &Dict) -> Option<FontInfo> {
        Self::read_in(file, dict, None)
    }

    /// Reads a font dictionary of a file encrypted with `crypt` (its
    /// `/ToUnicode` stream is decrypted to be read).
    pub(crate) fn read_in(file: &File<'_>, dict: &Dict, crypt: Option<&Crypt>) -> Option<FontInfo> {
        let subtype = dict.get(b"Subtype").and_then(|s| file.resolve(s))?;
        let subtype = subtype.as_name()?.to_vec();
        let resolve = |key: &[u8]| dict.get(key).and_then(|v| file.resolve(v));
        let base_name = resolve(b"BaseFont")
            .and_then(|n| n.as_name().map(|n| String::from_utf8_lossy(n).into_owned()))
            .unwrap_or_default();
        let name = match base_name.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => {
                rest.to_string()
            }
            _ => base_name,
        };
        let mut info = FontInfo {
            name,
            kind: FontKind::Unsupported,
            vertical: false,
            embedded: false,
            serif: false,
            bold: false,
            italic: false,
            fixed: false,
            first_char: 0,
            widths: Vec::new(),
            cid_widths: HashMap::new(),
            default_width: 1000.0,
            unicode: HashMap::new(),
            reverse: HashMap::new(),
            program: None,
        };

        let mut descriptor = resolve(b"FontDescriptor").and_then(|d| d.as_dict().cloned());
        match subtype.as_slice() {
            b"Type0" => {
                let encoding = resolve(b"Encoding");
                match encoding.as_ref().and_then(Object::as_name) {
                    Some(b"Identity-H") => info.kind = FontKind::Identity,
                    Some(b"Identity-V") => {
                        info.kind = FontKind::Identity;
                        info.vertical = true;
                    }
                    _ => {}
                }
                let descendant = resolve(b"DescendantFonts")
                    .and_then(|d| d.as_array().and_then(|a| a.first().cloned()))
                    .and_then(|d| file.resolve(&d))
                    .and_then(|d| d.as_dict().cloned());
                if let Some(cid) = descendant {
                    descriptor = cid.get(b"FontDescriptor").and_then(|d| file.resolve(d))?.as_dict().cloned();
                    if let Some(dw) = cid.get(b"DW").and_then(|v| file.resolve(v)).and_then(number) {
                        info.default_width = dw;
                    }
                    if let Some(w) = cid.get(b"W").and_then(|v| file.resolve(v)) {
                        read_cid_widths(file, &w, &mut info.cid_widths);
                    }
                }
            }
            b"Type3" => {}
            _ => {
                info.kind = FontKind::Simple;
                info.first_char = resolve(b"FirstChar").and_then(|v| v.as_int()).unwrap_or(0).max(0) as u32;
                if let Some(Object::Array(items)) = resolve(b"Widths") {
                    info.widths =
                        items.iter().map(|w| file.resolve(w).and_then(number).unwrap_or(0.0)).collect();
                } else {
                    info.default_width = 500.0; // a standard font without /Widths
                }
                let missing = descriptor
                    .as_ref()
                    .and_then(|d| d.get(b"MissingWidth"))
                    .and_then(|v| file.resolve(v))
                    .and_then(number);
                if !info.widths.is_empty() {
                    info.default_width = missing.unwrap_or(0.0);
                }
            }
        }

        if let Some(d) = &descriptor {
            let flags = d.get(b"Flags").and_then(|f| file.resolve(f)).and_then(|f| f.as_int()).unwrap_or(0);
            info.fixed = flags & 1 != 0;
            info.serif = flags & 2 != 0;
            info.italic = flags & 64 != 0;
            info.bold = flags & 0x4_0000 != 0;
            info.embedded =
                [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"].iter().any(|k| d.get(k).is_some());
            // TrueType, or OpenType (CFF or not) in `/FontFile3`; a Type 1
            // program or bare CFF does not load as a face and is not used.
            info.program = d.get(b"FontFile2").or_else(|| d.get(b"FontFile3")).cloned();
        }
        let lower = info.name.to_lowercase();
        info.bold |= lower.contains("bold") || lower.contains("black") || lower.contains("heavy");
        info.italic |= lower.contains("italic") || lower.contains("oblique");
        info.serif |=
            ["times", "georgia", "garamond", "palatino", "cambria", "naskh", "amiri", "roman", "serif"]
                .iter()
                .any(|s| lower.contains(s))
                && !lower.contains("sans");

        // How codes read as text: /ToUnicode first.
        if let Some(stream) = dict.get(b"ToUnicode")
            && let Some(data) = stream_data_in(file, stream, crypt)
        {
            parse_to_unicode(&data, &mut info.unicode);
        }
        if info.unicode.is_empty() && info.kind == FontKind::Simple {
            simple_encoding(file, dict, &mut info.unicode);
        }
        for (code, text) in &info.unicode {
            let mut chars = text.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                info.reverse.entry(ch).and_modify(|c| *c = (*c).min(*code)).or_insert(*code);
            }
        }
        Some(info)
    }

    /// The font's codes in `bytes`, each with its length in bytes.
    pub(crate) fn codes<'a>(&self, bytes: &'a [u8]) -> impl Iterator<Item = (u32, usize)> + 'a {
        let two = self.kind == FontKind::Identity;
        let mut at = 0;
        std::iter::from_fn(move || {
            if two {
                let pair = bytes.get(at..at + 2)?;
                at += 2;
                Some((u32::from(pair[0]) << 8 | u32::from(pair[1]), 2))
            } else {
                let byte = *bytes.get(at)?;
                at += 1;
                Some((u32::from(byte), 1))
            }
        })
    }

    /// A code's advance in thousandths of an em.
    pub(crate) fn width(&self, code: u32) -> f64 {
        match self.kind {
            FontKind::Simple => code
                .checked_sub(self.first_char)
                .and_then(|i| self.widths.get(i as usize).copied())
                .unwrap_or(self.default_width),
            _ => self.cid_widths.get(&code).copied().unwrap_or(self.default_width),
        }
    }

    pub(crate) fn text(&self, code: u32) -> Option<&str> {
        self.unicode.get(&code).map(String::as_str)
    }

    /// Advances of its letters and digits that do not join, in thousandths of
    /// an em, from `/Widths` (or `/W`) through its codes' text: what a
    /// substitute should come close to. Arabic and Hebrew letters are left
    /// out: a code's width there is one joined form's.
    pub(crate) fn letter_widths(&self) -> Vec<(char, f32)> {
        use unicode_bidi::{BidiClass, bidi_class};
        let mut widths: Vec<(char, f32)> = self
            .unicode
            .iter()
            .filter_map(|(&code, text)| {
                let mut chars = text.chars();
                let (Some(c), None) = (chars.next(), chars.next()) else { return None };
                let width = self.width(code) as f32;
                (c.is_alphanumeric() && !matches!(bidi_class(c), BidiClass::R | BidiClass::AL) && width > 0.0)
                    .then_some((c, width))
            })
            .collect();
        widths.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        widths.dedup_by_key(|(c, _)| *c);
        widths
    }

    /// A simple font with one code per character of `chars` (from 32 up), at
    /// those widths: what a test needs to stand for a page's font.
    #[cfg(test)]
    pub(crate) fn simple_for_test(name: &str, serif: bool, chars: &[(char, f64)]) -> FontInfo {
        let codes = (32u32..).zip(chars.iter().copied());
        FontInfo {
            name: name.to_string(),
            kind: FontKind::Simple,
            vertical: false,
            embedded: true,
            serif,
            bold: false,
            italic: false,
            fixed: false,
            first_char: 32,
            widths: chars.iter().map(|&(_, w)| w).collect(),
            cid_widths: HashMap::new(),
            default_width: 0.0,
            unicode: codes.clone().map(|(code, (c, _))| (code, c.to_string())).collect(),
            reverse: codes.map(|(code, (c, _))| (c, code)).collect(),
            program: None,
        }
    }

    /// The font's embedded program as a face to lay text out with, when it
    /// has one the layout engine can read (TrueType or OpenType), decrypted
    /// with `crypt` in an encrypted file.
    pub(crate) fn program_face(
        &self,
        file: &File<'_>,
        crypt: Option<&Crypt>,
    ) -> Option<crate::typeset::FontFace> {
        let data = stream_data_in(file, self.program.as_ref()?, crypt)?;
        crate::typeset::FontFace::from_bytes(data, 0).ok()
    }

    /// Whether text in this font can be read and written back.
    pub(crate) fn usable(&self) -> bool {
        self.kind != FontKind::Unsupported && !self.vertical && !self.unicode.is_empty()
    }

    /// The bytes that show `ch` in this font, if it has a code for it.
    pub(crate) fn encode(&self, ch: char) -> Option<Vec<u8>> {
        let code = *self.reverse.get(&ch)?;
        Some(match self.kind {
            FontKind::Simple => vec![u8::try_from(code).ok()?],
            _ => vec![(code >> 8) as u8, code as u8],
        })
    }
}

fn number(object: Object) -> Option<f64> {
    match object {
        Object::Int(i) => Some(i as f64),
        Object::Real(r) => Some(r),
        _ => None,
    }
}

/// `/W`: `c [w1 w2 …]` and `c_first c_last w`.
fn read_cid_widths(file: &File<'_>, w: &Object, out: &mut HashMap<u32, f64>) {
    let Some(items) = w.as_array() else { return };
    let mut i = 0;
    while i < items.len() {
        let Some(first) = file.resolve(&items[i]).and_then(|v| v.as_int()) else { return };
        match items.get(i + 1).and_then(|v| file.resolve(v)) {
            Some(Object::Array(list)) => {
                for (n, width) in list.iter().enumerate() {
                    if let Some(width) = file.resolve(width).and_then(number) {
                        out.insert((first as u32).saturating_add(n as u32), width);
                    }
                }
                i += 2;
            }
            Some(last) => {
                let (Some(last), Some(width)) =
                    (last.as_int(), items.get(i + 2).and_then(|v| file.resolve(v)).and_then(number))
                else {
                    return;
                };
                for code in (first as u32)..=(last as u32).min(first as u32 + 65_535) {
                    out.insert(code, width);
                }
                i += 3;
            }
            None => return,
        }
    }
}

/// The decoded data of a stream object (a reference to it); in a file
/// encrypted with `crypt`, decrypted under the stream's own number first.
pub(crate) fn stream_data_in(file: &File<'_>, object: &Object, crypt: Option<&Crypt>) -> Option<Vec<u8>> {
    let &Object::Ref(number, generation) = object else { return None };
    let super::Entry::Offset(offset) = *file.entries.get(&number)? else { return None };
    let (dict, data) = file.indirect_at(offset)?;
    match crypt {
        Some(crypt) => {
            let data = crypt.decrypt_stream_of(number, generation, &dict, data?)?;
            decode_stream(&dict, &data, file)
        }
        None => decode_stream(&dict, data?, file),
    }
}

/// `beginbfchar` / `beginbfrange` sections of a ToUnicode CMap.
fn parse_to_unicode(data: &[u8], out: &mut HashMap<u32, String>) {
    let text = String::from_utf8_lossy(data);
    let hex = |token: &str| -> Option<Vec<u8>> {
        let inner = token.trim().strip_prefix('<')?.strip_suffix('>')?;
        let digits: Vec<u8> = inner.bytes().filter(u8::is_ascii_hexdigit).collect();
        digits.chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()).collect()
    };
    let code = |bytes: &[u8]| bytes.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b));
    let utf16 = |bytes: &[u8]| -> Option<String> {
        let units: Vec<u16> =
            bytes.as_chunks::<2>().0.iter().map(|p| u16::from(p[0]) << 8 | u16::from(p[1])).collect();
        String::from_utf16(&units).ok()
    };
    // Tokens: `<…>` groups and the `[`, `]` of range arrays.
    let mut section: Option<&str> = None;
    let mut line_tokens: Vec<String> = Vec::new();
    let flush = |section: Option<&str>, tokens: &mut Vec<String>, out: &mut HashMap<u32, String>| {
        match section {
            Some("char") if tokens.len() >= 2 => {
                if let (Some(src), Some(dst)) = (hex(&tokens[0]), hex(&tokens[1]))
                    && let Some(text) = utf16(&dst)
                {
                    out.insert(code(&src), text);
                }
            }
            Some("range") if tokens.len() >= 3 => {
                if let (Some(lo), Some(hi)) = (hex(&tokens[0]), hex(&tokens[1])) {
                    let (lo, hi) = (code(&lo), code(&hi).min(code(&lo).saturating_add(65_535)));
                    if tokens[2] == "[" {
                        for (n, item) in tokens[3..].iter().filter(|t| t.starts_with('<')).enumerate() {
                            if let Some(text) = hex(item).and_then(|d| utf16(&d)) {
                                out.insert(lo + n as u32, text);
                            }
                        }
                    } else if let Some(dst) = hex(&tokens[2])
                        && let Some(start) = utf16(&dst)
                    {
                        let mut chars: Vec<char> = start.chars().collect();
                        for c in lo..=hi {
                            out.insert(c, chars.iter().collect());
                            if let Some(last) = chars.last_mut() {
                                *last = char::from_u32(u32::from(*last) + 1).unwrap_or(*last);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        tokens.clear();
    };
    for line in text.lines() {
        let line = line.trim();
        if line.ends_with("beginbfchar") {
            section = Some("char");
        } else if line.ends_with("beginbfrange") {
            section = Some("range");
        } else if line.starts_with("endbf") {
            section = None;
        } else if section.is_some() {
            let mut tokens: Vec<String> = Vec::new();
            let mut rest = line;
            while let Some(c) = rest.chars().next() {
                if c == '<' {
                    let Some(end) = rest.find('>') else { break };
                    tokens.push(rest[..=end].to_string());
                    rest = &rest[end + 1..];
                } else if c == '[' || c == ']' {
                    tokens.push(c.to_string());
                    rest = &rest[1..];
                } else {
                    rest = &rest[c.len_utf8()..];
                }
            }
            line_tokens.clear();
            line_tokens.extend(tokens);
            flush(section, &mut line_tokens, out);
        }
    }
}

/// A simple font's code → text from its base encoding and `/Differences`.
fn simple_encoding(file: &File<'_>, dict: &Dict, out: &mut HashMap<u32, String>) {
    let encoding = dict.get(b"Encoding").and_then(|e| file.resolve(e));
    let (base, differences) = match &encoding {
        Some(Object::Name(name)) => (name.clone(), None),
        Some(Object::Dict(d)) => (
            d.get(b"BaseEncoding")
                .and_then(|b| file.resolve(b))
                .and_then(|b| b.as_name().map(<[u8]>::to_vec))
                .unwrap_or_else(|| b"StandardEncoding".to_vec()),
            d.get(b"Differences").and_then(|v| file.resolve(v)),
        ),
        _ => (b"StandardEncoding".to_vec(), None),
    };
    for code in 0x20u32..=0xFF {
        let ch = match base.as_slice() {
            b"WinAnsiEncoding" => win_ansi(code),
            _ if code < 0x7F => char::from_u32(code),
            _ => None,
        };
        if let Some(ch) = ch {
            out.insert(code, ch.to_string());
        }
    }
    if let Some(Object::Array(items)) = differences {
        let mut code = 0u32;
        for item in items {
            match file.resolve(&item) {
                Some(Object::Int(n)) => code = n.max(0) as u32,
                Some(Object::Name(name)) => {
                    match glyph_char(&name) {
                        Some(ch) => out.insert(code, ch.to_string()),
                        None => out.remove(&code),
                    };
                    code += 1;
                }
                _ => {}
            }
        }
    }
}

fn win_ansi(code: u32) -> Option<char> {
    const HIGH: [char; 32] = [
        '€', '\u{fffd}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{fffd}', 'Ž', '\u{fffd}',
        '\u{fffd}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{fffd}', 'ž', 'Ÿ',
    ];
    match code {
        0x20..=0x7E | 0xA0..=0xFF => char::from_u32(code),
        0x80..=0x9F => Some(HIGH[(code - 0x80) as usize]).filter(|c| *c != '\u{fffd}'),
        _ => None,
    }
}

/// The character a standard glyph name stands for: single letters, the
/// spelled-out digits, common punctuation, and `uniXXXX`.
fn glyph_char(name: &[u8]) -> Option<char> {
    let name = std::str::from_utf8(name).ok()?;
    if name.len() == 1 {
        return name.chars().next().filter(char::is_ascii_alphabetic);
    }
    if let Some(hex) = name.strip_prefix("uni").filter(|h| h.len() == 4) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    Some(match name {
        "space" => ' ',
        "zero" => '0',
        "one" => '1',
        "two" => '2',
        "three" => '3',
        "four" => '4',
        "five" => '5',
        "six" => '6',
        "seven" => '7',
        "eight" => '8',
        "nine" => '9',
        "period" => '.',
        "comma" => ',',
        "colon" => ':',
        "semicolon" => ';',
        "hyphen" | "minus" => '-',
        "endash" => '–',
        "emdash" => '—',
        "quotesingle" => '\'',
        "quotedbl" => '"',
        "quoteright" => '’',
        "quoteleft" => '‘',
        "quotedblleft" => '“',
        "quotedblright" => '”',
        "parenleft" => '(',
        "parenright" => ')',
        "slash" => '/',
        "question" => '?',
        "exclam" => '!',
        "percent" => '%',
        "ampersand" => '&',
        "at" => '@',
        "dollar" => '$',
        "plus" => '+',
        "equal" => '=',
        "underscore" => '_',
        "asterisk" => '*',
        "numbersign" => '#',
        "fi" => 'ﬁ',
        "fl" => 'ﬂ',
        "bullet" => '•',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_to_unicode_map_reads_chars_and_ranges() {
        let cmap = b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
            2 beginbfchar\n<0003> <0644>\n<0004> <00660066>\nendbfchar\n\
            2 beginbfrange\n<0010> <0012> <0041>\n<0020> <0021> [<0062> <0063>]\nendbfrange\n";
        let mut map = HashMap::new();
        parse_to_unicode(cmap, &mut map);
        assert_eq!(map.get(&3).map(String::as_str), Some("ل"));
        assert_eq!(map.get(&4).map(String::as_str), Some("ff"));
        assert_eq!(map.get(&0x11).map(String::as_str), Some("B"));
        assert_eq!(map.get(&0x12).map(String::as_str), Some("C"));
        assert_eq!(map.get(&0x21).map(String::as_str), Some("c"));
    }

    #[test]
    fn win_ansi_has_the_cp1252_extras() {
        assert_eq!(win_ansi(0x80), Some('€'));
        assert_eq!(win_ansi(0x41), Some('A'));
        assert_eq!(win_ansi(0x81), None);
        assert_eq!(glyph_char(b"uni0644"), Some('ل'));
    }
}
