//! Making forms (ADR 0047): fields of every kind added to a page that had no
//! form, drawn by us (Arabic shaped, at the start of its own direction),
//! filled here, by PDFium, and by PDFKit; moved, resized, renamed, reordered
//! for Tab, and removed; in a plain, a turned, and an encrypted file; saved
//! and reopened. And the assist that finds fields on a flat form.

use std::path::{Path, PathBuf};
use std::process::Command;

use mirsam_core::fonts::FontFamily;
use mirsam_core::forms::{FieldAlign, FieldDesign, FieldKind, FieldOption, FieldSpec, FormEdit, FormField};
use mirsam_core::save::save;
use mirsam_core::text_edit::NewText;
use mirsam_core::typeset::{Align, BaseDirection};
use mirsam_core::{Bitmap, Document, Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mirsam-form-design-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf(args: &[&str]) {
    let output = Command::new("qpdf").args(args).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf {args:?} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn qpdf_check(document: &Document, password: Option<&str>, name: &str) {
    let path = scratch(&format!("{name}.pdf"));
    std::fs::write(&path, document.bytes()).expect("write");
    let password = format!("--password={}", password.unwrap_or(""));
    qpdf(&[&password, "--check", &path.to_string_lossy()]);
}

/// One empty page, 612 × 792, turned `rotate` degrees, with a line of
/// Helvetica text; `/Size` and an xref table as most writers leave them.
fn blank_pdf(rotate: i64, content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate {rotate} /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>"
        ),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ];
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    out
}

fn open(bytes: Vec<u8>, password: Option<&str>) -> Document {
    engine().open_bytes(bytes, password).expect("opens")
}

fn spec(kind: FieldKind, name: &str) -> FieldSpec {
    FieldSpec {
        kind,
        name: name.into(),
        tooltip: String::new(),
        required: false,
        read_only: false,
        multiline: false,
        max_len: None,
        default_values: Vec::new(),
        align: FieldAlign::Start,
        font: Some(FontFamily::Sans),
        font_size: 0.0,
        options: Vec::new(),
        multi_select: false,
        editable: false,
        bordered: true,
        caption: String::new(),
    }
}

fn option(value: &str, label: &str) -> FieldOption {
    FieldOption { value: value.into(), label: label.into() }
}

fn design(document: &Document, edits: &[FieldDesign]) -> Document {
    document.with_form_design(edits).unwrap_or_else(|err| panic!("{edits:?}: {err}")).0
}

fn fields(document: &Document) -> Vec<FormField> {
    document.form().expect("form").fields
}

fn field<'a>(fields: &'a [FormField], name: &str) -> &'a FormField {
    fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no field {name} in {fields:?}"))
}

// Boxes (left, bottom, right, top) on the 612 × 792 page.
const NAME: [f32; 4] = [100.0, 700.0, 400.0, 724.0];
const NOTES: [f32; 4] = [100.0, 610.0, 400.0, 690.0];
const AGREE: [f32; 4] = [100.0, 570.0, 116.0, 586.0];
const YES: [f32; 4] = [100.0, 530.0, 116.0, 546.0];
const NO: [f32; 4] = [150.0, 530.0, 166.0, 546.0];
const CITY: [f32; 4] = [100.0, 480.0, 300.0, 504.0];
const LANGS: [f32; 4] = [100.0, 390.0, 300.0, 460.0];
const SEND: [f32; 4] = [100.0, 340.0, 200.0, 370.0];
const AMOUNT: [f32; 4] = [300.0, 340.0, 500.0, 364.0];

/// Every kind of field, on a page that had no form, in one update each.
fn every_kind(document: &Document) -> Document {
    let name = FieldSpec {
        default_values: vec!["محمد علي".into()],
        tooltip: "الاسم الكامل".into(),
        required: true,
        font_size: 12.0,
        ..spec(FieldKind::Text, "الاسم")
    };
    let notes =
        FieldSpec { multiline: true, font: Some(FontFamily::Naskh), ..spec(FieldKind::Text, "notes") };
    let agree = FieldSpec { default_values: vec!["Yes".into()], ..spec(FieldKind::Checkbox, "agree") };
    let answer = FieldSpec {
        options: vec![option("نعم", "نعم")],
        default_values: vec!["نعم".into()],
        ..spec(FieldKind::Radio, "answer")
    };
    let city = FieldSpec {
        options: vec![option("ruh", "الرياض"), option("jed", "جدة"), option("dmm", "")],
        default_values: vec!["jed".into()],
        ..spec(FieldKind::Combo, "city")
    };
    let langs = FieldSpec {
        options: vec![option("ar", "العربية"), option("en", "English"), option("fr", "Français")],
        multi_select: true,
        default_values: vec!["ar".into(), "en".into()],
        font_size: 10.0,
        ..spec(FieldKind::List, "langs")
    };
    let send = FieldSpec { caption: "إرسال".into(), ..spec(FieldKind::PushButton, "send") };
    let amount = FieldSpec {
        align: FieldAlign::End,
        default_values: vec!["١٢٣ ريال".into()],
        max_len: Some(12),
        ..spec(FieldKind::Text, "amount")
    };
    let mut document = design(
        document,
        &[
            FieldDesign::Add { page: 0, rect: NAME, spec: name },
            FieldDesign::Add { page: 0, rect: NOTES, spec: notes },
            FieldDesign::Add { page: 0, rect: AGREE, spec: agree },
            FieldDesign::Add { page: 0, rect: YES, spec: answer },
            FieldDesign::Add { page: 0, rect: CITY, spec: city },
            FieldDesign::Add { page: 0, rect: LANGS, spec: langs },
            FieldDesign::Add { page: 0, rect: SEND, spec: send },
            FieldDesign::Add { page: 0, rect: AMOUNT, spec: amount },
        ],
    );
    let answer = field(&fields(&document), "answer").id;
    document =
        design(&document, &[FieldDesign::AddButton { field: answer, page: 0, rect: NO, value: "لا".into() }]);
    document
}

const SCALE: f32 = 2.0;

fn render(document: &Document) -> Bitmap {
    document.render_page(0, SCALE).expect("renders")
}

/// Dark pixels inside a page-space box (on an unturned page).
fn ink(bitmap: &Bitmap, [l, b, r, t]: [f32; 4]) -> usize {
    let mut count = 0;
    for y in ((792.0 - t) * SCALE) as u32..((792.0 - b) * SCALE) as u32 {
        for x in (l * SCALE) as u32..(r * SCALE) as u32 {
            if let Some([red, green, blue, _]) = bitmap.pixel(x, y)
                && u32::from(red) + u32::from(green) + u32::from(blue) < 200
            {
                count += 1;
            }
        }
    }
    count
}

/// The page with its widgets merged into it, its text read back.
fn flattened_text(document: &Document, password: Option<&str>, name: &str) -> String {
    let input = scratch(&format!("{name}-in.pdf"));
    let output = scratch(&format!("{name}-flat.pdf"));
    std::fs::write(&input, document.bytes()).expect("write");
    let password = format!("--password={}", password.unwrap_or(""));
    let (input, output) = (input.to_string_lossy().into_owned(), output.to_string_lossy().into_owned());
    qpdf(&[&password, "--check", &input]);
    qpdf(&[&password, "--decrypt", "--flatten-annotations=all", &input, &output]);
    let flat = engine().open_file(Path::new(&output), None).expect("the flattened file opens");
    flat.page_text(0).expect("text").text()
}

#[test]
fn fields_of_every_kind_are_made_on_a_page_without_a_form() {
    let blank = open(blank_pdf(0, "BT /F1 12 Tf 72 760 Td (Application) Tj ET"), None);
    assert!(fields(&blank).is_empty());
    let document = every_kind(&blank);
    qpdf_check(&document, None, "every-kind");
    // Incremental: the file as it was is still its start.
    assert!(document.bytes().starts_with(&blank.bytes()));

    let fields = fields(&document);
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["الاسم", "notes", "agree", "answer", "city", "langs", "send", "amount"]);
    use FieldKind::*;
    let kinds: Vec<FieldKind> = fields.iter().map(|f| f.kind).collect();
    assert_eq!(kinds, [Text, Text, Checkbox, Radio, Combo, List, PushButton, Text]);

    let name = field(&fields, "الاسم");
    assert_eq!(name.values, ["محمد علي"]);
    assert_eq!(name.default_values, ["محمد علي"]);
    assert_eq!(name.tooltip, "الاسم الكامل");
    assert!(name.required && !name.read_only);
    assert_eq!((name.font, name.font_size, name.align), (Some(FontFamily::Sans), 12.0, FieldAlign::Start));
    let shown = name.widgets[0].view.expect("on the page");
    assert_eq!((shown.x, shown.y, shown.width, shown.height), (100.0, 68.0, 300.0, 24.0));

    let notes = field(&fields, "notes");
    assert!(notes.multiline && notes.font == Some(FontFamily::Naskh));
    assert_eq!(field(&fields, "agree").values, ["Yes"]);
    // Radio buttons are named by position, their export values in `/Opt`.
    let answer = field(&fields, "answer");
    assert_eq!(answer.widgets.len(), 2);
    assert_eq!(answer.options, [option("0", "نعم"), option("1", "لا")]);
    assert_eq!(answer.values, ["0"]);
    let city = field(&fields, "city");
    assert_eq!(city.options, [option("ruh", "الرياض"), option("jed", "جدة"), option("dmm", "dmm")]);
    assert_eq!(city.values, ["jed"]);
    let langs = field(&fields, "langs");
    assert!(langs.multi_select);
    assert_eq!(langs.values, ["ar", "en"]);
    assert_eq!(field(&fields, "send").caption, "إرسال");
    let amount = field(&fields, "amount");
    assert_eq!((amount.align, amount.max_len), (FieldAlign::End, Some(12)));
    // Tab order follows the order they were made in.
    let tabs: Vec<Option<usize>> = fields.iter().map(|f| f.widgets[0].annot_index).collect();
    assert_eq!(tabs, [Some(0), Some(1), Some(2), Some(3), Some(4), Some(5), Some(6), Some(7)]);
}

#[test]
fn new_fields_are_drawn_inside_their_boxes_and_arabic_reads_back() {
    let blank = open(blank_pdf(0, ""), None);
    let before = render(&blank);
    let document = every_kind(&blank);
    let after = render(&document);
    for (name, rect) in [("name", NAME), ("agree", AGREE), ("yes", YES), ("city", CITY), ("send", SEND)] {
        assert!(ink(&after, rect) > ink(&before, rect) + 20, "{name} is drawn");
    }
    // Nothing between the fields.
    assert_eq!(ink(&after, [420.0, 400.0, 600.0, 700.0]), 0);

    // An Arabic value in a "start" field sits at the right; in an "end" field, at the left.
    let half = |[l, b, r, t]: [f32; 4], right: bool| {
        let mid = (l + r) / 2.0;
        if right { [mid, b, r, t] } else { [l, b, mid, t] }
    };
    assert!(
        ink(&after, half(NAME, true)) > 10 * ink(&after, half(NAME, false)).max(1),
        "start of Arabic: right"
    );
    assert!(
        ink(&after, half(AMOUNT, false)) > 10 * ink(&after, half(AMOUNT, true)).max(1),
        "end of Arabic: left"
    );

    // Shaped and in logical order once merged into the page.
    let text = flattened_text(&document, None, "drawn");
    for expected in ["محمد علي", "جدة", "العربية", "English", "إرسال"] {
        assert!(text.contains(expected), "{expected:?} in {text:?}");
    }
}

#[test]
fn new_fields_are_filled_here_saved_and_reopened() {
    let document = every_kind(&open(blank_pdf(0, ""), None));
    let fields = fields(&document);
    let id = |name: &str| field(&fields, name).id;
    let edits = [
        FormEdit { field: id("الاسم"), values: vec!["Jane Roe".into()] },
        FormEdit { field: id("agree"), values: vec![] },
        FormEdit { field: id("answer"), values: vec!["1".into()] },
        FormEdit { field: id("city"), values: vec!["ruh".into()] },
        FormEdit { field: id("langs"), values: vec!["fr".into()] },
    ];
    let (filled, _) = document.with_form_edits(&edits).expect("fills");
    let path = scratch("filled.pdf");
    save(&filled, &path).expect("saves");
    let reopened = engine().open_file(&path, None).expect("reopens");
    let fields = self::fields(&reopened);
    assert_eq!(field(&fields, "الاسم").values, ["Jane Roe"]);
    assert_eq!(field(&fields, "الاسم").default_values, ["محمد علي"], "the default stays");
    assert!(field(&fields, "agree").values.is_empty());
    assert_eq!(field(&fields, "answer").values, ["1"]);
    assert_eq!(field(&fields, "city").values, ["ruh"]);
    assert_eq!(field(&fields, "langs").values, ["fr"]);
    let text = flattened_text(&reopened, None, "filled");
    assert!(text.contains("Jane Roe") && text.contains("الرياض"), "{text:?}");
}

#[test]
fn fields_are_moved_resized_changed_reordered_and_removed() {
    let document = every_kind(&open(blank_pdf(0, ""), None));
    let all = fields(&document);
    let name = field(&all, "الاسم").id;

    // Moved and resized: its box, drawn again there.
    let moved = design(
        &document,
        &[FieldDesign::SetRect { field: name, widget: 0, rect: [300.0, 720.0, 560.0, 760.0] }],
    );
    let after = render(&moved);
    assert!(ink(&after, [300.0, 720.0, 560.0, 760.0]) > 50);
    assert_eq!(ink(&after, [100.0, 700.0, 290.0, 718.0]), 0, "nothing left where it was");
    assert_eq!(field(&fields(&moved), "الاسم").widgets[0].rect, [300.0, 720.0, 560.0, 760.0]);

    // Changed: renamed, read-only, a new default, set in Naskh.
    let changed = FieldSpec {
        name: "full name".into(),
        read_only: true,
        default_values: vec!["علي".into()],
        font: Some(FontFamily::Naskh),
        font_size: 14.0,
        align: FieldAlign::Center,
        ..spec(FieldKind::Text, "")
    };
    let updated = design(&moved, &[FieldDesign::Update { field: name, spec: changed.clone() }]);
    let all = fields(&updated);
    let renamed = field(&all, "full name");
    assert!(renamed.read_only && !renamed.required);
    assert_eq!(
        (renamed.values.as_slice(), renamed.font, renamed.font_size),
        (&["علي".to_string()][..], Some(FontFamily::Naskh), 14.0)
    );
    assert_eq!(renamed.align, FieldAlign::Center);
    // Names are unique; kinds stay.
    let clash = FieldSpec { name: "notes".into(), ..changed.clone() };
    assert!(matches!(
        updated.with_form_design(&[FieldDesign::Update { field: name, spec: clash }]),
        Err(Error::InvalidEdit(_))
    ));
    let other_kind = FieldSpec { kind: FieldKind::Combo, ..changed };
    assert!(updated.with_form_design(&[FieldDesign::Update { field: name, spec: other_kind }]).is_err());
    let twice = FieldSpec { ..spec(FieldKind::Text, "notes") };
    assert!(updated.with_form_design(&[FieldDesign::Add { page: 0, rect: NAME, spec: twice }]).is_err());

    // Radio buttons renamed (the value follows), one removed.
    let answer = field(&all, "answer");
    let renamed_buttons = FieldSpec {
        options: vec![option("0", "yes"), option("1", "no")],
        default_values: vec!["0".into()],
        ..spec(FieldKind::Radio, "answer")
    };
    let updated = design(&updated, &[FieldDesign::Update { field: answer.id, spec: renamed_buttons }]);
    let answer = field(&fields(&updated), "answer").clone();
    assert_eq!(answer.options, [option("0", "yes"), option("1", "no")]);
    assert_eq!(answer.values, ["0"]);
    let updated = design(&updated, &[FieldDesign::RemoveButton { field: answer.id, widget: 0 }]);
    let answer = field(&fields(&updated), "answer").clone();
    assert_eq!(answer.widgets.len(), 1);
    assert_eq!(answer.options, [option("0", "no")], "the button left is named by its new position");
    assert!(answer.values.is_empty(), "the chosen button went");
    assert!(updated.with_form_design(&[FieldDesign::RemoveButton { field: answer.id, widget: 0 }]).is_err());

    // Tab order: amount first, then city; the rest as they were.
    let all = fields(&updated);
    let (amount, city) = (field(&all, "amount").id, field(&all, "city").id);
    let reordered = design(&updated, &[FieldDesign::TabOrder { page: 0, fields: vec![amount, city] }]);
    let mut order: Vec<(usize, String)> = fields(&reordered)
        .iter()
        .map(|f| (f.widgets[0].annot_index.expect("on the page"), f.name.clone()))
        .collect();
    order.sort();
    let order: Vec<&str> = order.iter().map(|(_, name)| name.as_str()).collect();
    // The group's remaining button was added last, so it comes last.
    assert_eq!(order, ["amount", "city", "full name", "notes", "agree", "langs", "send", "answer"]);
    let bytes = reordered.bytes();
    let tail = String::from_utf8_lossy(&bytes[updated.bytes().len()..]);
    assert!(tail.contains("/Tabs /W"), "the page says its order is the widgets' order");

    // Removed: gone from the form and the page.
    let removed = design(&reordered, &[FieldDesign::Remove { field: city }]);
    assert!(fields(&removed).iter().all(|f| f.name != "city"));
    assert_eq!(fields(&removed).len(), 7);
    assert_eq!(ink(&render(&removed), CITY), 0, "nothing drawn where it was");
    let removed = design(&removed, &[FieldDesign::Remove { field: answer.id }]);
    assert!(fields(&removed).iter().all(|f| f.name != "answer"));
    qpdf_check(&removed, None, "removed");
}

#[test]
fn fields_on_a_turned_page_stand_upright() {
    let blank = open(blank_pdf(90, ""), None);
    let text =
        FieldSpec { default_values: vec!["Upright".into()], font_size: 14.0, ..spec(FieldKind::Text, "t") };
    // On the page turned 90°, a box wide as displayed is tall in page space.
    let document =
        design(&blank, &[FieldDesign::Add { page: 0, rect: [100.0, 100.0, 130.0, 400.0], spec: text }]);
    let shown = field(&fields(&document), "t").widgets[0].view.expect("shown");
    assert!(shown.width > shown.height, "{shown:?}");
    // As displayed, the text runs across the box: its ink is wider than tall.
    let bitmap = document.render_page(0, SCALE).expect("renders");
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in (shown.y * SCALE) as u32 + 4..((shown.y + shown.height) * SCALE) as u32 - 4 {
        for x in (shown.x * SCALE) as u32 + 4..((shown.x + shown.width) * SCALE) as u32 - 4 {
            if let Some([r, g, b, _]) = bitmap.pixel(x, y)
                && u32::from(r) + u32::from(g) + u32::from(b) < 200
            {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    assert!(x1 > x0 && x1 - x0 > 3 * (y1 - y0), "upright text: ink {x0},{y0} to {x1},{y1}");
}

#[test]
fn fields_in_an_encrypted_file() {
    let plain = scratch("plain.pdf");
    let encrypted = scratch("encrypted.pdf");
    std::fs::write(&plain, blank_pdf(0, "")).expect("write");
    let (plain, encrypted) = (plain.to_string_lossy().into_owned(), encrypted.to_string_lossy().into_owned());
    qpdf(&["--encrypt", "user-pw", "owner-pw", "256", "--", &plain, &encrypted]);
    let document = engine().open_file(Path::new(&encrypted), Some("user-pw")).expect("opens");
    let document = every_kind(&document);
    qpdf_check(&document, Some("user-pw"), "encrypted");
    let fields = fields(&document);
    assert_eq!(field(&fields, "الاسم").tooltip, "الاسم الكامل", "strings decrypt");
    assert_eq!(field(&fields, "city").options[0].label, "الرياض");
    let text = flattened_text(&document, Some("user-pw"), "encrypted");
    assert!(text.contains("محمد علي"), "{text:?}");
}

/// A flat form: an English line with its label, a box with a label above
/// it, a check box with its label, a table of two cells, and an Arabic line
/// whose label (written with our text writer) is at its right.
fn flat_form() -> Document {
    let content = "\
        BT /F1 11 Tf 72 702 Td (Name:) Tj ET\n\
        0.5 w 110 700 m 300 700 l S\n\
        BT /F1 11 Tf 72 632 Td (Address) Tj ET\n\
        72 560 328 60 re S\n\
        72 520 12 12 re S\n\
        BT /F1 11 Tf 90 522 Td (I agree) Tj ET\n\
        BT /F1 11 Tf 72 482 Td (Phone) Tj ET\n\
        72 450 m 400 450 l S 72 470 m 400 470 l S 72 450 m 72 470 l S 200 450 m 200 470 l S 400 450 m 400 470 l S\n\
        320 700 m 470 700 l S";
    let document = open(blank_pdf(0, content), None);
    let label = NewText {
        page: 0,
        // Displayed top-left corner; Arabic starts at the box's right edge (545).
        x: 475.0,
        y: 792.0 - 714.0,
        width: 70.0,
        text: "الهاتف".into(),
        family: FontFamily::Naskh,
        size: 11.0,
        color: [0.0; 3],
        align: Align::Start,
        direction: BaseDirection::Auto,
    };
    document.with_new_text(&label).expect("label").0
}

#[test]
fn fields_are_found_on_a_flat_form_and_made_in_one_step() {
    let document = flat_form();
    let found = document.detect_fields(0).expect("detects");
    let summary: Vec<(FieldKind, &str, bool)> =
        found.iter().map(|f| (f.kind, f.name.as_str(), f.multiline)).collect();
    assert_eq!(
        summary,
        [
            (FieldKind::Text, "Name", false),
            (FieldKind::Text, "الهاتف", false),
            (FieldKind::Text, "Address", true),
            (FieldKind::Checkbox, "I agree", false),
            (FieldKind::Text, "Phone", false),
            // A cell with no label beside or above it: the designer names it.
            (FieldKind::Text, "", false),
        ],
        "{found:#?}"
    );
    let name = &found[0];
    assert!((name.rect[0] - 110.0).abs() < 2.0 && (name.rect[2] - 300.0).abs() < 2.0, "{:?}", name.rect);
    assert!(name.view.is_some());

    // Each made into a field (names unique), one update.
    let mut taken = std::collections::HashSet::new();
    let edits: Vec<FieldDesign> = found
        .iter()
        .map(|f| {
            let base = if f.name.is_empty() { "Text" } else { f.name.as_str() };
            let name = mirsam_core::forms::unique_name(base, &taken);
            taken.insert(name.clone());
            FieldDesign::Add {
                page: 0,
                rect: f.rect,
                spec: FieldSpec { multiline: f.multiline, bordered: false, ..spec(f.kind, &name) },
            }
        })
        .collect();
    let (made, _) = document.with_form_design(&edits).expect("made");
    let names: Vec<String> = fields(&made).into_iter().map(|f| f.name).collect();
    assert_eq!(names, ["Name", "الهاتف", "Address", "I agree", "Phone", "Text"]);
    // Found again: nothing, every blank has its field.
    assert!(made.detect_fields(0).expect("detects").is_empty());
}

/// macOS PDFKit (Preview's engine) sees the fields, with their kinds and
/// values, and fills one in a way we read back.
#[test]
fn pdfkit_sees_and_fills_the_new_fields() {
    if !cfg!(target_os = "macos") {
        println!("not checked here: PDFKit (macOS only)");
        return;
    }
    let document = every_kind(&open(blank_pdf(0, ""), None));
    let path = scratch("pdfkit.pdf");
    save(&document, &path).expect("saves");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/pdfkit_form.swift");
    let run = |args: &[&str]| -> String {
        let output = Command::new("xcrun").arg("swift").arg(&script).args(args).output().expect("swift runs");
        assert!(output.status.success(), "PDFKit failed: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let seen = run(&[&path.to_string_lossy()]);
    let rows: Vec<Vec<&str>> = seen.lines().map(|line| line.split('\t').collect()).collect();
    let row =
        |name: &str| rows.iter().find(|row| row[0] == name).unwrap_or_else(|| panic!("{name} in {seen}"));
    assert_eq!(row("الاسم")[1..], ["/Tx", "محمد علي"]);
    assert_eq!(row("city")[1], "/Ch");
    assert_eq!(row("agree")[1..], ["/Btn", "Yes"]);
    assert_eq!(rows.iter().filter(|row| row[0] == "answer").count(), 2, "both radio buttons");

    let out = scratch("pdfkit-filled.pdf");
    let filled = run(&[&path.to_string_lossy(), "fill", "الاسم", "سارة أحمد", &out.to_string_lossy()]);
    assert!(filled.contains("سارة أحمد"), "{filled}");
    let reopened = engine().open_file(&out, None).expect("PDFKit's file opens here");
    let after = self::fields(&reopened);
    assert_eq!(field(&after, "الاسم").values, ["سارة أحمد"]);
    // The rest as we made them, after PDFKit wrote the file again.
    let answer = field(&after, "answer");
    assert_eq!((answer.kind, answer.values.as_slice()), (FieldKind::Radio, &["0".to_string()][..]));
    // PDFKit drops a radio group's `/Opt`: its buttons keep their positions.
    assert_eq!(answer.options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["0", "1"]);
    assert_eq!(field(&after, "city").values, ["jed"]);
    assert_eq!(field(&after, "agree").values, ["Yes"]);
}
