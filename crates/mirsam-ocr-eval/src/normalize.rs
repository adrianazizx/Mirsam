use unicode_normalization::UnicodeNormalization;

/// How much to forgive before comparing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The gated profile. Only differences that are invisible on paper are
    /// removed: Unicode composition form, bidi control characters, and runs of
    /// whitespace. Diacritics, digit style, and punctuation must match.
    Strict,
    /// Informational. Additionally drops tashkeel and tatweel and unifies the
    /// letter variants that Arabic search treats as equal (spec section 6.2).
    BaseLetters,
}

/// Invisible directional and formatting characters that carry no page content.
fn is_format_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{061C}')
}

/// Arabic combining marks: harakat, tanween, shadda, sukun, Quranic annotation, superscript alef.
fn is_tashkeel(c: char) -> bool {
    matches!(c, '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}')
}

const TATWEEL: char = '\u{0640}';

fn fold_base_letter(c: char) -> char {
    match c {
        'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
        'ى' => 'ي',
        'ؤ' => 'و',
        'ئ' => 'ي',
        other => other,
    }
}

/// Canonical form of a text for comparison under `profile`: NFC, no bidi
/// controls, whitespace collapsed to single spaces, trimmed.
pub fn normalize(text: &str, profile: Profile) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for c in text.nfc() {
        if is_format_control(c) {
            continue;
        }
        if c.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        let c = match profile {
            Profile::Strict => c,
            Profile::BaseLetters => {
                if is_tashkeel(c) || c == TATWEEL {
                    continue;
                }
                fold_base_letter(c)
            }
        };
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_keeps_everything_visible() {
        let text = "  قَالَ\u{200F}  الْمُعَلِّمُ:\n١٢٣  ";
        assert_eq!(normalize(text, Profile::Strict), "قَالَ الْمُعَلِّمُ: ١٢٣");
    }

    #[test]
    fn strict_composes_but_does_not_fold_presentation_forms() {
        // Decomposed alef + hamza above composes to U+0623 under NFC...
        assert_eq!(normalize("ا\u{0654}", Profile::Strict), "أ");
        // ...but a presentation-form glyph (U+FEFB lam-alef ligature) is an engine
        // error the strict profile must still see.
        assert_ne!(normalize("\u{FEFB}", Profile::Strict), "لا");
    }

    #[test]
    fn base_letters_drops_marks_and_unifies_variants() {
        assert_eq!(normalize("إِنَّ الْعِلْمَ نُورٌ", Profile::BaseLetters), "ان العلم نور");
        assert_eq!(normalize("مـــرحبا إلى", Profile::BaseLetters), "مرحبا الي");
    }
}
