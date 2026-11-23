//! A small AcroForm document written by hand, so the form tests know every
//! field's place and flags: text (one line, multi-line, comb, read-only),
//! a check box with its own appearances, radio buttons without any, a
//! drop-down with Arabic labels, and a multi-select list.

/// Where each field's widget is (left, bottom, right, top), on a 612 × 792 page.
pub const NAME: [f32; 4] = [100.0, 700.0, 400.0, 724.0];
pub const ADDRESS: [f32; 4] = [100.0, 600.0, 400.0, 680.0];
pub const AGREE: [f32; 4] = [100.0, 560.0, 116.0, 576.0];
pub const RADIO_A: [f32; 4] = [100.0, 520.0, 116.0, 536.0];
pub const RADIO_B: [f32; 4] = [150.0, 520.0, 166.0, 536.0];
pub const CITY: [f32; 4] = [100.0, 470.0, 300.0, 494.0];
pub const LANGS: [f32; 4] = [100.0, 380.0, 300.0, 440.0];
pub const ID: [f32; 4] = [100.0, 330.0, 300.0, 354.0];
pub const LOCKED: [f32; 4] = [100.0, 280.0, 400.0, 304.0];

/// UTF-16BE hex string with a byte-order mark.
fn utf16(text: &str) -> String {
    let hex: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
    format!("<FEFF{hex}>")
}

fn rect([l, b, r, t]: [f32; 4]) -> String {
    format!("[{l} {b} {r} {t}]")
}

/// The document; `need_appearances` sets `/NeedAppearances true`.
pub fn form_pdf(need_appearances: bool) -> Vec<u8> {
    let page = 3;
    let widget = |r: [f32; 4], rest: &str| {
        format!(
            "<< /Type /Annot /Subtype /Widget /Rect {} /P {page} 0 R /F 4 /MK << /BC [0] /BG [1] >> {rest} >>",
            rect(r)
        )
    };
    // Object 1: catalog; 2: pages; 3: page; 4: content; 5: font; 6..: form.
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /Helv 5 0 R >> >> \
         /Annots [8 0 R 9 0 R 10 0 R 12 0 R 13 0 R 14 0 R 15 0 R 16 0 R 17 0 R] >>"
            .into(),
        String::new(), // content, below
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!(
            "<< /Fields [7 0 R 9 0 R 10 0 R 11 0 R 14 0 R 15 0 R 16 0 R 17 0 R] /DA (/Helv 0 Tf 0 g) \
             /DR << /Font << /Helv 5 0 R >> >>{} >>",
            if need_appearances { " /NeedAppearances true" } else { "" }
        ),
        // 7: a parent naming its one child "applicant.name".
        "<< /T (applicant) /Kids [8 0 R] >>".into(),
        // 8: the name field, merged with its widget; size 0 (fit).
        widget(NAME, "/Parent 7 0 R /FT /Tx /T (name)"),
        // 9: multi-line address, 10 pt.
        widget(ADDRESS, "/FT /Tx /T (address) /Ff 4096 /DA (/Helv 10 Tf 0 0 0.5 rg)"),
        // 10: a check box with its own appearances (18, 19).
        widget(
            AGREE,
            "/FT /Btn /T (agree) /V /Off /AS /Off /AP << /N << /Yes 18 0 R /Off 19 0 R >> >> /MK << /CA (4) >>",
        ),
        // 11: radio buttons (flags: radio, no toggle to off), no appearances of their own.
        "<< /FT /Btn /T (choice) /Ff 49152 /Kids [12 0 R 13 0 R] /DA (/ZaDb 0 Tf 0 g) >>".into(),
        widget(RADIO_A, "/Parent 11 0 R"),
        widget(RADIO_B, "/Parent 11 0 R"),
        // 14: a drop-down with Arabic labels.
        widget(
            CITY,
            &format!(
                "/FT /Ch /T (city) /Ff 131072 /DA (/Helv 11 Tf 0 g) /Opt [[(ruh) {}] [(jed) {}] [(dmm) (Dammam)]]",
                utf16("الرياض"),
                utf16("جدة")
            ),
        ),
        // 15: a list taking several choices.
        widget(
            LANGS,
            "/FT /Ch /T (langs) /Ff 2097152 /DA (/Helv 10 Tf 0 g) /Opt [(Arabic) (English) (French)]",
        ),
        // 16: a comb field of ten cells.
        widget(ID, "/FT /Tx /T (id) /Ff 16777216 /MaxLen 10 /DA (/Helv 12 Tf 0 g)"),
        // 17: read-only, with a value but no appearance.
        widget(LOCKED, &format!("/FT /Tx /T (locked) /Ff 1 /V {}", utf16("سري للغاية"))),
    ];
    let checked = "q 0 g 2 2 12 12 re f Q";
    let unchecked = "q 0 G 1 w 0.5 0.5 15 15 re S Q";
    objects.push(format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 16 16] /Length {} >>\nstream\n{checked}\nendstream",
        checked.len()
    ));
    objects.push(format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 16 16] /Length {} >>\nstream\n{unchecked}\nendstream",
        unchecked.len()
    ));
    let content = "BT /Helv 10 Tf 20 760 Td (Application form) Tj ET";
    objects[3] = format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len());

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
