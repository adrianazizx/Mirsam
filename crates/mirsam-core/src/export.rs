//! Exporting pages (M6, ADR 0050): as images, as plain text, as Markdown.
//!
//! - **Images.** A page is rendered by PDFium at the chosen resolution and
//!   encoded as PNG (the `png` crate) or JPEG (our own baseline encoder,
//!   `jpeg`). Pages render opaque (white paper), so JPEG needs no alpha.
//! - **Text.** The text of each page in *logical* reading order (spec §6.1),
//!   which is the order Arabic is stored and copied in: the file holds the
//!   letters in the order they are read, and any editor that applies the
//!   Unicode bidirectional algorithm shows them correctly. No directional
//!   control characters are added, because they would travel with a copied
//!   passage.
//! - **Markdown.** The same text as paragraphs, one per block of the page.
//!   A block that reads right to left on a document that does not (or the
//!   other way round) is wrapped in `<div dir="rtl">`, which every
//!   Markdown renderer that takes inline HTML honours. Text is escaped so
//!   it cannot become markup. Headings are not detected: the text engine
//!   does not report font sizes.

use serde::{Deserialize, Serialize};

use crate::Bitmap;
use crate::document::{Document, MAX_BITMAP_SIDE};
use crate::error::{Error, Result};
use crate::text::{Direction, PageText};

/// Image formats a page can be exported as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImageFormat {
    Png,
    /// Quality 1 to 100.
    Jpeg {
        quality: u8,
    },
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg { .. } => "jpg",
        }
    }
}

/// How to write a document's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextFormat {
    Plain,
    Markdown,
}

impl TextFormat {
    pub fn extension(self) -> &'static str {
        match self {
            TextFormat::Plain => "txt",
            TextFormat::Markdown => "md",
        }
    }
}

/// The resolutions an export accepts, in dots per inch.
pub const DPI_RANGE: std::ops::RangeInclusive<u32> = 36..=600;

/// Encodes a rendered page.
pub fn encode_bitmap(bitmap: &Bitmap, format: ImageFormat) -> Result<Vec<u8>> {
    let (width, height) = (bitmap.width(), bitmap.height());
    let too_big = || Error::InvalidEdit("the image is too large to export");
    match format {
        ImageFormat::Png => {
            let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
            for pixel in bitmap.as_bgra().as_chunks::<4>().0 {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            }
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            let mut writer = encoder.write_header().map_err(|_| too_big())?;
            writer.write_image_data(&rgb).map_err(|_| too_big())?;
            writer.finish().map_err(|_| too_big())?;
            Ok(out)
        }
        ImageFormat::Jpeg { quality } => {
            if !(1..=100).contains(&quality) {
                return Err(Error::InvalidEdit("JPEG quality runs from 1 to 100"));
            }
            let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
            for pixel in bitmap.as_bgra().as_chunks::<4>().0 {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            }
            crate::jpeg::encode_rgb(&rgb, width, height, quality).ok_or_else(too_big)
        }
    }
}

impl Document {
    /// Page `index` as an image at `dpi` dots per inch.
    pub fn export_page_image(&self, index: usize, dpi: u32, format: ImageFormat) -> Result<Vec<u8>> {
        if !DPI_RANGE.contains(&dpi) {
            return Err(Error::InvalidEdit("images are exported at 36 to 600 dpi"));
        }
        let size = self.page_size(index)?;
        let scale = dpi as f32 / 72.0;
        if (size.width * scale).ceil() > MAX_BITMAP_SIDE as f32
            || (size.height * scale).ceil() > MAX_BITMAP_SIDE as f32
        {
            return Err(Error::InvalidEdit("the page is too large to export at this resolution"));
        }
        encode_bitmap(&self.render_page(index, scale)?, format)
    }

    /// The text of `pages` (zero-based, in the order given) as one string.
    pub fn export_text(&self, pages: &[usize], format: TextFormat) -> Result<String> {
        if pages.is_empty() {
            return Err(Error::InvalidEdit("no pages given"));
        }
        let mut parts = Vec::with_capacity(pages.len());
        for &page in pages {
            parts.push((page, self.page_text(page)?));
        }
        Ok(match format {
            TextFormat::Plain => plain_text(&parts),
            TextFormat::Markdown => markdown(&parts),
        })
    }
}

/// Pages separated by a form feed on a line of its own, the plain-text page
/// break; blocks by a blank line.
fn plain_text(pages: &[(usize, PageText)]) -> String {
    let mut out = String::new();
    for (i, (_, text)) in pages.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\u{000C}\n");
        }
        out.push_str(text.text().trim_end());
        out.push('\n');
    }
    out
}

fn markdown(pages: &[(usize, PageText)]) -> String {
    // The document's direction is the first page that has text.
    let document = pages
        .iter()
        .find(|(_, text)| !text.blocks.is_empty())
        .map_or(Direction::Ltr, |(_, text)| text.direction);
    let mut out = String::new();
    for (i, (page, text)) in pages.iter().enumerate() {
        if i > 0 {
            out.push_str("\n---\n\n");
        }
        out.push_str(&format!("<!-- page {} -->\n\n", page + 1));
        for block in &text.blocks {
            let paragraph = block.lines.iter().map(|line| line.text.trim()).collect::<Vec<_>>().join(" ");
            if paragraph.is_empty() {
                continue;
            }
            if block.direction == document {
                out.push_str(&escape_markdown(&paragraph));
            } else {
                let dir = if block.direction == Direction::Rtl { "rtl" } else { "ltr" };
                out.push_str(&format!("<div dir=\"{dir}\">\n\n{}\n\n</div>", escape_markdown(&paragraph)));
            }
            out.push_str("\n\n");
        }
    }
    out
}

/// Text that cannot become markup: Markdown's own characters and HTML's are
/// escaped where they could matter.
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (i, c) in text.chars().enumerate() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '|' | '~' => {
                out.push('\\');
                out.push(c);
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            // A line that starts like a heading, a list, or a quote.
            '#' | '+' | '-' | '=' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    // "1. text" would start a numbered list.
    let digits = out.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && out[digits..].starts_with(['.', ')']) {
        out.insert(digits, '\\');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_escapes_what_would_become_markup() {
        assert_eq!(escape_markdown("a *b* <i>&"), "a \\*b\\* &lt;i&gt;&amp;");
        assert_eq!(escape_markdown("# not a heading"), "\\# not a heading");
        assert_eq!(escape_markdown("1. not a list"), "1\\. not a list");
        assert_eq!(escape_markdown("نص عربي"), "نص عربي");
    }

    #[test]
    fn quality_is_checked() {
        let bitmap = Bitmap::from_bgra(1, 1, vec![0, 0, 0, 255]).expect("bitmap");
        assert!(matches!(
            encode_bitmap(&bitmap, ImageFormat::Jpeg { quality: 0 }),
            Err(Error::InvalidEdit(_))
        ));
        assert!(encode_bitmap(&bitmap, ImageFormat::Png).is_ok());
    }
}
