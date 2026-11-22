//! Shared drivers for the fuzz targets (fuzz/README.md).
//!
//! Every target ends in [`exercise`]: the readers safha-core runs on a file's
//! own bytes, without PDFium — the catalog reader, the page tree and the page
//! edits written over it, the content-stream interpreter behind text blocks
//! and redaction, the font and ToUnicode readers, form fields (and filling
//! them in), decryption, sanitizing, and ink strokes.
//! A panic, an arithmetic overflow, a hang or a stack overflow in any of them
//! is a finding; an `Err` is the expected answer to a damaged file.
//!
//! The targets differ in what the fuzzer controls: a whole file (`file`,
//! `open`), the objects of a file whose cross-reference table is always right
//! (`objects`), one page's content stream (`content`), or one font's
//! ToUnicode CMap (`cmap`).

use safha_core::edit::PageEdit;
use safha_core::forms::FormEdit;
use safha_core::redact::{Mark, SanitizeOptions};
use safha_core::text::Rect;
use safha_core::view::ViewTransform;

/// Runs every PDFium-free reader of safha-core over `bytes`.
pub fn exercise(bytes: &[u8]) {
    let _ = safha_core::catalog::catalog_hints(bytes);
    let view = ViewTransform::unrotated(842.0);
    for page in 0..2 {
        let _ = safha_core::block_edit::text_blocks(bytes, &view, page);
        let _ = safha_core::annotate::ink_strokes(bytes, page);
    }
    for edit in edits() {
        if let Ok(update) = safha_core::edit::page_edit_update(bytes, &edit) {
            // The next edit starts from the edited file, so it must read back.
            let edited = [bytes, &update].concat();
            let _ = safha_core::catalog::catalog_hints(&edited);
            let _ = safha_core::block_edit::text_blocks(&edited, &view, 0);
        }
    }
    // Forms are read with the document's password, if any; the fixtures' is
    // `safha-test` (fixtures/generate.mjs), so encrypted seeds decrypt.
    for password in [None, Some("safha-test")] {
        let Ok(form) = safha_core::forms::read(bytes, password) else { continue };
        for field in form.fields.iter().take(4) {
            for values in [vec!["\u{0627}\u{0644} 12 ab".to_string()], Vec::new()] {
                let edit = FormEdit { field: field.id, values };
                let _ = safha_core::forms::form_update(bytes, password, &edit);
            }
        }
    }
    let _ = safha_core::redact::sanitize_update(bytes, SanitizeOptions::default());
    let marks = [Mark {
        page: 0,
        rect: Rect { left: 0.0, bottom: 300.0, right: 400.0, top: 760.0 },
        text: Some("ab".into()),
    }];
    let _ = safha_core::redact::redaction_update(bytes, &marks);
}

fn edits() -> [PageEdit; 5] {
    [
        PageEdit::Rotate { pages: vec![0], quarter_turns: 1 },
        PageEdit::Delete { pages: vec![0] },
        PageEdit::Move { pages: vec![1], to: 0 },
        PageEdit::Duplicate { pages: vec![0] },
        PageEdit::InsertBlank { at: 0, width: 595.0, height: 842.0 },
    ]
}

/// A file holding `bodies` as objects 1, 2, … with a correct classic
/// cross-reference table. The root is the first body that mentions
/// `/Catalog` (object 1 if none does).
pub fn pdf_from_objects(bodies: &[&[u8]]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(bodies.len());
    for (index, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let root = bodies.iter().position(|body| contains(body, b"/Catalog")).map_or(1, |index| index + 1);
    let xref = out.len();
    let size = bodies.len() + 1;
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root {root} 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

/// Splits fuzz input into object bodies at each `endobj` (the seed format
/// fuzz/seed.mjs writes for the `objects` target).
pub fn split_bodies(data: &[u8]) -> Vec<&[u8]> {
    const SEPARATOR: &[u8] = b"endobj";
    /// Bounds the file the target builds, not the parser.
    const MAX_OBJECTS: usize = 4096;
    let mut bodies = Vec::new();
    let mut rest = data;
    while bodies.len() < MAX_OBJECTS {
        match find(rest, SEPARATOR) {
            Some(at) => {
                bodies.push(&rest[..at]);
                rest = &rest[at + SEPARATOR.len()..];
            }
            None => {
                bodies.push(rest);
                break;
            }
        }
    }
    bodies
}

/// A stream object body: `<< /Length n extra >> stream … endstream`.
pub fn stream(extra: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} {extra} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// One A4 page drawn by `content`, with the resources a real page uses: a
/// simple font (F1), a composite Identity-H font (F2) whose ToUnicode CMap is
/// `to_unicode`, a form XObject, an image, a graphics state and an optional
/// content group that is off (sanitize removes what it hides).
pub fn page_pdf(content: &[u8], to_unicode: &[u8]) -> Vec<u8> {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 200 100] /Resources << /Font << /F1 5 0 R >> >>",
        b"BT /F1 10 Tf 5 5 Td (inside a form) Tj ET",
    );
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[0x00, 0xff, 0xff, 0x00],
    );
    let bodies: [&[u8]; 12] = [
        b"<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [11 0 R] /D << /OFF [11 0 R] >> >> >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << \
          /Font << /F1 5 0 R /F2 6 0 R >> /XObject << /X1 9 0 R /Im1 10 0 R >> \
          /ExtGState << /G1 << /CA 0.5 /ca 0.5 >> >> /Properties << /OC1 11 0 R >> >> >>",
        &stream("", content),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        b"<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Amiri-Regular /Encoding /Identity-H \
          /DescendantFonts [7 0 R] /ToUnicode 8 0 R >>",
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Amiri-Regular \
          /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
          /FontDescriptor 12 0 R /DW 500 /W [1 [600 700 800] 16 20 450] >>",
        &stream("", to_unicode),
        &form,
        &image,
        b"<< /Type /OCG /Name (Hidden) >>",
        b"<< /Type /FontDescriptor /FontName /ABCDEF+Amiri-Regular /Flags 4 \
          /FontBBox [0 -300 1000 900] /ItalicAngle 0 /Ascent 900 /Descent -300 /CapHeight 700 /StemV 80 >>",
    ];
    pdf_from_objects(&bodies)
}

/// The page content the `cmap` target draws its fuzzed CMap with.
pub const CONTENT: &[u8] = b"q 1 0 0 1 0 0 cm /G1 gs
BT /F2 14 Tf 1 0 0 1 300 700 Tm <000100020003> Tj [<0010> -250 <00110012>] TJ ET
BT /F1 12 Tf 72 650 Td 14 TL (Hello, world) Tj T* (second line) ' 2 1 (third) \" ET
/OC /OC1 BDC BT /F1 12 Tf 72 600 Td (hidden) Tj ET EMC
/X1 Do 100 0 0 100 72 400 cm /Im1 Do
Q";

/// The ToUnicode CMap the `content` target's composite font uses.
pub const TO_UNICODE: &[u8] = b"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
3 beginbfchar
<0001> <0627>
<0002> <0644>
<0003> <FEFB>
endbfchar
2 beginbfrange
<0010> <0014> <0661>
<0020> <0021> [<0645> <0020>]
endbfrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end";

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The templates must be files safha-core reads in full; otherwise the
    /// fuzzer would spend its time on the first error.
    #[test]
    fn the_page_template_reads_as_text() {
        let pdf = page_pdf(CONTENT, TO_UNICODE);
        let blocks = safha_core::block_edit::text_blocks(&pdf, &ViewTransform::unrotated(842.0), 0)
            .expect("the template's page reads");
        let text: String = blocks.iter().map(|block| block.text.as_str()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("Hello, world"), "{text}");
        assert!(text.contains('\u{0627}'), "the composite font's text reads through the CMap: {text}");
        assert!(
            safha_core::edit::page_edit_update(&pdf, &PageEdit::Rotate { pages: vec![0], quarter_turns: 1 })
                .is_ok()
        );
        let (_, report) =
            safha_core::redact::sanitize_update(&pdf, SanitizeOptions::default()).expect("sanitizes");
        assert!(report.hidden > 0, "the hidden layer is found: {report:?}");
    }

    #[test]
    fn objects_split_and_rejoin_into_a_readable_file() {
        let data = b"<< /Type /Catalog /Pages 2 0 R /ViewerPreferences << /Direction /R2L >> >>endobj\
                     << /Type /Pages /Kids [3 0 R] /Count 1 >>endobj\
                     << /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>";
        let pdf = pdf_from_objects(&split_bodies(data));
        let hints = safha_core::catalog::catalog_hints(&pdf);
        assert_eq!(hints.direction, Some(safha_core::catalog::ReadingDirection::RightToLeft));
        assert!(safha_core::edit::page_edit_update(&pdf, &PageEdit::Delete { pages: vec![0] }).is_err());
        assert!(safha_core::edit::page_edit_update(&pdf, &PageEdit::Duplicate { pages: vec![0] }).is_ok());
    }
}
