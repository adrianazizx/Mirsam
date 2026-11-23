//! Taking redacted text out of everything else in the file (ADR 0042):
//! document information, XMP metadata, outline titles, annotation text, form
//! values, structure-tree alternates. Anything that is a text string, or the
//! XMP packet, is searched for the redacted strings and the matches are
//! replaced. Matching is the search engine's, so an Arabic string matches
//! through tashkeel, alef variants, and digit styles.

use super::overlay::Overlay;
use crate::pdf::{Dict, Object, decode_stream};
use crate::search::{Query, SearchOptions};

pub(crate) const REPLACEMENT: &str = "[redacted]";

pub(crate) struct Scrubber {
    queries: Vec<Query>,
    pub(crate) replaced: usize,
}

impl Scrubber {
    /// `None` when there is nothing to look for.
    pub(crate) fn new(texts: &[String]) -> Option<Self> {
        let options = SearchOptions { case_sensitive: true, ..SearchOptions::default() };
        let mut queries = Vec::new();
        for text in texts {
            let text = text.trim();
            // A single character would also be found in unrelated strings.
            if text.chars().count() >= 2
                && let Ok(query) = Query::new(text, options.clone())
            {
                queries.push(query);
            }
        }
        (!queries.is_empty()).then_some(Self { queries, replaced: 0 })
    }

    /// `text` with every match replaced; `None` when there is none.
    fn clean(&mut self, text: &str) -> Option<String> {
        let mut ranges: Vec<(usize, usize)> = self.queries.iter().flat_map(|q| q.find_in(text)).collect();
        if ranges.is_empty() {
            return None;
        }
        ranges.sort_unstable();
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for (start, end) in ranges {
            if start < at {
                continue;
            }
            out.push_str(&text[at..start]);
            out.push_str(REPLACEMENT);
            at = end;
            self.replaced += 1;
        }
        out.push_str(&text[at..]);
        Some(out)
    }

    /// A PDF text string with its matches replaced. Only strings that are
    /// text are touched: UTF-16BE or UTF-8 with a byte-order mark, or plain
    /// ASCII. (A binary string, such as a palette, is never rewritten.)
    fn string(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
            let (pairs, _) = rest.as_chunks::<2>();
            let units: Vec<u16> = pairs.iter().map(|p| u16::from_be_bytes(*p)).collect();
            let text = String::from_utf16(&units).ok()?;
            let clean = self.clean(&text)?;
            let mut out = vec![0xFE, 0xFF];
            for unit in clean.encode_utf16() {
                out.extend_from_slice(&unit.to_be_bytes());
            }
            Some(out)
        } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
            let clean = self.clean(std::str::from_utf8(rest).ok()?)?;
            let mut out = vec![0xEF, 0xBB, 0xBF];
            out.extend_from_slice(clean.as_bytes());
            Some(out)
        } else if bytes.iter().all(|b| matches!(b, 9 | 10 | 13 | 0x20..=0x7E)) {
            self.clean(std::str::from_utf8(bytes).ok()?).map(String::into_bytes)
        } else {
            None
        }
    }

    fn object(&mut self, object: &mut Object) -> bool {
        match object {
            Object::Str(bytes) => match self.string(bytes) {
                Some(clean) => {
                    *bytes = clean;
                    true
                }
                None => false,
            },
            Object::Array(items) => items.iter_mut().fold(false, |any, item| self.object(item) | any),
            Object::Dict(dict) => self.dict(dict),
            _ => false,
        }
    }

    fn dict(&mut self, dict: &mut Dict) -> bool {
        dict.0.iter_mut().fold(false, |any, (_, value)| self.object(value) | any)
    }
}

/// Scrubs every object of the file. Returns how many strings were replaced.
pub(crate) fn scrub_all(overlay: &mut Overlay<'_, '_>, texts: &[String]) -> usize {
    let Some(mut scrubber) = Scrubber::new(texts) else { return 0 };
    for number in overlay.numbers() {
        let Some((mut object, data)) = overlay.get(number) else { continue };
        let generation = overlay.generation(number);
        match data {
            None => {
                if scrubber.object(&mut object) {
                    overlay.put(number, generation, object);
                }
            }
            Some(data) => {
                let Object::Dict(mut dict) = object else { continue };
                let is_xmp = dict.get(b"Type").and_then(Object::as_name) == Some(b"Metadata")
                    || dict.get(b"Subtype").and_then(Object::as_name) == Some(b"XML");
                let mut changed = scrubber.dict(&mut dict);
                let mut data = data;
                if is_xmp {
                    // Held streams are written as given; a stream read from
                    // the file may be compressed.
                    let plain = decode_stream(&dict, &data, overlay.file)
                        .or_else(|| dict.get(b"Filter").is_none().then(|| data.clone()));
                    if let Some(plain) = plain
                        && let Ok(text) = String::from_utf8(plain)
                        && let Some(clean) = scrubber.clean(&text)
                    {
                        data = clean.into_bytes();
                        dict.0.retain(|(k, _)| !matches!(k.as_slice(), b"Filter" | b"DecodeParms"));
                        changed = true;
                    }
                }
                if changed {
                    overlay.put_stream(number, generation, dict, data);
                }
            }
        }
    }
    scrubber.replaced
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrubber(texts: &[&str]) -> Scrubber {
        Scrubber::new(&texts.iter().map(|t| t.to_string()).collect::<Vec<_>>()).expect("something to find")
    }

    #[test]
    fn plain_and_utf16_strings_are_cleaned() {
        let mut scrubber = scrubber(&["Jane Roe"]);
        assert_eq!(scrubber.string(b"Memo by Jane Roe, 2026"), Some(b"Memo by [redacted], 2026".to_vec()));
        assert_eq!(scrubber.string(b"nothing here"), None);
        let mut utf16 = vec![0xFE, 0xFF];
        for unit in "Jane Roe".encode_utf16() {
            utf16.extend_from_slice(&unit.to_be_bytes());
        }
        let clean = scrubber.string(&utf16).expect("cleaned");
        assert_eq!(&clean[..2], &[0xFE, 0xFF]);
        assert_eq!(clean.len(), 2 + 2 * REPLACEMENT.len());
    }

    #[test]
    fn arabic_matches_through_digit_styles_and_marks() {
        let mut scrubber = scrubber(&["رقم 12345"]);
        let mut text = vec![0xFE, 0xFF];
        for unit in "الرقم ١٢٣٤٥".encode_utf16().chain(" ok".encode_utf16()) {
            text.extend_from_slice(&unit.to_be_bytes());
        }
        // "رقم ١٢٣٤٥" (Arabic-Indic digits) is the same number.
        assert!(scrubber.string(&text).is_some());
    }

    #[test]
    fn binary_strings_are_left_alone() {
        let mut scrubber = scrubber(&["ab"]);
        assert_eq!(scrubber.string(&[0x00, b'a', b'b', 0xFF]), None);
        assert_eq!(scrubber.string(&[0xFF, 0x00]), None);
    }

    #[test]
    fn nothing_to_find_is_no_scrubber() {
        assert!(Scrubber::new(&[]).is_none());
        assert!(Scrubber::new(&["x".to_string(), "  ".to_string()]).is_none());
    }
}
