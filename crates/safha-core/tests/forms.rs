//! Form filling (ADR 0045) on a hand-written AcroForm document: values are
//! set, every widget gets an appearance of ours that PDFium draws inside the
//! widget and nowhere else, Arabic reads back in logical order once the
//! appearances are merged into the page, and everything survives a save
//! through PDFium and reopening — in English and Arabic, in a plain and an
//! encrypted file, and in one that asked viewers to draw its fields.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::form_pdf::{ADDRESS, AGREE, CITY, ID, LANGS, LOCKED, NAME, RADIO_A, RADIO_B, form_pdf};
use safha_core::forms::{FieldKind, FormEdit, FormField};
use safha_core::save::save;
use safha_core::{Bitmap, Document, Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-forms-{}", std::process::id()));
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

fn open(bytes: Vec<u8>, password: Option<&str>) -> Document {
    engine().open_bytes(bytes, password).expect("opens")
}

fn field<'a>(fields: &'a [FormField], name: &str) -> &'a FormField {
    fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no field {name}"))
}

fn fill(document: &Document, name: &str, values: &[&str]) -> Document {
    let form = document.form().expect("form");
    let id = field(&form.fields, name).id;
    let edit = FormEdit { field: id, values: values.iter().map(|v| v.to_string()).collect() };
    document.with_form_edits(&[edit]).unwrap_or_else(|err| panic!("{name}: {err}")).0
}

fn try_fill(document: &Document, name: &str, values: &[&str]) -> Result<Document, Error> {
    let id = field(&document.form().expect("form").fields, name).id;
    let edit = FormEdit { field: id, values: values.iter().map(|v| v.to_string()).collect() };
    document.with_form_edits(&[edit]).map(|(document, _)| document)
}

const SCALE: f32 = 2.0;
const PAGE_HEIGHT: f32 = 792.0;

fn render(document: &Document) -> Bitmap {
    document.render_page(0, SCALE).expect("renders")
}

/// Pixels that changed inside a page-space rectangle, and outside every one of `allowed`.
fn changes(before: &Bitmap, after: &Bitmap, inside: [f32; 4], allowed: &[[f32; 4]]) -> (usize, usize) {
    let within = |r: [f32; 4], x: f32, y: f32| {
        x >= r[0] - 1.0 && x <= r[2] + 1.0 && y >= PAGE_HEIGHT - r[3] - 1.0 && y <= PAGE_HEIGHT - r[1] + 1.0
    };
    let (mut changed_inside, mut changed_outside) = (0, 0);
    for y in 0..after.height() {
        for x in 0..after.width() {
            if before.pixel(x, y) == after.pixel(x, y) {
                continue;
            }
            let (vx, vy) = (x as f32 / SCALE, y as f32 / SCALE);
            if within(inside, vx, vy) {
                changed_inside += 1;
            } else if !allowed.iter().any(|r| within(*r, vx, vy)) {
                changed_outside += 1;
            }
        }
    }
    (changed_inside, changed_outside)
}

/// The document with its annotations (widgets included) merged into the page.
fn flattened(document: &Document, password: Option<&str>, name: &str) -> Document {
    let input = scratch(&format!("{name}-in.pdf"));
    let output = scratch(&format!("{name}-flat.pdf"));
    std::fs::write(&input, document.bytes()).expect("write");
    let password = format!("--password={}", password.unwrap_or(""));
    let (input, output) = (input.to_string_lossy().into_owned(), output.to_string_lossy().into_owned());
    qpdf(&[&password, "--check", &input]);
    qpdf(&[&password, "--decrypt", "--flatten-annotations=all", &input, &output]);
    engine().open_file(std::path::Path::new(&output), None).expect("the flattened file opens")
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn fields_are_read_with_their_kinds_values_and_choices() {
    let form = open(form_pdf(false), None).form().expect("form");
    assert!(!form.xfa && !form.need_appearances);
    let names: Vec<&str> = form.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["applicant.name", "address", "agree", "choice", "city", "langs", "id", "locked"]);
    let kinds: Vec<FieldKind> = form.fields.iter().map(|f| f.kind).collect();
    use FieldKind::*;
    assert_eq!(kinds, [Text, Text, Checkbox, Radio, Combo, List, Text, Text]);

    let address = field(&form.fields, "address");
    assert!(address.multiline && !address.read_only);
    let agree = field(&form.fields, "agree");
    assert_eq!(agree.options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["Yes"]);
    assert!(agree.values.is_empty(), "unchecked");
    let choice = field(&form.fields, "choice");
    assert_eq!(choice.widgets.len(), 2);
    assert_eq!(choice.options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["0", "1"]);
    let city = field(&form.fields, "city");
    assert_eq!(city.options[0].value, "ruh");
    assert_eq!(city.options[0].label, "الرياض");
    let langs = field(&form.fields, "langs");
    assert!(langs.multi_select);
    let id = field(&form.fields, "id");
    assert!(id.comb && id.max_len == Some(10));
    let locked = field(&form.fields, "locked");
    assert!(locked.read_only);
    assert_eq!(locked.values, ["سري للغاية"]);

    // Each widget on the page, where it is displayed.
    let shown = field(&form.fields, "applicant.name").widgets[0].view.expect("on the page");
    assert_eq!((shown.x, shown.y, shown.width, shown.height), (100.0, 68.0, 300.0, 24.0));
}

#[test]
fn text_in_english_and_arabic_is_drawn_inside_its_field_and_reads_back() {
    let document = open(form_pdf(false), None);
    let name = "محمد بن عبدالله العلي";
    let address = "شارع الملك فهد ١٢\nRiyadh 12211, Saudi Arabia";
    let before = render(&document);
    let filled = fill(&document, "applicant.name", &[name]);
    let after = render(&filled);
    let (inside, outside) = changes(&before, &after, NAME, &[]);
    assert!(inside > 300, "{inside} pixels changed in the field");
    assert_eq!(outside, 0, "nothing changes outside it");

    let before = after;
    let filled = fill(&filled, "address", &[address]);
    let (inside, outside) = changes(&before, &render(&filled), ADDRESS, &[]);
    assert!(inside > 300 && outside == 0, "{inside} inside, {outside} outside");

    let form = filled.form().expect("form");
    assert_eq!(field(&form.fields, "applicant.name").values, [name]);
    assert_eq!(field(&form.fields, "address").values, [address]);

    // Merged into the page, the fields read in logical order, as typed.
    let flat = flattened(&filled, None, "text");
    let read = normalize(&flat.page_text(0).expect("text").text());
    assert!(read.contains(name), "{read}");
    assert!(read.contains("شارع الملك فهد ١٢") && read.contains("Riyadh 12211, Saudi Arabia"), "{read}");
}

#[test]
fn values_and_appearances_survive_saving_and_reopening() {
    let document = open(form_pdf(false), None);
    let filled = fill(&document, "applicant.name", &["Sarah Al-Harbi — سارة الحربي"]);
    let filled = fill(&filled, "agree", &["Yes"]);
    let filled = fill(&filled, "city", &["jed"]);
    let rendered = render(&filled);
    let path = scratch("saved.pdf");
    save(&filled, &path).expect("saves");
    let reopened = engine().open_file(&path, None).expect("reopens");
    let form = reopened.form().expect("form");
    assert_eq!(field(&form.fields, "applicant.name").values, ["Sarah Al-Harbi — سارة الحربي"]);
    assert_eq!(field(&form.fields, "agree").values, ["Yes"]);
    assert_eq!(field(&form.fields, "city").values, ["jed"]);
    // Drawn the same after the round trip.
    let (inside, outside) = changes(&rendered, &render(&reopened), [0.0, 0.0, 612.0, 792.0], &[]);
    assert_eq!(inside + outside, 0, "the saved file looks the same");
}

#[test]
fn check_boxes_and_radio_buttons_switch_states() {
    let document = open(form_pdf(false), None);
    let before = render(&document);
    let checked = fill(&document, "agree", &["Yes"]);
    let (inside, outside) = changes(&before, &render(&checked), AGREE, &[]);
    assert!(inside > 50 && outside == 0, "{inside} inside, {outside} outside");
    assert_eq!(field(&checked.form().expect("form").fields, "agree").values, ["Yes"]);
    let unchecked = fill(&checked, "agree", &[]);
    assert!(field(&unchecked.form().expect("form").fields, "agree").values.is_empty());

    // Radio buttons without appearances get ours: the chosen one shows a dot.
    let before = render(&unchecked);
    let chosen = fill(&unchecked, "choice", &["1"]);
    let after = render(&chosen);
    let (dot, outside) = changes(&before, &after, RADIO_B, &[RADIO_A]);
    assert!(dot > 30 && outside == 0, "{dot} in the chosen button, {outside} outside both");
    assert_eq!(field(&chosen.form().expect("form").fields, "choice").values, ["1"]);
    let other = fill(&chosen, "choice", &["0"]);
    let (now_on, _) = changes(&after, &render(&other), RADIO_A, &[RADIO_B]);
    assert!(now_on > 30, "the other button is on");
    assert!(matches!(try_fill(&other, "choice", &["7"]), Err(Error::InvalidEdit(_))));
}

#[test]
fn choices_show_their_labels_and_refuse_unknown_values() {
    let document = open(form_pdf(false), None);
    let before = render(&document);
    let city = fill(&document, "city", &["ruh"]);
    let (inside, outside) = changes(&before, &render(&city), CITY, &[]);
    assert!(inside > 100 && outside == 0, "{inside} inside, {outside} outside");
    let flat = flattened(&city, None, "city");
    assert!(
        normalize(&flat.page_text(0).expect("text").text()).contains("الرياض"),
        "the label, not the value"
    );

    let langs = fill(&city, "langs", &["Arabic", "French"]);
    assert_eq!(field(&langs.form().expect("form").fields, "langs").values, ["Arabic", "French"]);
    let (inside, outside) = changes(&render(&city), &render(&langs), LANGS, &[]);
    assert!(inside > 100 && outside == 0, "{inside} inside, {outside} outside");

    assert!(matches!(try_fill(&document, "city", &["mecca"]), Err(Error::InvalidEdit(_))));
    assert!(matches!(try_fill(&document, "langs", &["German"]), Err(Error::InvalidEdit(_))));
    assert!(matches!(try_fill(&document, "locked", &["x"]), Err(Error::InvalidEdit(_))));
    assert!(matches!(try_fill(&document, "id", &["12345678901"]), Err(Error::InvalidEdit(_))));
}

#[test]
fn comb_fields_spread_their_characters() {
    let document = open(form_pdf(false), None);
    let before = render(&document);
    let filled = fill(&document, "id", &["١٢٣٤٥٦٧٨٩٠"]);
    let after = render(&filled);
    let (inside, outside) = changes(&before, &after, ID, &[]);
    assert!(inside > 100 && outside == 0, "{inside} inside, {outside} outside");
    // Ink in every one of the ten cells.
    let cell = (ID[2] - ID[0]) / 10.0;
    for i in 0..10 {
        let x0 = ID[0] + cell * i as f32;
        let (ink, _) = changes(&before, &after, [x0 + 2.0, ID[1], x0 + cell - 2.0, ID[3]], &[ID]);
        assert!(ink > 5, "cell {i} is empty");
    }
}

#[test]
fn a_form_that_asked_viewers_to_draw_it_is_drawn_by_us() {
    let document = open(form_pdf(true), None);
    assert!(document.form().expect("form").need_appearances);
    let filled = fill(&document, "applicant.name", &["Ali"]);
    let form = filled.form().expect("form");
    assert!(!form.need_appearances, "the flag is turned off");
    // The read-only field's value, which had no appearance, now has ours.
    let flat = flattened(&filled, None, "need");
    let read = normalize(&flat.page_text(0).expect("text").text());
    assert!(read.contains("سري للغاية") && read.contains("Ali"), "{read}");
    // And PDFium draws it in its box.
    let bare = open(form_pdf(false), None);
    let (inside, _) = changes(&render(&bare), &render(&filled), LOCKED, &[NAME, AGREE, RADIO_A, RADIO_B]);
    assert!(inside > 100, "{inside}");
}

#[test]
fn fields_in_encrypted_files() {
    let plain = scratch("plain-form.pdf");
    let encrypted = scratch("encrypted-form.pdf");
    std::fs::write(&plain, form_pdf(false)).expect("write");
    let (plain_s, encrypted_s) =
        (plain.to_string_lossy().into_owned(), encrypted.to_string_lossy().into_owned());
    qpdf(&["--encrypt", "user-pw", "owner-pw", "256", "--", &plain_s, &encrypted_s]);
    let bytes = std::fs::read(&encrypted).expect("read");
    let document = open(bytes, Some("user-pw"));
    let form = document.form().expect("form");
    assert_eq!(field(&form.fields, "city").options[1].label, "جدة", "strings are decrypted");
    let filled = fill(&document, "applicant.name", &["نورة القحطاني"]);
    assert_eq!(field(&filled.form().expect("form").fields, "applicant.name").values, ["نورة القحطاني"]);
    let flat = flattened(&filled, Some("user-pw"), "encrypted");
    assert!(normalize(&flat.page_text(0).expect("text").text()).contains("نورة القحطاني"));
}

#[test]
fn several_fields_are_one_update() {
    let document = open(form_pdf(false), None);
    let form = document.form().expect("form");
    let edits = vec![
        FormEdit { field: field(&form.fields, "applicant.name").id, values: vec!["Omar".into()] },
        FormEdit { field: field(&form.fields, "agree").id, values: vec!["Yes".into()] },
    ];
    let (filled, update) = document.with_form_edits(&edits).expect("applies");
    assert!(update.starts_with(b"\n") || update.starts_with(b"1") || !update.is_empty());
    let form = filled.form().expect("form");
    assert_eq!(field(&form.fields, "applicant.name").values, ["Omar"]);
    assert_eq!(field(&form.fields, "agree").values, ["Yes"]);
}
