//! A one-page PDF that packs in the content-stream features the corpus lacks, so
//! regeneration is tested against them too. Written byte by byte (no PDF library)
//! so nothing normalizes it on the way out.

/// Every feature, in paint order. The comment after `%` names it in the report.
const CONTENT: &str = r#"% z-order: blue overlaps red
0.9 0.2 0.2 rg 20 200 120 80 re f
0.2 0.2 0.9 rg 80 180 120 80 re f
% clip path around overflowing text
q 20 120 100 40 re W n BT /F1 24 Tf 0 g 22 130 Td (CLIPPED TEXT HERE) Tj ET Q
% text render mode 7 (text as clip), then a shading through it
q BT 7 Tr /F1 40 Tf 210 215 Td (SHADE) Tj ET /Sh0 sh Q
% tagged structure content
/P <</MCID 0>> BDC BT /F1 12 Tf 0 g 20 100 Td (Tagged paragraph) Tj ET EMC
% optional content by property-list name
/OC /oc1 BDC 0 0.6 0 rg 220 120 60 40 re f EMC
% ActualText span
/Span <</ActualText (REPLACED)>> BDC BT /F1 12 Tf 0 g 20 84 Td (actual) Tj ET EMC
% artifact
/Artifact BMC 0.5 g 20 75 120 1 re f EMC
% inline image
q 40 0 0 40 330 20 cm BI /W 2 /H 2 /CS /RGB /BPC 8 ID
"#;

const CONTENT_AFTER_IMAGE: &str = r#"EI Q
% form XObject
q 1 0 0 1 300 100 cm /Fm0 Do Q
% ExtGState constant alpha
q /GS0 gs 1 0.5 0 rg 150 20 100 60 re f Q
% text state: Tc Tw Tz TL T* '
BT /F1 10 Tf 2 Tc 6 Tw 150 Tz 12 TL 0 g 20 60 Td (Spaced words here) Tj T* (Next line) ' ET
% dash, cap, join, width
q [6 3] 0 d 2 J 1 j 3 w 0 0 0 RG 20 30 m 140 30 l S Q
% compatibility section with an unknown operator
BX /Foo 1 xyzzy EX
"#;

const FORM: &str = "0 0.5 0.5 rg 0 0 80 60 re f BT /F1 10 Tf 1 g 5 25 Td (Form text) Tj ET";

pub fn build() -> Vec<u8> {
    // 2x2 RGB: red, green / blue, white.
    let image: [u8; 12] = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    // The `%` lines are notes for readers of this file; the stream gets the operators only.
    let ops = |text: &str| -> Vec<u8> {
        text.lines().filter(|l| !l.starts_with('%')).collect::<Vec<_>>().join("\n").into_bytes()
    };
    let mut content = ops(CONTENT);
    content.extend_from_slice(b"\n");
    content.extend_from_slice(&image);
    content.extend_from_slice(b"\n");
    content.extend_from_slice(&ops(CONTENT_AFTER_IMAGE));

    let objects: Vec<Vec<u8>> = vec![
        // 1: catalog
        b"<< /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> /StructTreeRoot 9 0 R \
          /OCProperties << /OCGs [8 0 R] /D << /ON [8 0 R] /Order [8 0 R] >> >> >>"
            .to_vec(),
        // 2: pages
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        // 3: page
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Contents 4 0 R /StructParents 0 \
          /Resources << /Font << /F1 5 0 R >> /Shading << /Sh0 6 0 R >> /XObject << /Fm0 7 0 R >> \
          /ExtGState << /GS0 << /ca 0.5 /CA 0.5 >> >> /Properties << /oc1 8 0 R >> >> >>"
            .to_vec(),
        // 4: content
        stream("", &content),
        // 5: font
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        // 6: axial shading
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [210 0 390 0] /Extend [true true] \
          /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>"
            .to_vec(),
        // 7: form
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 80 60] /Resources << /Font << /F1 5 0 R >> >>",
            FORM.as_bytes(),
        ),
        // 8: optional content group
        b"<< /Type /OCG /Name (Layer) >>".to_vec(),
        // 9: structure tree root
        b"<< /Type /StructTreeRoot /K [10 0 R] /ParentTree << /Nums [0 [10 0 R]] >> >>".to_vec(),
        // 10: the paragraph element owning MCID 0
        b"<< /Type /StructElem /S /P /P 9 0 R /Pg 3 0 R /K 0 >>".to_vec(),
    ];

    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}
