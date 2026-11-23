//! Review mode (spec §5.8, ADR 0037): a person steps through the words the
//! pipeline flagged, beside the image, and accepts, corrects or marks each one.
//!
//! - **What is flagged** ([`Reason`]): readers that disagreed (`uncertain`),
//!   words re-read into something else (`corrected`), number–unit compounds
//!   turned round (`reordered`), words the primary was unsure of even after
//!   the re-reads agreed (`lowConfidence`), and, on request, every number,
//!   date and ID (`number`).
//! - **Decisions** ([`Action`]): *accept* the text as recognised, *edit* it
//!   (typing goes into the text layer), *check* it (looked at, left as it is,
//!   not vouched for), or *check the page* (every word left on it, and a
//!   page whose reading was cut off).
//! - **Decisions follow the document.** A correction is an edit of the
//!   document like any other, so Undo can take it away. Each decision is kept
//!   with the digest of the layer it leaves ([`PageLayer::digest`], written
//!   into the layer's stream), and a decision's parent is the state it was
//!   taken in: a tree. The review shows the newest state whose layer is the
//!   one the document has ([`resolve`]). Undo a correction and the word is
//!   flagged again; redo it and it is corrected again.
//!
//! Nothing here invents text: an edit is what a person typed or chose, and
//! the word keeps what the pipeline read (`WordReview::Edited { read }`).

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use super::jobs::{JobId, OcrWord, PageOcr};
use super::layer::{Clear, LayerQuality, LayerState, PageLayer};
use super::reconcile::WordStatus;

/// Words the primary engine was less sure of than this are listed even when
/// the re-reads agreed with it.
pub const LOW_CONFIDENCE: f32 = 0.80;
/// The longest text a word is corrected to, in characters.
pub const MAX_WORD: usize = 200;

/// What a person decided about a word.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WordReview {
    /// Not looked at yet.
    #[default]
    Open,
    /// The text is right as recognised.
    Accepted,
    /// A person typed or chose the text; `read` is what the pipeline wrote.
    Edited { read: String },
    /// Looked at and left as it is, without vouching for it.
    Checked,
}

impl WordReview {
    pub fn is_open(&self) -> bool {
        matches!(self, WordReview::Open)
    }
}

/// A decision taken in review.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    Accept,
    Check,
    /// The word's text, as typed; empty when it is not text at all (a speck).
    Edit {
        text: String,
    },
    /// Every word still open on the page is checked, and so is the page.
    CheckPage,
}

/// A decision as kept with the page's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    /// The decision it was taken after (an index into the page's decisions);
    /// `None`: the result as recognised.
    pub parent: Option<u32>,
    /// The word (its place among the page's words); `None` for the page.
    pub word: Option<u32>,
    pub action: Action,
    /// The digest of the layer the decision leaves.
    pub digest: u32,
}

/// Why a word is on the review list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    Uncertain,
    Corrected,
    Reordered,
    LowConfidence,
    Number,
}

/// Which words the list holds besides the flagged ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFilter {
    /// Every word holding a digit: numbers, dates, IDs.
    #[serde(default)]
    pub numbers: bool,
}

/// Why `word` is listed; empty when it is not.
pub fn reasons(word: &OcrWord) -> Vec<Reason> {
    let mut reasons = Vec::new();
    match word.status {
        WordStatus::Uncertain { .. } => reasons.push(Reason::Uncertain),
        WordStatus::Corrected { .. } => reasons.push(Reason::Corrected),
        WordStatus::Reordered { .. } => reasons.push(Reason::Reordered),
        WordStatus::Unchecked | WordStatus::Confirmed => {}
    }
    if word.probability < LOW_CONFIDENCE && !reasons.contains(&Reason::Uncertain) {
        reasons.push(Reason::LowConfidence);
    }
    if word.text.chars().any(char::is_numeric)
        || matches!(&word.review, WordReview::Edited { read } if read.chars().any(char::is_numeric))
    {
        reasons.push(Reason::Number);
    }
    reasons
}

/// A page's result with the decisions on one branch applied.
#[derive(Clone, Debug, PartialEq)]
pub struct PageView {
    pub page: usize,
    pub words: Vec<OcrWord>,
    /// The layer these words make (what the document has when it matches).
    pub layer: PageLayer,
    /// A person checked the page as a whole.
    pub checked: bool,
    pub truncated: bool,
}

impl PageView {
    /// Whether a person must still decide something before the page's text can
    /// be trusted: a word the readers disagree on, or a reading cut off.
    pub fn needs_decision(&self) -> bool {
        (self.truncated && !self.checked)
            || self
                .words
                .iter()
                .any(|w| matches!(w.status, WordStatus::Uncertain { .. }) && w.review.is_open())
    }

    /// The words still needing a decision.
    pub fn undecided(&self) -> usize {
        self.words
            .iter()
            .filter(|w| matches!(w.status, WordStatus::Uncertain { .. }) && w.review.is_open())
            .count()
    }
}

impl PageOcr {
    /// The digest of the layer as recognised.
    pub fn base_digest(&self) -> u32 {
        self.digest.unwrap_or_else(|| self.layer.digest())
    }

    /// The digest after decision `node` (`None`: as recognised).
    pub fn digest_at(&self, node: Option<usize>) -> u32 {
        match node.and_then(|n| self.decisions.get(n)) {
            Some(decision) => decision.digest,
            None => self.base_digest(),
        }
    }

    /// The page with the decisions from the result as recognised to `node` applied.
    pub fn view(&self, node: Option<usize>) -> PageView {
        let mut view = PageView {
            page: self.page,
            words: self.words.clone(),
            layer: self.layer.clone(),
            checked: self.checked,
            truncated: self.truncated,
        };
        locate(&mut view);
        let mut chain = Vec::new();
        let mut at = node.filter(|&n| n < self.decisions.len());
        while let Some(n) = at {
            chain.push(n);
            // A parent always comes before its child; anything else is ignored.
            at = self.decisions[n].parent.map(|p| p as usize).filter(|&p| p < n);
        }
        for n in chain.into_iter().rev() {
            let decision = &self.decisions[n];
            apply(&mut view, decision.word.map(|w| w as usize), &decision.action);
        }
        view
    }
}

/// Gives every word its place in the layer (`at`): results kept before places
/// were recorded find theirs by text and box.
fn locate(view: &mut PageView) {
    let close = |a: f32, b: f32| (a - b).abs() < 0.01;
    let mut taken = std::collections::HashSet::new();
    for word in view.words.iter().filter_map(|w| w.at) {
        taken.insert(word);
    }
    for word in &mut view.words {
        if word.at.is_some() {
            continue;
        }
        let Some(b) = word.bounds else { continue };
        let found = view.layer.lines.iter().enumerate().find_map(|(l, line)| {
            line.words.iter().enumerate().find_map(|(p, lw)| {
                let at = [l as u32, p as u32];
                (lw.text == word.text
                    && close(lw.left, b[0])
                    && close(lw.top, b[1])
                    && close(lw.right, b[2])
                    && close(lw.bottom, b[3])
                    && !taken.contains(&at))
                .then_some(at)
            })
        });
        if let Some(at) = found {
            taken.insert(at);
            word.at = Some(at);
        }
    }
}

/// Text as a word is corrected to: NFC, trimmed, inner white space one space.
pub fn clean_text(text: &str) -> String {
    text.nfc().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn apply(view: &mut PageView, word: Option<usize>, action: &Action) {
    match (action, word) {
        (Action::CheckPage, _) => {
            view.checked = true;
            for w in &mut view.words {
                if w.review.is_open() && !reasons(w).is_empty() {
                    w.review = WordReview::Checked;
                }
            }
        }
        (Action::Accept, Some(i)) => {
            if let Some(w) = view.words.get_mut(i)
                && !matches!(w.review, WordReview::Edited { .. })
            {
                w.review = WordReview::Accepted;
            }
        }
        (Action::Check, Some(i)) => {
            if let Some(w) = view.words.get_mut(i)
                && w.review.is_open()
            {
                w.review = WordReview::Checked;
            }
        }
        (Action::Edit { text }, Some(i)) => {
            let Some(w) = view.words.get_mut(i) else { return };
            let read = match &w.review {
                WordReview::Edited { read } => read.clone(),
                _ => w.text.clone(),
            };
            w.text = text.clone();
            w.review = WordReview::Edited { read };
            if let Some([l, p]) = w.at
                && let Some(lw) =
                    view.layer.lines.get_mut(l as usize).and_then(|line| line.words.get_mut(p as usize))
            {
                lw.text = text.clone();
            }
        }
        _ => {}
    }
}

/// Why a decision was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecideError {
    #[error("no such word on the page")]
    NoWord,
    #[error("a word is corrected to at most 200 characters")]
    TooLong,
    /// The page's text is page content now (ADR 0039): it is edited there.
    #[error("the page was converted to editable text")]
    Converted,
    /// The document's layer is not one the review knows (written elsewhere,
    /// or its review data was removed): writing would lose it.
    #[error("the page's text layer changed outside the review")]
    Stale,
}

impl DecideError {
    pub fn code(&self) -> &'static str {
        match self {
            DecideError::NoWord => "ocr_review_no_word",
            DecideError::TooLong => "ocr_review_too_long",
            DecideError::Converted => "ocr_review_converted",
            DecideError::Stale => "ocr_review_stale",
        }
    }
}

/// Takes `action` on `word` (`None`: the page) in the state `node` of `row`,
/// keeping the decision in `row`. Returns the page after it, and the layer to
/// write when the text changed.
pub fn decide(
    row: &mut PageOcr,
    node: Option<usize>,
    word: Option<usize>,
    action: Action,
) -> Result<(PageView, Option<PageLayer>), DecideError> {
    let mut view = row.view(node);
    let before = row.digest_at(node);
    let action = match action {
        Action::Edit { text } => {
            let text = clean_text(&text);
            if text.chars().count() > MAX_WORD {
                return Err(DecideError::TooLong);
            }
            let current = word.and_then(|i| view.words.get(i)).ok_or(DecideError::NoWord)?;
            if text == current.text { Action::Accept } else { Action::Edit { text } }
        }
        other => other,
    };
    if let Some(i) = word
        && i >= view.words.len()
    {
        return Err(DecideError::NoWord);
    }
    if word.is_none() && action != Action::CheckPage {
        return Err(DecideError::NoWord);
    }
    apply(&mut view, word, &action);
    let writes = matches!(action, Action::Edit { .. });
    view.layer.quality = LayerQuality::Final;
    view.layer.clear = Clear::None;
    let digest = if writes { view.layer.digest() } else { before };
    row.decisions.push(Decision {
        parent: node.map(|n| n as u32),
        word: word.map(|w| w as u32),
        action,
        digest,
    });
    let layer = writes.then(|| view.layer.clone());
    Ok((view, layer))
}

/// A page's review data, resolved against the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// The job whose result it is.
    pub job: JobId,
    /// The decision whose state it is (`None`: as recognised).
    pub node: Option<usize>,
    pub view: PageView,
    /// The page's text was converted to page content (ADR 0039).
    pub converted: bool,
    /// The document's layer is not one the review knows: shown, not written.
    pub stale: bool,
}

/// Which of a page's kept results (`rows`: its final results, from any job)
/// and decisions the document's page shows (`doc`). `None` when the page has
/// no layer of ours (never read, the layer undone) or only a draft (being read
/// again).
pub fn resolve(rows: &[(JobId, &PageOcr)], doc: LayerState) -> Option<Resolved> {
    let converted = doc.quality.is_none() && doc.converted.is_some();
    let target = match doc.quality {
        Some(LayerQuality::Draft) => return None,
        Some(LayerQuality::Final) => doc.digest,
        None if converted => doc.converted.filter(|&d| d != 0),
        None => return None,
    };
    let mut newest: Vec<&(JobId, &PageOcr)> = rows.iter().collect();
    newest.sort_by_key(|(job, _)| std::cmp::Reverse(*job));
    if let Some(target) = target {
        for (job, row) in &newest {
            let nodes = (0..row.decisions.len()).rev().map(Some).chain(std::iter::once(None));
            for node in nodes {
                if row.digest_at(node) == target {
                    return Some(Resolved { job: *job, node, view: row.view(node), converted, stale: false });
                }
            }
        }
    }
    // A layer written before digests (or one we have no record of): the latest
    // state, which a correction must not overwrite when the layer is unknown.
    let (job, row) = newest.first()?;
    let node = row.decisions.len().checked_sub(1);
    Some(Resolved { job: *job, node, view: row.view(node), converted, stale: target.is_some() })
}

// ---------- the list ----------

/// A page on the pages-needing-review list.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPage {
    pub page: usize,
    /// Words the readers disagree on (the page read was kept).
    pub uncertain: usize,
    /// Words replaced because re-reads agreed on something else.
    pub corrected: usize,
    /// Number–unit compounds turned into logical order.
    pub reordered: usize,
    /// The answer was cut off: part of the page may be missing.
    pub truncated: bool,
    /// The page's mean and lowest word probability (0–1).
    pub confidence: f32,
    pub lowest: f32,
    /// Words still to decide (uncertain and not decided).
    pub undecided: usize,
    /// Listed words nobody has looked at yet.
    pub open: usize,
    /// A person checked the page as a whole.
    pub checked: bool,
    /// Converted to page text (ADR 0039): its words are fixed with the Edit tool.
    pub converted: bool,
    /// The document's layer is not one the review knows: corrections are refused.
    pub stale: bool,
    /// The words to look at, in reading order.
    pub words: Vec<ReviewWord>,
}

/// A word on the review list.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewWord {
    /// Its place among the page's words (what a decision names).
    pub index: usize,
    pub text: String,
    pub probability: f32,
    pub status: WordStatus,
    pub bounds: Option<[f32; 4]>,
    /// Its line's box on the page (points), for the image beside it.
    pub line: Option<[f32; 4]>,
    pub review: WordReview,
    pub reasons: Vec<Reason>,
}

impl ReviewPage {
    /// An uncertain word not decided, or a cut-off answer not checked.
    pub fn needs_decision(&self) -> bool {
        (self.truncated && !self.checked) || self.undecided > 0
    }
}

/// A page's entry on the list, or `None` when it holds nothing to look at.
pub fn review_page(resolved: &Resolved, filter: ReviewFilter) -> Option<ReviewPage> {
    let view = &resolved.view;
    let count = |f: fn(&WordStatus) -> bool| view.words.iter().filter(|w| f(&w.status)).count();
    let probabilities: Vec<f32> = view.words.iter().map(|w| w.probability).collect();
    let line_box = |at: Option<[u32; 2]>| -> Option<[f32; 4]> {
        let line = view.layer.lines.get(at?[0] as usize)?;
        let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for w in line.words.iter().filter(|w| w.right > w.left) {
            b = [b[0].min(w.left), b[1].min(w.top), b[2].max(w.right), b[3].max(w.bottom)];
        }
        b.iter().all(|v| v.is_finite()).then_some(b)
    };
    let words: Vec<ReviewWord> = view
        .words
        .iter()
        .enumerate()
        .filter_map(|(index, w)| {
            let reasons: Vec<Reason> =
                reasons(w).into_iter().filter(|r| filter.numbers || *r != Reason::Number).collect();
            (!reasons.is_empty()).then(|| ReviewWord {
                index,
                text: w.text.clone(),
                probability: w.probability,
                status: w.status.clone(),
                bounds: w.bounds,
                line: line_box(w.at),
                review: w.review.clone(),
                reasons,
            })
        })
        .collect();
    if words.is_empty() && !view.truncated {
        return None;
    }
    Some(ReviewPage {
        page: view.page,
        uncertain: count(|s| matches!(s, WordStatus::Uncertain { .. })),
        corrected: count(|s| matches!(s, WordStatus::Corrected { .. })),
        reordered: count(|s| matches!(s, WordStatus::Reordered { .. })),
        truncated: view.truncated,
        confidence: if probabilities.is_empty() {
            1.0
        } else {
            probabilities.iter().sum::<f32>() / probabilities.len() as f32
        },
        lowest: probabilities.iter().copied().fold(1.0, f32::min),
        undecided: view.undecided(),
        open: words.iter().filter(|w| w.review.is_open()).count(),
        checked: view.checked,
        converted: resolved.converted,
        stale: resolved.stale,
        words,
    })
}

/// Orders the list: pages needing a decision first, the most undecided words
/// first among them, then by page.
pub fn sort_review(list: &mut [ReviewPage]) {
    list.sort_by_key(|r| (!r.needs_decision(), std::cmp::Reverse(r.undecided), r.page));
}

#[cfg(test)]
mod tests {
    use super::super::layer::{LayerLine, LayerWord};
    use super::*;

    fn word(text: &str, status: WordStatus, probability: f32, at: [u32; 2]) -> OcrWord {
        OcrWord {
            text: text.into(),
            probability,
            status,
            bounds: Some([
                at[1] as f32 * 10.0,
                at[0] as f32 * 20.0,
                at[1] as f32 * 10.0 + 8.0,
                at[0] as f32 * 20.0 + 12.0,
            ]),
            placed_by_width: false,
            at: Some(at),
            review: WordReview::Open,
        }
    }

    /// An Arabic line with an unsure number and an English line.
    fn row() -> PageOcr {
        let words = vec![
            word("صدر", WordStatus::Unchecked, 0.99, [0, 0]),
            word("١٣٣٤", WordStatus::Uncertain { alternatives: vec!["١٢٣٤".into()] }, 0.6, [0, 1]),
            word("Invoice", WordStatus::Unchecked, 0.7, [1, 0]),
            word("2025", WordStatus::Confirmed, 0.99, [1, 1]),
        ];
        let lines = vec![
            LayerLine { rtl: true, words: words[..2].iter().map(layer_word).collect() },
            LayerLine { rtl: false, words: words[2..].iter().map(layer_word).collect() },
        ];
        PageOcr {
            page: 2,
            layer: PageLayer {
                page: 2,
                quality: LayerQuality::Final,
                lines,
                placement: [1.0, 0.0, 0.0, 1.0, 0.0, 842.0],
                clear: Clear::None,
            },
            words,
            truncated: false,
            engine: "test".into(),
            seconds: 1.0,
            checked: false,
            decisions: Vec::new(),
            digest: None,
        }
    }

    fn layer_word(w: &OcrWord) -> LayerWord {
        let b = w.bounds.unwrap_or_default();
        LayerWord { text: w.text.clone(), left: b[0], top: b[1], right: b[2], bottom: b[3] }
    }

    fn state(digest: u32) -> LayerState {
        LayerState { quality: Some(LayerQuality::Final), digest: Some(digest), converted: None }
    }

    #[test]
    fn flags_say_why_and_numbers_only_on_request() {
        let row = row();
        let view = row.view(None);
        let resolved = Resolved { job: 1, node: None, view, converted: false, stale: false };
        let page = review_page(&resolved, ReviewFilter::default()).expect("listed");
        let listed: Vec<(&str, &[Reason])> =
            page.words.iter().map(|w| (w.text.as_str(), &w.reasons[..])).collect();
        assert_eq!(listed, [("١٣٣٤", &[Reason::Uncertain][..]), ("Invoice", &[Reason::LowConfidence][..])]);
        assert!(page.needs_decision());
        assert_eq!((page.undecided, page.open), (1, 2));
        let page = review_page(&resolved, ReviewFilter { numbers: true }).expect("listed");
        assert_eq!(
            page.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            ["١٣٣٤", "Invoice", "2025"]
        );
        assert_eq!(page.words[0].reasons, [Reason::Uncertain, Reason::Number]);
        assert_eq!(page.words[0].line, Some([0.0, 0.0, 18.0, 12.0]));
    }

    #[test]
    fn an_edit_goes_into_the_layer_and_keeps_what_was_read() {
        let mut row = row();
        let base = row.base_digest();
        let (view, layer) =
            decide(&mut row, None, Some(1), Action::Edit { text: " ١٢٣٤ ".into() }).expect("edit");
        let layer = layer.expect("the text changed");
        assert_eq!(layer.lines[0].words[1].text, "١٢٣٤");
        assert_ne!(layer.digest(), base);
        assert_eq!(view.words[1].review, WordReview::Edited { read: "١٣٣٤".into() });
        assert!(!view.needs_decision());
        // Typing what is there already is accepting it: nothing to write.
        let (view, layer) =
            decide(&mut row, Some(0), Some(2), Action::Edit { text: "Invoice".into() }).expect("same");
        assert!(layer.is_none());
        assert_eq!(view.words[2].review, WordReview::Accepted);
        assert_eq!(view.words[1].text, "١٢٣٤", "on the edited branch");
        assert!(matches!(decide(&mut row, None, Some(9), Action::Accept), Err(DecideError::NoWord)));
    }

    #[test]
    fn the_review_follows_the_document_through_undo_and_redo() {
        let mut row = row();
        let base = row.base_digest();
        let (_, layer) = decide(&mut row, None, Some(1), Action::Edit { text: "١٢٣٤".into() }).expect("edit");
        let edited = layer.expect("written").digest();
        let rows = [(4, &row)];
        // The document has the correction.
        let now = resolve(&rows, state(edited)).expect("known");
        assert_eq!((now.node, now.view.words[1].text.as_str(), now.stale), (Some(0), "١٢٣٤", false));
        // Undone: flagged again.
        let undone = resolve(&rows, state(base)).expect("known");
        assert_eq!(undone.node, None);
        assert!(undone.view.needs_decision());
        // Decided on the undone branch, then redone: the redo's branch shows.
        let mut row = row.clone();
        decide(&mut row, None, Some(2), Action::Check).expect("check");
        let rows = [(4, &row)];
        assert_eq!(resolve(&rows, state(base)).expect("known").view.words[2].review, WordReview::Checked);
        let redone = resolve(&rows, state(edited)).expect("known");
        assert_eq!(redone.view.words[2].review, WordReview::Open);
        assert_eq!(redone.view.words[1].text, "١٢٣٤");
        // No layer, or a draft being read again: nothing to review.
        assert!(resolve(&rows, LayerState::default()).is_none());
        let draft = LayerState { quality: Some(LayerQuality::Draft), ..state(base) };
        assert!(resolve(&rows, draft).is_none());
        // A layer we have no record of: shown, but marked stale.
        assert!(resolve(&rows, state(12345)).expect("shown").stale);
        // Converted from the corrected layer.
        let converted = LayerState { quality: None, digest: None, converted: Some(edited) };
        let shown = resolve(&rows, converted).expect("converted");
        assert!(shown.converted && !shown.stale);
    }

    #[test]
    fn checking_a_page_closes_what_is_left_on_it() {
        let mut row = row();
        row.truncated = true;
        let (view, layer) = decide(&mut row, None, None, Action::CheckPage).expect("page");
        assert!(layer.is_none());
        assert!(view.checked && !view.needs_decision());
        assert_eq!(view.words[1].review, WordReview::Checked);
        assert_eq!(view.words[0].review, WordReview::Open, "not flagged: untouched");
    }

    #[test]
    fn newer_results_win_and_old_results_find_their_places() {
        let mut old = row();
        for w in &mut old.words {
            w.at = None;
        }
        let view = old.view(None);
        assert_eq!(
            view.words.iter().map(|w| w.at).collect::<Vec<_>>(),
            [Some([0, 0]), Some([0, 1]), Some([1, 0]), Some([1, 1])]
        );
        let mut newer = row();
        newer.words[1].text = "١٢٣٤".into();
        newer.layer.lines[0].words[1].text = "١٢٣٤".into();
        let d = newer.base_digest();
        let rows = [(1, &old), (2, &newer)];
        assert_eq!(resolve(&rows, state(d)).expect("found").job, 2);
        assert_eq!(resolve(&rows, state(old.base_digest())).expect("found").job, 1);
        // A layer without a digest (written before digests): the newest.
        let undigested = LayerState { quality: Some(LayerQuality::Final), digest: None, converted: None };
        let shown = resolve(&rows, undigested).expect("found");
        assert_eq!((shown.job, shown.stale), (2, false));
    }
}
