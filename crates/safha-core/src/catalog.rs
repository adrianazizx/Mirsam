//! A minimal, read-only reader for a PDF's document catalog: just enough to
//! learn the document's own reading preferences, which PDFium exposes only
//! through an `unsafe` call (`FPDF_VIEWERREF_GetName`).
//!
//! - `/ViewerPreferences /Direction /R2L`: pages are bound on the right, so a
//!   two-page spread puts the earlier page on the right (spec section 6.5;
//!   most Arabic books).
//! - `/PageLayout`: the layout the author asked for (single, continuous,
//!   two-page with the cover alone or not).
//!
//! It follows the cross-reference chain from the end of the file (classic
//! tables, cross-reference streams, hybrid files, incremental updates) to the
//! catalog, decompressing object streams with Flate when the catalog lives in
//! one. Input is untrusted: every loop, nesting level, and decompressed size is
//! bounded, and anything unexpected (damage, other filters, encrypted object
//! streams) yields "no preference" rather than an error. PDFium still renders
//! such files; they just open left to right.

use crate::pdf::File;

/// The document's reading preferences, where it states them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatalogHints {
    pub direction: Option<ReadingDirection>,
    pub page_layout: Option<PageLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingDirection {
    LeftToRight,
    RightToLeft,
}

/// `/PageLayout` values (ISO 32000-1, table 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageLayout {
    SinglePage,
    OneColumn,
    /// Two columns, odd pages on the left.
    TwoColumnLeft,
    /// Two columns, odd pages on the right (the first page alone).
    TwoColumnRight,
    TwoPageLeft,
    TwoPageRight,
}

/// Reads the catalog's hints; empty when the file does not state them or
/// cannot be read this way.
pub fn catalog_hints(bytes: &[u8]) -> CatalogHints {
    read_hints(bytes).unwrap_or_default()
}

fn read_hints(bytes: &[u8]) -> Option<CatalogHints> {
    let file = File::new(bytes)?;
    let catalog = file.resolve(file.trailer.get(b"Root")?)?;
    let catalog = catalog.as_dict()?;
    let direction = catalog
        .get(b"ViewerPreferences")
        .and_then(|prefs| file.resolve(prefs))
        .and_then(|prefs| prefs.as_dict().and_then(|d| d.get(b"Direction")).cloned())
        .and_then(|direction| match file.resolve(&direction)?.as_name()? {
            b"R2L" => Some(ReadingDirection::RightToLeft),
            b"L2R" => Some(ReadingDirection::LeftToRight),
            _ => None,
        });
    let page_layout = catalog.get(b"PageLayout").and_then(|layout| {
        Some(match file.resolve(layout)?.as_name()? {
            b"SinglePage" => PageLayout::SinglePage,
            b"OneColumn" => PageLayout::OneColumn,
            b"TwoColumnLeft" => PageLayout::TwoColumnLeft,
            b"TwoColumnRight" => PageLayout::TwoColumnRight,
            b"TwoPageLeft" => PageLayout::TwoPageLeft,
            b"TwoPageRight" => PageLayout::TwoPageRight,
            _ => return None,
        })
    });
    Some(CatalogHints { direction, page_layout })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::pdf::startxref;

    /// A one-page PDF with a classic cross-reference table. `catalog` is the
    /// catalog dictionary's body; `extra` are further objects from number 4.
    fn classic(catalog: &str, extra: &[&str]) -> Vec<u8> {
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3 2] >>".to_string(),
        ];
        objects.extend(extra.iter().map(|s| s.to_string()));
        let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
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

    /// Appends an incremental update replacing the catalog.
    fn update_catalog(mut pdf: Vec<u8>, catalog: &str) -> Vec<u8> {
        let previous = startxref(&pdf).expect("startxref");
        let offset = pdf.len();
        pdf.extend_from_slice(
            format!("1 0 obj\n<< /Type /Catalog /Pages 2 0 R {catalog} >>\nendobj\n").as_bytes(),
        );
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n1 1\n{offset:010} 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        pdf
    }

    /// The same document with the catalog in a Flate-compressed object stream,
    /// indexed by a cross-reference stream with a PNG Up predictor.
    fn with_object_stream(catalog: &str, encrypted: bool) -> Vec<u8> {
        let catalog = format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>");
        let header = "1 0 ";
        let objstm_data = format!("{header}{catalog}");
        let objstm = miniz_oxide::deflate::compress_to_vec_zlib(objstm_data.as_bytes(), 6);
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = HashMap::new();
        offsets.insert(2u32, pdf.len());
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        offsets.insert(3, pdf.len());
        pdf.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3 2] >>\nendobj\n");
        offsets.insert(4, pdf.len());
        pdf.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /ObjStm /N 1 /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
                header.len(),
                objstm.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&objstm);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        let xref_offset = pdf.len();
        // Rows: type (1 byte), field 2 (4 bytes), field 3 (1 byte), each
        // with a PNG "Up" filter byte, for objects 0..=5.
        let rows: Vec<[u8; 6]> = vec![
            [0, 0, 0, 0, 0, 0],
            [2, 0, 0, 0, 4, 0],
            row(1, offsets[&2]),
            row(1, offsets[&3]),
            row(1, offsets[&4]),
            row(1, xref_offset),
        ];
        let mut raw = Vec::new();
        let mut previous = [0u8; 6];
        for current in &rows {
            raw.push(2); // Up
            for i in 0..6 {
                raw.push(current[i].wrapping_sub(previous[i]));
            }
            previous = *current;
        }
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let encrypt = if encrypted { "/Encrypt << /Filter /Standard >>" } else { "" };
        pdf.extend_from_slice(
            format!(
                "5 0 obj\n<< /Type /XRef /Size 6 /W [1 4 1] /Root 1 0 R {encrypt} /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 6 >> /Length {} >>\nstream\n",
                data.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&data);
        pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes());
        pdf
    }

    fn row(kind: u8, offset: usize) -> [u8; 6] {
        let o = u32::try_from(offset).expect("small").to_be_bytes();
        [kind, o[0], o[1], o[2], o[3], 0]
    }

    const RTL: CatalogHints = CatalogHints {
        direction: Some(ReadingDirection::RightToLeft),
        page_layout: Some(PageLayout::TwoPageRight),
    };

    #[test]
    fn reads_direction_and_layout_from_a_classic_file() {
        let pdf = classic("/ViewerPreferences << /Direction /R2L >> /PageLayout /TwoPageRight", &[]);
        assert_eq!(catalog_hints(&pdf), RTL);
        let pdf = classic("/ViewerPreferences 4 0 R", &["<< /Direction /L2R >>"]);
        assert_eq!(catalog_hints(&pdf).direction, Some(ReadingDirection::LeftToRight));
        assert_eq!(catalog_hints(&classic("", &[])), CatalogHints::default());
    }

    #[test]
    fn the_newest_incremental_update_wins() {
        let pdf = classic("/ViewerPreferences << /Direction /L2R >>", &[]);
        let pdf = update_catalog(pdf, "/ViewerPreferences << /Direction /R2L >> /PageLayout /TwoPageRight");
        assert_eq!(catalog_hints(&pdf), RTL);
    }

    #[test]
    fn follows_cross_reference_and_object_streams() {
        let pdf =
            with_object_stream("/ViewerPreferences << /Direction /R2L >> /PageLayout /TwoPageRight", false);
        assert_eq!(catalog_hints(&pdf), RTL);
        // Encrypted object streams cannot be read without the key: no hints.
        let pdf = with_object_stream("/ViewerPreferences << /Direction /R2L >>", true);
        assert_eq!(catalog_hints(&pdf), CatalogHints::default());
    }

    #[test]
    fn damaged_or_hostile_input_gives_no_hints() {
        assert_eq!(catalog_hints(b""), CatalogHints::default());
        assert_eq!(catalog_hints(b"%PDF-1.7\nstartxref\n999999\n%%EOF"), CatalogHints::default());
        let pdf = classic("/ViewerPreferences << /Direction /R2L >>", &[]);
        assert_eq!(catalog_hints(&pdf[..pdf.len() / 2]), CatalogHints::default());
        // A /Prev that points at itself, and deep nesting, terminate.
        let mut looped = classic("", &[]);
        let at = startxref(&looped).expect("startxref");
        let tail = format!("trailer\n<< /Size 4 /Root 1 0 R /Prev {at} >>\nstartxref\n{at}\n%%EOF\n");
        looped.extend_from_slice(tail.as_bytes());
        let _ = catalog_hints(&looped);
        let deep = format!("/X {}{}", "[".repeat(10_000), "]".repeat(10_000));
        let _ = catalog_hints(&classic(&deep, &[]));
    }
}
