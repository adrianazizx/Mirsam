//! Font matching (spec §7, step 4): when an edited paragraph's own font cannot
//! be used, pick the closest font we have — by family name first, then by
//! classification (serif / sans / monospace) and style (weight, italic) — and
//! say how close it is, so the editor can show "Font substituted: …".
//!
//! The text decides first: a font that cannot draw every character of the
//! paragraph is never preferred over one that can, because the paragraph is
//! re-set in a single font for visual consistency.

use read_fonts::TableProvider;
use read_fonts::model::attribute::{Attributes, Style};
use read_fonts::types::NameId;

use super::FontFace;

/// What a font is, as far as matching cares.
#[derive(Debug, Clone, PartialEq)]
pub struct FontTraits {
    /// Typographic family name, e.g. "IBM Plex Sans Arabic".
    pub family: String,
    /// PostScript name, e.g. "IBMPlexSansArabic-SemiBold".
    pub postscript_name: Option<String>,
    /// 100 (thin) to 900 (black); 400 is regular, 700 bold.
    pub weight: f32,
    pub italic: bool,
    /// `None` when neither the font nor its name says.
    pub serif: Option<bool>,
    pub monospace: bool,
}

/// What we know about the font a PDF paragraph was set in, from its font
/// dictionary and descriptor. Everything but the name is optional.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontRequest {
    /// `/BaseFont`, as written: "ABCDEF+Amiri-Bold", "Arial,BoldItalic",
    /// "TimesNewRomanPS-BoldMT".
    pub name: String,
    /// `/FontFamily`, when the descriptor has one.
    pub family: Option<String>,
    /// `/FontWeight`, or a weight implied by the name.
    pub weight: Option<f32>,
    pub italic: Option<bool>,
    pub serif: Option<bool>,
    pub monospace: Option<bool>,
}

/// How good a match is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// Same family, same weight and slant: not a substitution.
    Exact,
    /// Same family, another style (the bold was asked for, only regular is here).
    SameFamily,
    /// Another family, chosen by classification and style.
    Substitute,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontMatch {
    /// Index into the candidates.
    pub index: usize,
    pub kind: MatchKind,
    /// Whether the font has every character of the text.
    pub covers_text: bool,
}

impl FontFace {
    /// The font's family, style and classification.
    pub fn traits(&self) -> FontTraits {
        let font = self.shaper_font();
        let names = font.names();
        let name =
            |id: NameId| names.get(id).and_then(|n| n.english_or_first()).map(|l| l.name().to_string());
        let family =
            name(NameId::TYPOGRAPHIC_FAMILY_NAME).or_else(|| name(NameId::FAMILY_NAME)).unwrap_or_default();
        let postscript_name = name(NameId::POSTSCRIPT_NAME);

        let attributes = Attributes::from_variations(font, self.variation_list().iter().copied());
        let italic = !matches!(attributes.style, Style::Normal);
        let tables = font.tables();
        let os2 = tables.os2().ok();
        let monospace = tables.post().is_ok_and(|p| p.is_fixed_pitch() != 0)
            || os2.as_ref().is_some_and(|o| o.panose_10().get(3) == Some(&9));
        // PANOSE (Latin text, family kind 2): serif styles 2–10, sans 11–13.
        // IBM's class: 1–7 and 10 are serif families, 8 sans serif.
        let serif = os2
            .as_ref()
            .and_then(|o| {
                let panose = o.panose_10();
                let by_panose = match (panose.first(), panose.get(1)) {
                    (Some(2), Some(2..=10)) => Some(true),
                    (Some(2), Some(11..=13)) => Some(false),
                    _ => None,
                };
                by_panose.or(match o.s_family_class() >> 8 {
                    1..=7 | 10 => Some(true),
                    8 => Some(false),
                    _ => None,
                })
            })
            .or_else(|| known_class(&family_key(&family)).map(|c| c == Class::Serif));
        FontTraits { family, postscript_name, weight: attributes.weight.value(), italic, serif, monospace }
    }
}

impl FontRequest {
    /// A request from a PDF font: `/BaseFont`, and from the descriptor
    /// `/FontFamily`, `/FontWeight`, `/Flags` and `/ItalicAngle`, where present.
    /// Style words in the name ("Bold", "Italic", "SemiBold") fill what the
    /// descriptor leaves out.
    pub fn from_pdf(
        base_font: &str,
        family: Option<&str>,
        font_weight: Option<f32>,
        flags: Option<u32>,
        italic_angle: Option<f32>,
    ) -> Self {
        let parsed = parse_name(base_font);
        FontRequest {
            name: base_font.to_string(),
            family: family.map(str::to_string),
            weight: font_weight.filter(|w| (100.0..=900.0).contains(w)).or(parsed.weight),
            italic: match (flags, italic_angle) {
                (Some(f), _) if f & (1 << 6) != 0 => Some(true),
                (_, Some(a)) if a.abs() > 1.0 => Some(true),
                _ => parsed.italic.then_some(true).or(flags.map(|_| false)),
            },
            // Flags bit 2 is Serif. A clear bit is weak evidence (many producers
            // never set it), so it says nothing; the name table may.
            serif: flags.filter(|f| f & (1 << 1) != 0).map(|_| true),
            monospace: flags.map(|f| f & 1 != 0),
        }
    }
}

/// Picks the candidate closest to `request` that can draw `text`.
///
/// Order of preference: draws every character of `text`; same family; then
/// classification and style. Returns `None` only when there are no candidates.
pub fn match_font(request: &FontRequest, text: &str, candidates: &[FontFace]) -> Option<FontMatch> {
    let parsed = parse_name(&request.name);
    let wanted_keys: Vec<String> = request
        .family
        .iter()
        .map(|f| family_key(f))
        .chain(std::iter::once(parsed.family_key.clone()))
        .filter(|k| !k.is_empty())
        .collect();
    let wanted_weight = request.weight.or(parsed.weight).unwrap_or(400.0);
    let wanted_italic = request.italic.unwrap_or(parsed.italic);
    let wanted_serif =
        request.serif.or_else(|| wanted_keys.iter().find_map(|k| known_class(k)).map(|c| c == Class::Serif));
    let wanted_mono = request.monospace.unwrap_or(false)
        || wanted_keys.iter().any(|k| known_class(k) == Some(Class::Monospace));
    let needed: Vec<char> = text.chars().filter(|c| !c.is_whitespace() && !c.is_control()).collect();

    candidates
        .iter()
        .enumerate()
        .map(|(index, face)| {
            let traits = face.traits();
            let covered = needed.iter().filter(|c| face.covers(**c)).count();
            let keys: Vec<String> = std::iter::once(family_key(&traits.family))
                .chain(traits.postscript_name.as_deref().map(|p| parse_name(p).family_key))
                .filter(|k| !k.is_empty())
                .collect();
            let same_family = wanted_keys.iter().any(|w| keys.contains(w));
            // "IBMPlexSans" asked for, "IBM Plex Sans Arabic" here: a relative.
            let related = !same_family
                && wanted_keys
                    .iter()
                    .any(|w| keys.iter().any(|k| k.starts_with(w.as_str()) || w.starts_with(k.as_str())));

            let mut score = 0.0f32;
            if same_family {
                score += 1000.0;
            } else if related {
                score += 400.0;
            }
            if let (Some(want), Some(have)) = (wanted_serif, traits.serif) {
                score += if want == have { 60.0 } else { -60.0 };
            }
            score += if wanted_mono == traits.monospace { 30.0 } else { -80.0 };
            score -= (wanted_weight - traits.weight).abs() / 10.0;
            if wanted_italic != traits.italic {
                score -= 25.0;
            }
            let style_matches =
                (wanted_weight - traits.weight).abs() < 50.0 && wanted_italic == traits.italic;
            let kind = match (same_family, style_matches) {
                (true, true) => MatchKind::Exact,
                (true, false) => MatchKind::SameFamily,
                _ => MatchKind::Substitute,
            };
            (index, covered, score, kind)
        })
        .max_by(|a, b| a.1.cmp(&b.1).then(a.2.total_cmp(&b.2)))
        .map(|(index, covered, _, kind)| FontMatch { index, kind, covers_text: covered == needed.len() })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Serif,
    Sans,
    Monospace,
}

/// Families common in Arabic and English documents whose fonts often leave
/// PANOSE and the IBM class empty. Keys as made by [`family_key`].
fn known_class(key: &str) -> Option<Class> {
    const KNOWN: &[(&str, Class)] = &[
        // Naskh and other text faces with serif-like contrast.
        ("amiri", Class::Serif),
        ("notonaskharabic", Class::Serif),
        ("traditionalarabic", Class::Serif),
        ("simplifiedarabic", Class::Serif),
        ("arabictypesetting", Class::Serif),
        ("sakkalmajalla", Class::Serif),
        ("timesnewroman", Class::Serif),
        ("times", Class::Serif),
        ("georgia", Class::Serif),
        ("cambria", Class::Serif),
        ("garamond", Class::Serif),
        ("scheherazade", Class::Serif),
        ("lateef", Class::Serif),
        // Sans and Kufi-style faces.
        ("notosansarabic", Class::Sans),
        ("ibmplexsansarabic", Class::Sans),
        ("ibmplexsans", Class::Sans),
        ("arial", Class::Sans),
        ("helvetica", Class::Sans),
        ("helveticaneue", Class::Sans),
        ("tahoma", Class::Sans),
        ("segoeui", Class::Sans),
        ("calibri", Class::Sans),
        ("verdana", Class::Sans),
        ("dubai", Class::Sans),
        ("geezapro", Class::Sans),
        ("dejavusans", Class::Sans),
        ("cairo", Class::Sans),
        ("tajawal", Class::Sans),
        ("courier", Class::Monospace),
        ("couriernew", Class::Monospace),
        ("consolas", Class::Monospace),
        ("menlo", Class::Monospace),
    ];
    KNOWN.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedName {
    family_key: String,
    weight: Option<f32>,
    italic: bool,
}

/// Splits a PDF or PostScript font name into a family key and style:
/// "ABCDEF+TimesNewRomanPS-BoldItalicMT" → ("timesnewroman", 700, italic).
fn parse_name(name: &str) -> ParsedName {
    // A subset tag is six capitals and a plus sign.
    let name = match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => name,
    };
    let (family, style) = match name.find([',', '-']) {
        Some(at) => (&name[..at], &name[at + 1..]),
        None => (name, ""),
    };
    // Style words can also be glued to the family: "Arial Bold", "ArialBold".
    let mut family = family.to_string();
    let mut style = style.to_ascii_lowercase();
    for word in ["BoldItalic", "BoldOblique", "Bold", "Italic", "Oblique", "Regular"] {
        if family.len() > word.len() && family.ends_with(word) {
            family.truncate(family.len() - word.len());
            style.insert_str(0, &word.to_ascii_lowercase());
        }
    }
    let style = style.replace(['-', ' ', '_'], "");
    let weight = [
        ("extralight", 200.0),
        ("ultralight", 200.0),
        ("semibold", 600.0),
        ("demibold", 600.0),
        ("extrabold", 800.0),
        ("ultrabold", 800.0),
        ("thin", 100.0),
        ("light", 300.0),
        ("medium", 500.0),
        ("demi", 600.0),
        ("bold", 700.0),
        ("black", 900.0),
        ("heavy", 900.0),
        ("regular", 400.0),
        ("book", 400.0),
    ]
    .iter()
    .find(|(word, _)| style.contains(word))
    .map(|(_, w)| *w);
    ParsedName {
        family_key: family_key(&family),
        weight,
        italic: style.contains("italic") || style.contains("oblique"),
    }
}

/// Comparable family key: letters and digits only, lower case, without the
/// "PS"/"MT" vendor suffixes ("TimesNewRomanPSMT" and "Times New Roman" agree).
fn family_key(family: &str) -> String {
    let mut key: String = family.chars().filter(char::is_ascii_alphanumeric).collect();
    for suffix in ["PSMT", "MT", "PS"] {
        if key.len() > suffix.len() + 2 && key.ends_with(suffix) {
            key.truncate(key.len() - suffix.len());
        }
    }
    key.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_into_family_and_style() {
        let p = parse_name("ABCDEF+TimesNewRomanPS-BoldItalicMT");
        assert_eq!((p.family_key.as_str(), p.weight, p.italic), ("timesnewroman", Some(700.0), true));
        let p = parse_name("Arial,Bold");
        assert_eq!((p.family_key.as_str(), p.weight, p.italic), ("arial", Some(700.0), false));
        let p = parse_name("Amiri-Regular");
        assert_eq!((p.family_key.as_str(), p.weight), ("amiri", Some(400.0)));
        let p = parse_name("IBMPlexSansArabic-SemiBold");
        assert_eq!((p.family_key.as_str(), p.weight), ("ibmplexsansarabic", Some(600.0)));
        let p = parse_name("ArialMT");
        assert_eq!((p.family_key.as_str(), p.weight), ("arial", None));
        assert_eq!(family_key("Times New Roman"), "timesnewroman");
    }

    #[test]
    fn descriptor_flags_and_weight_fill_the_request() {
        let r = FontRequest::from_pdf("ABCDEF+Mystery", None, Some(700.0), Some(1 << 6 | 1 << 1), None);
        assert_eq!(
            (r.weight, r.italic, r.serif, r.monospace),
            (Some(700.0), Some(true), Some(true), Some(false))
        );
        let r = FontRequest::from_pdf("Tahoma-Bold", None, None, Some(32), Some(0.0));
        assert_eq!((r.weight, r.italic, r.serif), (Some(700.0), Some(false), None));
    }
}
