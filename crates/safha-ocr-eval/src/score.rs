use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::align::{Op, align, edit_distance};
use crate::normalize::{Profile, normalize};

/// An error count over a reference length. Kept as a pair so categories can be
/// micro-averaged (sum of errors over sum of lengths) rather than averaging rates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub errors: usize,
    pub reference: usize,
}

impl Counts {
    /// Error rate. Can exceed 1.0 when the hypothesis is much longer than the reference.
    pub fn rate(&self) -> f64 {
        if self.reference == 0 {
            if self.errors == 0 { 0.0 } else { 1.0 }
        } else {
            self.errors as f64 / self.reference as f64
        }
    }

    pub(crate) fn add(&mut self, other: Counts) {
        self.errors += other.errors;
        self.reference += other.reference;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldScore {
    pub matched: usize,
    pub total: usize,
}

impl FieldScore {
    pub fn rate(&self) -> f64 {
        if self.total == 0 { 1.0 } else { self.matched as f64 / self.total as f64 }
    }
}

/// Text that was recognized but ended up in the wrong order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderScore {
    /// Runs of two or more consecutive reference words that are missing from
    /// their place yet present elsewhere in the output: a line, column, or table
    /// cell read in the wrong sequence, or a line whose words are reversed.
    pub displaced_runs: usize,
    /// Words whose letters came out reversed (visual-order Arabic).
    pub reversed_words: usize,
}

impl OrderScore {
    pub fn total(&self) -> usize {
        self.displaced_runs + self.reversed_words
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleScore {
    /// Strict character error rate — the gated number.
    pub cer: Counts,
    /// Strict word error rate.
    pub wer: Counts,
    /// CER ignoring tashkeel, tatweel, and letter variants. Informational.
    pub cer_base: Counts,
    pub fields: FieldScore,
    pub order: OrderScore,
}

/// Scores one page of OCR output against its ground truth.
///
/// `fields` are values (numbers, dates, IDs) that must appear in the output
/// exactly, character for character.
pub fn score_sample(truth: &str, hypothesis: &str, fields: &[String]) -> SampleScore {
    let truth_strict = normalize(truth, Profile::Strict);
    let hyp_strict = normalize(hypothesis, Profile::Strict);

    let truth_chars: Vec<char> = truth_strict.chars().collect();
    let hyp_chars: Vec<char> = hyp_strict.chars().collect();
    let cer = Counts { errors: edit_distance(&truth_chars, &hyp_chars), reference: truth_chars.len() };

    let truth_words: Vec<&str> = truth_strict.split(' ').filter(|word| !word.is_empty()).collect();
    let hyp_words: Vec<&str> = hyp_strict.split(' ').filter(|word| !word.is_empty()).collect();
    let ops = align(&truth_words, &hyp_words);
    let wer = Counts {
        errors: ops.iter().filter(|op| !matches!(op, Op::Match { .. })).count(),
        reference: truth_words.len(),
    };

    let truth_base: Vec<char> = normalize(truth, Profile::BaseLetters).chars().collect();
    let hyp_base: Vec<char> = normalize(hypothesis, Profile::BaseLetters).chars().collect();
    let cer_base = Counts { errors: edit_distance(&truth_base, &hyp_base), reference: truth_base.len() };

    let matched =
        fields.iter().filter(|field| contains_field(&hyp_strict, &normalize(field, Profile::Strict))).count();

    SampleScore {
        cer,
        wer,
        cer_base,
        fields: FieldScore { matched, total: fields.len() },
        order: order_errors(&truth_words, &hyp_words, &ops),
    }
}

/// True when `field` occurs in `text` as a whole value: not embedded in a longer
/// run of letters or digits (so "4,500" does not match inside "14,500").
fn contains_field(text: &str, field: &str) -> bool {
    if field.is_empty() {
        return false;
    }
    text.match_indices(field).any(|(start, matched)| {
        let before = text[..start].chars().next_back();
        let after = text[start + matched.len()..].chars().next();
        let joins = |neighbour: Option<char>, edge: Option<char>| {
            matches!((neighbour, edge), (Some(n), Some(e)) if n.is_alphanumeric() && e.is_alphanumeric())
        };
        !joins(before, field.chars().next()) && !joins(after, field.chars().next_back())
    })
}

fn order_errors(truth_words: &[&str], hyp_words: &[&str], ops: &[Op]) -> OrderScore {
    // Hypothesis words the in-order alignment could not place.
    let mut stray: HashMap<&str, usize> = HashMap::new();
    for op in ops {
        if let Op::Insert { h } | Op::Substitute { h, .. } = op {
            *stray.entry(hyp_words[*h]).or_default() += 1;
        }
    }

    // Reference words missing from their place, in reference order.
    let missing: Vec<usize> = ops
        .iter()
        .filter_map(|op| match op {
            Op::Delete { r } | Op::Substitute { r, .. } => Some(*r),
            _ => None,
        })
        .collect();

    let mut displaced = vec![false; truth_words.len()];
    let mut reversed_words = 0;
    for &r in &missing {
        let word = truth_words[r];
        if let Some(count) = stray.get_mut(word).filter(|count| **count > 0) {
            *count -= 1;
            displaced[r] = true;
            continue;
        }
        let backwards: String = word.chars().rev().collect();
        let is_reversible = word.chars().count() >= 3 && backwards != word;
        if is_reversible && let Some(count) = stray.get_mut(backwards.as_str()).filter(|count| **count > 0) {
            *count -= 1;
            reversed_words += 1;
        }
    }

    // A lone displaced word is usually coincidence (a common word dropped here
    // and hallucinated there); two or more in a row is a moved segment.
    let mut displaced_runs = 0;
    let mut run = 0;
    for flag in displaced.iter().chain(std::iter::once(&false)) {
        if *flag {
            run += 1;
        } else {
            if run >= 2 {
                displaced_runs += 1;
            }
            run = 0;
        }
    }

    OrderScore { displaced_runs, reversed_words }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(truth: &str, hypothesis: &str) -> SampleScore {
        score_sample(truth, hypothesis, &[])
    }

    #[test]
    fn perfect_output_scores_zero() {
        let text = "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال (شاملة الضريبة).";
        let result = score(text, text);
        assert_eq!((result.cer.errors, result.wer.errors, result.order.total()), (0, 0, 0));
        assert_eq!(result.cer.reference, text.chars().count());
    }

    #[test]
    fn line_breaks_and_bidi_marks_are_not_errors() {
        let result = score("صدر القرار رقم ١٢٣٤ لسنة ٢٠٢٥", "صدر القرار\u{200F}\nرقم ١٢٣٤   لسنة ٢٠٢٥\n");
        assert_eq!(result.cer.errors, 0);
    }

    #[test]
    fn counts_character_and_word_errors() {
        // One substituted letter in one of four words.
        let result = score("the quick brown fox", "the quick brawn fox");
        assert_eq!(result.cer, Counts { errors: 1, reference: 19 });
        assert_eq!(result.wer, Counts { errors: 1, reference: 4 });
        assert!((result.wer.rate() - 0.25).abs() < 1e-12);
    }

    #[test]
    fn dropped_tashkeel_is_a_strict_error_but_not_a_base_letter_error() {
        let result = score("قَالَ الْمُعَلِّمُ", "قال المعلم");
        assert!(result.cer.errors >= 8);
        assert_eq!(result.cer_base.errors, 0);
    }

    #[test]
    fn digit_style_must_be_preserved() {
        let result = score("رقم ١٢٣٤", "رقم 1234");
        assert_eq!(result.cer.errors, 4);
    }

    #[test]
    fn detects_a_line_with_reversed_word_order() {
        let truth = "يدفع المستأجر مبلغ الإيجار في اليوم الأول";
        let reversed: Vec<&str> = truth.split(' ').rev().collect();
        let result = score(truth, &reversed.join(" "));
        assert!(result.order.displaced_runs >= 1, "{:?}", result.order);
    }

    #[test]
    fn detects_letters_reversed_inside_words() {
        let truth = "عقد إيجار جديد";
        let visual: Vec<String> = truth.split(' ').map(|word| word.chars().rev().collect()).collect();
        let result = score(truth, &visual.join(" "));
        assert_eq!(result.order.reversed_words, 3);
    }

    #[test]
    fn detects_columns_read_in_the_wrong_order() {
        let left = "This Lease Agreement is made between the Landlord and the Tenant";
        let right = "أُبرم عقد الإيجار هذا بين المؤجر والمستأجر المذكورين أدناه";
        let result = score(&format!("{left} {right}"), &format!("{right} {left}"));
        assert!(result.order.displaced_runs >= 1);
        // Nothing was misrecognized, so the damage shows up as word errors only.
        assert!(result.wer.errors > 0);
    }

    #[test]
    fn a_plain_typo_is_not_an_order_error() {
        let result = score("the quick brown fox jumps", "the quikc brown fox jumps");
        assert_eq!(result.order.total(), 0);
    }

    #[test]
    fn fields_must_match_exactly_and_as_whole_values() {
        let fields: Vec<String> =
            ["4,500", "INV-2026-00417", "12/03/2026", "١٢٣٤"].iter().map(|s| s.to_string()).collect();
        let hypothesis = "المبلغ 14,500 ريال، الفاتورة INV-2026-00417 بتاريخ 12/03/2026 رقم 1234";
        let result = score_sample("unused", hypothesis, &fields);
        // "4,500" only occurs inside "14,500"; Arabic-Indic digits came out Western.
        assert_eq!(result.fields, FieldScore { matched: 2, total: 4 });
    }

    #[test]
    fn empty_output_is_a_total_miss() {
        let result = score("نص", "");
        assert_eq!(result.cer.rate(), 1.0);
        assert_eq!(result.wer.rate(), 1.0);
    }
}
