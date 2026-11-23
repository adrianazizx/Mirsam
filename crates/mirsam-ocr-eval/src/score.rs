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

/// Table cells: how many came out exactly, and how many of those are out of
/// reading order. Page-level CER punishes a table read in the wrong order so
/// heavily that it hides whether the cells themselves were read right (ADR 0030).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellScore {
    /// Cells whose text appears in the output exactly, as a whole value.
    pub matched: usize,
    pub total: usize,
    /// Matched cells that appear before the cell that precedes them in reading
    /// order (row by row, each row in its own direction). A right-to-left row
    /// read left to right costs one per cell after the first.
    pub misordered: usize,
}

impl CellScore {
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
    /// Table cells; all zero for a page without a table. Absent from reports
    /// written before set v2.
    #[serde(default)]
    pub cells: CellScore,
}

/// Scores one page of OCR output against its ground truth.
///
/// `fields` are values (numbers, dates, IDs) that must appear in the output
/// exactly, character for character. `cells` are the page's table cells, row by
/// row, each row in reading order.
pub fn score_sample(truth: &str, hypothesis: &str, fields: &[String], cells: &[Vec<String>]) -> SampleScore {
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
        cells: score_cells(&hyp_strict, cells),
    }
}

/// Scores the output against each acceptable truth (the reading orders a page
/// allows) and keeps the closest one: fewest character errors, then the first
/// listed. `truths` must not be empty.
pub fn score_best(
    truths: &[&str],
    hypothesis: &str,
    fields: &[String],
    cells: &[Vec<String>],
) -> SampleScore {
    let mut best: Option<SampleScore> = None;
    for truth in truths {
        let score = score_sample(truth, hypothesis, fields, cells);
        if best.as_ref().is_none_or(|b| score.cer.errors < b.cer.errors) {
            best = Some(score);
        }
    }
    best.unwrap_or_else(|| score_sample("", hypothesis, fields, cells))
}

/// True when `field` occurs in `text` as a whole value: not embedded in a longer
/// run of letters or digits (so "4,500" does not match inside "14,500").
fn contains_field(text: &str, field: &str) -> bool {
    whole_value_matches(text, field).next().is_some()
}

/// Byte ranges where `field` occurs in `text` as a whole value, in text order.
fn whole_value_matches<'a>(text: &'a str, field: &'a str) -> impl Iterator<Item = (usize, usize)> + 'a {
    let joins = |neighbour: Option<char>, edge: Option<char>| matches!((neighbour, edge), (Some(n), Some(e)) if n.is_alphanumeric() && e.is_alphanumeric());
    text.match_indices(field).filter(|_| !field.is_empty()).filter_map(move |(start, matched)| {
        let end = start + matched.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        let whole = !joins(before, field.chars().next()) && !joins(after, field.chars().next_back());
        whole.then_some((start, end))
    })
}

/// Walks the cells in reading order with a cursor through the output: each
/// matched cell is taken at its first whole-value occurrence after the previous
/// one; if there is none after it, the cell is out of order.
fn score_cells(hyp_strict: &str, rows: &[Vec<String>]) -> CellScore {
    let mut score = CellScore::default();
    let mut cursor = 0;
    for cell in rows.iter().flatten() {
        let cell = normalize(cell, Profile::Strict);
        if cell.is_empty() {
            continue;
        }
        score.total += 1;
        let occurrences: Vec<(usize, usize)> = whole_value_matches(hyp_strict, &cell).collect();
        let Some(&first) = occurrences.first() else { continue };
        score.matched += 1;
        let (_, end) = match occurrences.iter().find(|(start, _)| *start >= cursor) {
            Some(&next) => next,
            None => {
                score.misordered += 1;
                first
            }
        };
        cursor = end;
    }
    score
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
        score_sample(truth, hypothesis, &[], &[])
    }

    fn table() -> Vec<Vec<String>> {
        [["البند", "Item", "الكمية"], ["حبر طابعة", "Printer ink", "4"], ["الإجمالي", "Total", "14"]]
            .iter()
            .map(|row| row.iter().map(|cell| cell.to_string()).collect())
            .collect()
    }

    fn cells(hypothesis: &str) -> CellScore {
        score_sample("unused", hypothesis, &[], &table()).cells
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
        let result = score_sample("unused", hypothesis, &fields, &[]);
        // "4,500" only occurs inside "14,500"; Arabic-Indic digits came out Western.
        assert_eq!(result.fields, FieldScore { matched: 2, total: 4 });
    }

    #[test]
    fn table_cells_in_reading_order_score_perfectly() {
        // One cell per line or one row per line: whitespace does not matter.
        let by_cell = "البند\nItem\nالكمية\nحبر طابعة\nPrinter ink\n4\nالإجمالي\nTotal\n14";
        let by_row = "البند Item الكمية\nحبر طابعة Printer ink 4\nالإجمالي Total 14";
        for hypothesis in [by_cell, by_row] {
            assert_eq!(cells(hypothesis), CellScore { matched: 9, total: 9, misordered: 0 });
        }
    }

    #[test]
    fn a_right_to_left_table_read_left_to_right_is_misordered_not_misread() {
        let ltr = "الكمية Item البند\n4 Printer ink حبر طابعة\n14 Total الإجمالي";
        // Every cell was read; two cells per row are out of place.
        assert_eq!(cells(ltr), CellScore { matched: 9, total: 9, misordered: 6 });
    }

    #[test]
    fn table_cells_must_be_whole_values() {
        // "4" only occurs inside "14"; "Printer ink" is missing.
        let result = cells("البند Item الكمية\nحبر طابعة\nالإجمالي Total 14");
        assert_eq!(result, CellScore { matched: 7, total: 9, misordered: 0 });
        assert!((result.rate() - 7.0 / 9.0).abs() < 1e-12);
    }

    #[test]
    fn a_page_without_a_table_has_no_cells() {
        assert_eq!(cells("").total, 9);
        assert_eq!(score("نص", "نص").cells, CellScore::default());
    }

    #[test]
    fn either_column_order_is_accepted_when_the_page_allows_both() {
        let english = "This Lease Agreement is made between the Landlord and the Tenant";
        let arabic = "أُبرم عقد الإيجار هذا بين المؤجر والمستأجر المذكورين أدناه";
        let english_first = format!("{english}\n{arabic}");
        let arabic_first = format!("{arabic}\n{english}");
        let truths = [english_first.as_str(), arabic_first.as_str()];
        let result = score_best(&truths, &arabic_first, &[], &[]);
        assert_eq!((result.cer.errors, result.order.total()), (0, 0));
        // Interleaving the columns word by word fits neither order.
        let english_words: Vec<&str> = english.split(' ').collect();
        let arabic_words: Vec<&str> = arabic.split(' ').collect();
        let interleaved: Vec<&str> =
            english_words.iter().zip(&arabic_words).flat_map(|(e, a)| [*e, *a]).collect();
        assert!(score_best(&truths, &interleaved.join(" "), &[], &[]).cer.errors > 0);
    }

    #[test]
    fn empty_output_is_a_total_miss() {
        let result = score("نص", "");
        assert_eq!(result.cer.rate(), 1.0);
        assert_eq!(result.wer.rate(), 1.0);
    }
}
