//! Logical order for number–unit compounds read off right-to-left lines (ADR 0033).
//!
//! In an Arabic line, "64-bit" (logical) is drawn as "bit-64": the number takes the
//! Arabic context's direction and the Latin word keeps its own. A model reading
//! the page writes what it sees, "bit-64". The image cannot settle it. Logical
//! "bit-64" is drawn the same way, and "COVID-19" and "19-COVID" are likewise drawn
//! identically (ADR 0008 found the same from glyph order).
//!
//! So the choice is lexical, and kept narrow. Only where the Latin part is a unit
//! that English writes number-first ("64-bit", "3-day", "5-star") is the order
//! turned round. The change is reported so the word can be listed for review.
//! "COVID-19", "A-4" or "Windows-11" are left alone.

/// Units written after their number in English compounds.
pub const UNIT_WORDS: &[&str] = &[
    "bit", "byte", "core", "inch", "star", "day", "week", "month", "year", "hour", "minute", "second",
    "page", "point", "piece", "part", "step", "digit", "speed", "way", "door", "seat", "room", "lane",
    "track", "layer", "level", "sided", "fold", "storey", "story",
];

/// One word whose order was turned round.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderFix {
    /// Index of the word in the line (split on single spaces).
    pub word: usize,
    pub read: String,
    pub logical: String,
}

/// `line` with each `<unit>-<number>` word turned into `<number>-<unit>`, when the
/// line holds Arabic (so it was drawn right to left). Leading and trailing
/// punctuation stay where they are: "(bit-64)!" becomes "(64-bit)!".
pub fn resolve_unit_order(line: &str) -> (String, Vec<OrderFix>) {
    if !line.chars().any(is_arabic) {
        return (line.to_owned(), Vec::new());
    }
    let mut fixes = Vec::new();
    let words: Vec<String> = line
        .split(' ')
        .enumerate()
        .map(|(index, word)| match flip(word) {
            Some(logical) => {
                fixes.push(OrderFix { word: index, read: word.to_owned(), logical: logical.clone() });
                logical
            }
            None => word.to_owned(),
        })
        .collect();
    (words.join(" "), fixes)
}

fn is_arabic(c: char) -> bool {
    matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}')
}

/// `(bit-64)!` → `(64-bit)!` when `bit` is a unit word.
fn flip(word: &str) -> Option<String> {
    let core_start = word.find(|c: char| c.is_ascii_alphanumeric())?;
    let core_end = word.rfind(|c: char| c.is_ascii_alphanumeric())? + 1;
    let (lead, core, tail) = (&word[..core_start], &word[core_start..core_end], &word[core_end..]);
    let (unit, number) = core.split_once('-')?;
    let is_unit = !unit.is_empty()
        && unit.chars().all(|c| c.is_ascii_alphabetic())
        && UNIT_WORDS.contains(&unit.to_ascii_lowercase().as_str());
    let is_number = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    (is_unit && is_number).then(|| format!("{lead}{number}-{unit}{tail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_number_unit_compounds_in_arabic_lines() {
        let (line, fixes) = resolve_unit_order("هل تستخدم برنامج Adobe Acrobat؟ نعم، الإصدار 2024 (bit-64)!");
        assert_eq!(line, "هل تستخدم برنامج Adobe Acrobat؟ نعم، الإصدار 2024 (64-bit)!");
        assert_eq!(fixes, vec![OrderFix { word: 8, read: "(bit-64)!".into(), logical: "(64-bit)!".into() }]);
        assert_eq!(resolve_unit_order("فندق Star-5 فاخر").0, "فندق 5-Star فاخر");
    }

    #[test]
    fn leaves_everything_else_alone() {
        for line in [
            "The (bit-64) build",   // a left-to-right line: drawn as read
            "لقاح COVID-19 الجديد", // not a unit
            "ورق A-4 أبيض",         // not a unit
            "الإصدار Windows-11",    // not a unit
            "الإصدار bit-x64",       // not a number
            "رقم 64-bit صحيح",      // already logical
        ] {
            assert_eq!(resolve_unit_order(line), (line.to_owned(), Vec::new()), "{line}");
        }
    }
}
