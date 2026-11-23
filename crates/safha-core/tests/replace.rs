//! Find and replace (ADR 0052), on real files: every match of a search is
//! replaced through the in-place editor, in the paragraph's own font when it
//! can be and a reported substitute when not; matches that cannot be
//! replaced exactly are listed with why; replace-all is one update; turned
//! pages, encrypted files, and save and reopen work; Arabic and mixed lines
//! read back in logical order, in our extractor and in PDFium's.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::manifest;
use safha_core::block_edit::TextBlock;
use safha_core::fonts::FontFamily;
use safha_core::replace::{Occurrence, Plan, Report, SkipReason, Target, apply, occurrence_at, plan};
use safha_core::save::save;
use safha_core::search::{DocumentIndex, Query, SearchOptions};
use safha_core::text_edit::NewText;
use safha_core::typeset::{Align, BaseDirection};
use safha_core::{Document, Engine, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn fixture(name: &str) -> (Vec<u8>, Option<String>) {
    let manifest = manifest();
    let fixture = manifest
        .fixture_or_unavailable(name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    (std::fs::read(fixture.file()).expect("fixture readable"), fixture.password.clone())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-replace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf_check(bytes: &[u8], name: &str, password: Option<&str>) {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("write");
    let mut command = Command::new("qpdf");
    if let Some(password) = password {
        command.arg(format!("--password={password}"));
    }
    let output = command.arg("--check").arg(&path).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The page's text as our extractor reads it over PDFium (another viewer's view).
fn page_text(document: &Document, page: usize) -> String {
    normalize(&document.page_text(page).expect("text").text())
}

fn index(document: &Document) -> DocumentIndex {
    DocumentIndex::build(document, || false).expect("indexes").expect("not cancelled")
}

fn query(text: &str, options: SearchOptions) -> Query {
    Query::new(text, options).expect("valid query")
}

fn plan_all(document: &Document, query: &Query, template: &str) -> Plan {
    plan(document, &index(document), query, template, None, 5_000).expect("plans")
}

/// Plans and replaces every replaceable match: the new revision, its update, and the report.
fn replace_all(document: &Document, query: &Query, template: &str) -> (Plan, Document, Vec<u8>, Report) {
    let plan = plan_all(document, query, template);
    let (done, report) = apply(document, query, template, &plan.targets()).expect("applies");
    let (revision, update) = done.expect("something was replaced");
    (plan, revision, update, report)
}

fn blocks_with(document: &Document, page: usize, needle: &str) -> Vec<TextBlock> {
    document
        .text_blocks(page)
        .expect("blocks")
        .into_iter()
        .filter(|b| normalize(&b.text).contains(needle))
        .collect()
}

fn add_text_on(document: &Document, page: usize, text: &str, width: f32, y: f32) -> Document {
    let new = NewText {
        page,
        x: 72.0,
        y,
        width,
        text: text.into(),
        family: FontFamily::Sans,
        size: 14.0,
        color: [0.0, 0.0, 0.0],
        align: Align::Start,
        direction: BaseDirection::Auto,
    };
    document.with_new_text(&new).expect("adds").0
}

/// A file of pages with the given contents, all using a Helvetica with metrics as `/F1`.
fn pdf(pages: &[&str]) -> Vec<u8> {
    let widths = vec!["556"; 95].join(" ");
    let first_page = 4;
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", first_page + 2 * i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()),
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths [{widths}] >>"
        ),
    ];
    for (i, content) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 400] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
            first_page + 2 * i + 1
        ));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    pdf
}

const CONTRACT_1: &str = "BT /F1 12 Tf 14 TL 40 340 Td (The Contractor shall deliver the goods) Tj T* (to the Client before the agreed date.) Tj ET\nBT /F1 12 Tf 40 280 Td (Payment to the Contractor is due in 30 days.) Tj ET\n";
const CONTRACT_2: &str = "BT /F1 12 Tf 40 340 Td (Notices to the contractor go by mail.) Tj ET\n";

fn ready(plan: &Plan) -> Vec<&Occurrence> {
    plan.occurrences.iter().filter(|o| o.target.is_some()).collect()
}

// ------------------------------------------------------------------ English

#[test]
fn replace_all_in_english_sets_each_paragraph_again_in_its_own_font_as_one_update() {
    let document = engine().open_bytes(pdf(&[CONTRACT_1, CONTRACT_2]), None).expect("opens");
    let contractor = query("contractor", SearchOptions::default());

    // The preview: every match, with its context and what it becomes.
    let preview = plan_all(&document, &contractor, "Supplier");
    assert_eq!(preview.occurrences.len(), 3, "{preview:#?}");
    assert!(preview.occurrences.iter().all(|o| o.skip.is_none() && o.replacement == "Supplier"));
    assert_eq!(preview.occurrences.iter().map(|o| o.page).collect::<Vec<_>>(), [0, 0, 1]);
    let first = &preview.occurrences[0];
    assert_eq!(
        (first.context.before.as_str(), first.context.matched.as_str()),
        ("The ", "Contractor"),
        "{first:?}"
    );
    assert!(first.context.after.starts_with(" shall deliver"));
    // Its box is where PDFium sees the word.
    let hits = index(&document).search(&contractor);
    let view = document.view_transform(0).expect("view");
    let seen = view.rect(hits[0].1.rects[0]);
    let ours = first.rects[0];
    assert!(
        (ours.x - seen.x).abs() < 2.0
            && (ours.y + ours.height / 2.0 - (seen.y + seen.height / 2.0)).abs() < 4.0
    );

    let (done, report) = apply(&document, &contractor, "Supplier", &preview.targets()).expect("applies");
    let (edited, update) = done.expect("replaced");
    assert_eq!((report.replaced, report.paragraphs), (3, 3), "{report:?}");
    assert!(report.skipped.is_empty() && report.substituted.is_empty(), "{report:?}");
    // One update on top of the file as it was: one undo step. Helvetica set it: nothing embedded.
    let bytes = edited.bytes();
    assert_eq!(&bytes[..document.bytes().len()], &document.bytes()[..]);
    assert_eq!(bytes.len(), document.bytes().len() + update.len());
    assert!(!String::from_utf8_lossy(&update).contains("/FontFile"));
    // In a file that is not encrypted, every new stream is there to inflate (the
    // check the encrypted test relies on sees streams in the clear).
    let (streams, clear) = common::streams_in_the_clear(&update);
    assert!(streams > 0 && clear == streams, "{clear} of {streams}");

    let page0 = page_text(&edited, 0);
    assert!(page0.contains("The Supplier shall deliver the goods"), "{page0}");
    assert!(page0.contains("Payment to the Supplier is due in 30 days."), "{page0}");
    assert!(page_text(&edited, 1).contains("Notices to the Supplier go by mail."));
    assert!(index(&edited).search(&contractor).is_empty());
    qpdf_check(&bytes, "english.pdf", None);

    // Saved and opened again, it reads the same; nothing is left to replace.
    let target = scratch("english.pdf");
    save(&edited, &target).expect("saves");
    let reopened = engine().open_file(&target, None).expect("reopens");
    assert!(page_text(&reopened, 0).contains("The Supplier shall deliver the goods"));
    assert!(plan_all(&reopened, &contractor, "Supplier").occurrences.is_empty());
}

#[test]
fn only_the_chosen_matches_are_replaced() {
    let document = engine().open_bytes(pdf(&[CONTRACT_1, CONTRACT_2]), None).expect("opens");
    let contractor = query("contractor", SearchOptions::default());
    let preview = plan_all(&document, &contractor, "Vendor");
    // The second match only: its paragraph is set again; the others are left as they were.
    let chosen = [preview.occurrences[1].target.expect("replaceable")];
    let (done, report) = apply(&document, &contractor, "Vendor", &chosen).expect("applies");
    let (edited, _) = done.expect("replaced");
    assert_eq!(report.replaced, 1);
    let page0 = page_text(&edited, 0);
    assert!(page0.contains("The Contractor shall deliver") && page0.contains("Payment to the Vendor is due"));
    assert!(page_text(&edited, 1).contains("the contractor go by mail"));
}

#[test]
fn a_hit_found_in_search_names_its_match() {
    let document = engine().open_bytes(pdf(&[CONTRACT_1]), None).expect("opens");
    let contractor = query("contractor", SearchOptions::default());
    let built = index(&document);
    let view = built.view(0).expect("view");
    let page = plan(&document, &built, &contractor, "X", Some(0), 100).expect("plans");
    for (_, hit) in built.search(&contractor) {
        let rects: Vec<_> = hit.rects.iter().map(|r| view.rect(*r)).collect();
        let occurrence = occurrence_at(&page.occurrences, &rects).expect("the hit's match");
        assert!(occurrence.rects.iter().any(|r| (r.x - rects[0].x).abs() < 2.0));
    }
    assert!(
        occurrence_at(
            &page.occurrences,
            &[safha_core::view::ViewRect { x: 1.0, y: 1.0, width: 1.0, height: 1.0 }]
        )
        .is_none()
    );
}

#[test]
fn a_match_that_changed_since_the_preview_is_skipped() {
    let document = engine().open_bytes(pdf(&[CONTRACT_1]), None).expect("opens");
    let contractor = query("contractor", SearchOptions::default());
    let preview = plan_all(&document, &contractor, "Vendor");
    let mut stale: Target = preview.occurrences[0].target.expect("replaceable");
    stale.fingerprint ^= 1;
    let mut moved: Target = preview.occurrences[1].target.expect("replaceable");
    moved.start += 1;
    let (done, report) = apply(&document, &contractor, "Vendor", &[stale, moved]).expect("applies");
    assert!(done.is_none());
    assert_eq!(report.skipped.iter().map(|s| s.reason).collect::<Vec<_>>(), [SkipReason::Changed; 2]);
}

#[test]
fn a_hit_that_runs_from_one_paragraph_into_another_is_listed_and_skipped() {
    // "due now" drawn 5 pt below "Total amount", a word space after it (a
    // baseline shift, as a rise or a subscript makes): one line to search,
    // which allows half an em; two paragraphs to the editor, which a third.
    let content = "BT /F1 12 Tf 40 300 Td (Total amount) Tj ET\nBT /F1 12 Tf 125 295 Td (due now) Tj ET\n";
    let document = engine().open_bytes(pdf(&[content]), None).expect("opens");
    let across = query("amount due", SearchOptions::default());
    assert_eq!(
        index(&document).search(&across).len(),
        1,
        "search sees one line: {}",
        page_text(&document, 0)
    );
    let preview = plan_all(&document, &across, "sum due");
    assert_eq!(preview.occurrences.len(), 1, "{preview:#?}");
    let only = &preview.occurrences[0];
    assert_eq!((only.target, only.skip), (None, Some(SkipReason::CrossesParagraphs)));
    assert_eq!(only.context.matched, "amount due");
    assert_eq!(only.replacement, "sum due");
}

#[test]
fn text_that_cannot_be_edited_is_listed_with_why() {
    // Chrome's fixtures draw text in Type 3 fonts, which the editor does not read.
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let the = query("the", SearchOptions { whole_word: true, ..SearchOptions::default() });
    let preview = plan_all(&document, &the, "a");
    assert!(!preview.occurrences.is_empty());
    assert!(
        preview.occurrences.iter().all(|o| o.target.is_none() && o.skip == Some(SkipReason::Font)),
        "{:#?}",
        preview.occurrences.iter().map(|o| (o.skip, &o.context.matched)).collect::<Vec<_>>()
    );
    let (done, report) = apply(&document, &the, "a", &preview.targets()).expect("applies");
    assert!(done.is_none() && report.replaced == 0);
}

// ------------------------------------------------------------------- Arabic

const ARABIC: &str = "يجب أن يصل الطلب إلى المدير قبل نهاية الأسبوع، ثم يرسل الى الإدارة المالية للموافقة.";

#[test]
fn arabic_spellings_found_by_normalised_matching_are_replaced_and_read_back_in_logical_order() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    let document = add_text_on(&original, 0, ARABIC, 300.0, 560.0);
    // "الى" finds both "إلى" and "الى": alef forms are one letter to search.
    // As a whole word: folded, it is also inside "المالية".
    let ila = query("الى", SearchOptions { whole_word: true, ..SearchOptions::default() });
    let (preview, edited, _, report) = replace_all(&document, &ila, "إلى");
    assert_eq!(ready(&preview).len(), 2, "{preview:#?}");
    assert!(preview.occurrences.iter().all(|o| o.rtl));
    assert_eq!(report.replaced, 2);
    // One paragraph, set again once.
    assert_eq!(report.paragraphs, 1);
    let expected = ARABIC.replace("الى الإدارة", "إلى الإدارة");
    let block = &blocks_with(&edited, 0, "الإدارة المالية")[0];
    assert_eq!(normalize(&block.text), expected);
    assert!(block.rtl && block.editable);
    assert!(page_text(&edited, 0).contains(&expected), "{}", page_text(&edited, 0));
    qpdf_check(&edited.bytes(), "arabic.pdf", None);
}

#[test]
fn a_replacement_the_paragraphs_font_lacks_is_set_in_a_reported_substitute() {
    let document = engine().open_bytes(pdf(&[CONTRACT_2]), None).expect("opens");
    // Helvetica has no Arabic: the paragraph is set in a bundled face, and the report says so.
    let mail = query("mail", SearchOptions { whole_word: true, ..SearchOptions::default() });
    let (_, edited, update, report) = replace_all(&document, &mail, "البريد");
    assert_eq!((report.replaced, report.substituted_paragraphs), (1, 1), "{report:?}");
    assert_eq!(report.substituted[0].from, "Helvetica");
    assert!(String::from_utf8_lossy(&update).contains("/FontFile"), "a subset is embedded");
    assert!(
        page_text(&edited, 0).contains("Notices to the contractor go by البريد."),
        "{}",
        page_text(&edited, 0)
    );
}

// -------------------------------------------------------------------- Mixed

const MIXED: &str = "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال للجهاز، بموجب القرار رقم ١٢٣٤.";

#[test]
fn replacements_in_a_mixed_line_keep_its_order() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    let document = add_text_on(&original, 0, MIXED, 420.0, 560.0);

    // An English name inside Arabic becomes an Arabic one.
    let name = query("iPhone 15 Pro", SearchOptions::default());
    let (_, step1, _, report) = replace_all(&document, &name, "جالاكسي S24");
    assert_eq!(report.replaced, 1);
    let expected1 = MIXED.replace("iPhone 15 Pro", "جالاكسي S24");
    assert_eq!(normalize(&blocks_with(&step1, 0, "S24")[0].text), expected1);
    assert!(page_text(&step1, 0).contains(&expected1), "{}", page_text(&step1, 0));

    // A regular expression with groups: Western and Arabic-Indic digits alike, kept as written.
    let regex = SearchOptions { regex: true, ..SearchOptions::default() };
    let numbered = query(r"(\d+) أجهزة|رقم (\d+)", regex);
    let (preview, step2, _, report) = replace_all(&step1, &numbered, "${1}${2} وحدات");
    assert_eq!(ready(&preview).len(), 2, "{preview:#?}");
    assert_eq!(preview.occurrences[0].replacement, "3 وحدات");
    assert_eq!(report.replaced, 2);
    let expected2 = expected1.replace("3 أجهزة", "3 وحدات").replace("رقم ١٢٣٤", "١٢٣٤ وحدات");
    assert_eq!(normalize(&blocks_with(&step2, 0, "S24")[0].text), expected2);
    assert!(page_text(&step2, 0).contains(&expected2), "{}", page_text(&step2, 0));
}

// ------------------------------------------------- turned pages, encrypted files

#[test]
fn turned_pages_are_replaced_as_they_are_displayed() {
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    let text = "تمت الموافقة على العقد من الإدارة المالية بتاريخ اليوم";
    for page in 0..4 {
        document = add_text_on(&document, page, text, 220.0, 40.0);
    }
    let finance = query("الإدارة المالية", SearchOptions::default());
    let (preview, edited, _, report) = replace_all(&document, &finance, "الإدارة العامة");
    // Each page's paragraph, found where PDFium sees it: nothing listed twice or skipped.
    assert_eq!(preview.occurrences.len(), 4, "{preview:#?}");
    assert_eq!(ready(&preview).len(), 4);
    assert_eq!(report.replaced, 4);
    let expected = text.replace("الإدارة المالية", "الإدارة العامة");
    for page in 0..4 {
        assert_eq!(normalize(&blocks_with(&edited, page, "العامة")[0].text), expected, "page {page}");
        assert!(page_text(&edited, page).contains(&expected), "page {page}: {}", page_text(&edited, page));
    }
    qpdf_check(&edited.bytes(), "turned.pdf", None);
}

#[test]
fn encrypted_files_are_replaced_in_and_reopen_with_their_password() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let original = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let document =
        add_text_on(&original, 0, "The Contractor shall deliver the goods to the Client.", 300.0, 560.0);
    let contractor = query("contractor", SearchOptions::default());
    let (_, edited, update, report) = replace_all(&document, &contractor, "Supplier");
    assert_eq!(report.replaced, 1, "{report:?}");
    let (streams, clear) = common::streams_in_the_clear(&update);
    assert!(streams > 0 && clear == 0, "{clear} of {streams} streams written in the clear");
    assert!(page_text(&edited, 0).contains("The Supplier shall deliver the goods to the Client."));
    qpdf_check(&edited.bytes(), "encrypted.pdf", password.as_deref());

    let target = scratch("encrypted.pdf");
    save(&edited, &target).expect("saves");
    assert!(engine().open_file(&target, None).is_err(), "still encrypted");
    let reopened = engine().open_file(&target, password.as_deref()).expect("reopens");
    assert!(page_text(&reopened, 0).contains("The Supplier shall deliver the goods"));
    assert!(plan_all(&reopened, &contractor, "Supplier").occurrences.is_empty());
}
