//! The handful of font facts a PDF font dictionary needs, read straight from the
//! sfnt tables, plus a glyph -> Unicode reverse cmap.

use std::collections::HashMap;
use std::path::Path;

pub struct FontInfo {
    /// e.g. "Amiri-Regular" (file stem; good enough for a spike's /BaseFont).
    pub name: String,
    pub data: Vec<u8>,
    pub upem: u16,
    pub bbox: [i16; 4],
    pub ascent: i16,
    pub descent: i16,
    pub cap_height: i16,
    pub italic_angle: f32,
    pub num_glyphs: u16,
    advances: Vec<u16>,
    /// Glyph -> the code point(s) the font's own cmap maps to it. A glyph missing
    /// here is reachable only through GSUB (contextual forms, ligatures).
    pub rev_cmap: HashMap<u16, Vec<u32>>,
    pub shaper: harfrust::Font,
}

fn be16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}
fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn table<'a>(d: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let n = be16(d, 4) as usize;
    (0..n).map(|i| 12 + 16 * i).find(|&r| &d[r..r + 4] == tag).map(|r| {
        let (off, len) = (be32(d, r + 8) as usize, be32(d, r + 12) as usize);
        &d[off..off + len]
    })
}

impl FontInfo {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let data = std::fs::read(path)?;
        let head = table(&data, b"head").ok_or("no head")?;
        let hhea = table(&data, b"hhea").ok_or("no hhea")?;
        let maxp = table(&data, b"maxp").ok_or("no maxp")?;
        let hmtx = table(&data, b"hmtx").ok_or("no hmtx")?;
        let os2 = table(&data, b"OS/2");
        let post = table(&data, b"post");

        let num_glyphs = be16(maxp, 4);
        let n_hm = be16(hhea, 34) as usize;
        let mut advances = Vec::with_capacity(num_glyphs as usize);
        for g in 0..num_glyphs as usize {
            advances.push(be16(hmtx, 4 * g.min(n_hm - 1)));
        }
        let ascent = be16(hhea, 4) as i16;
        let cap_height =
            os2.filter(|t| be16(t, 0) >= 2 && t.len() >= 90).map(|t| be16(t, 88) as i16).unwrap_or(ascent);

        let shaper = harfrust::Font::new(data.clone(), 0).ok_or("harfrust cannot parse the font")?;
        let mut rev_cmap: HashMap<u16, Vec<u32>> = HashMap::new();
        for (cp, gid) in shaper.charmap().unicodes() {
            rev_cmap.entry(gid.to_u32() as u16).or_default().push(cp);
        }
        for v in rev_cmap.values_mut() {
            v.sort_unstable();
        }

        Ok(Self {
            name: path.file_stem().and_then(|s| s.to_str()).unwrap_or("Font").to_string(),
            upem: be16(head, 18),
            bbox: [
                be16(head, 36) as i16,
                be16(head, 38) as i16,
                be16(head, 40) as i16,
                be16(head, 42) as i16,
            ],
            ascent,
            descent: be16(hhea, 6) as i16,
            cap_height,
            italic_angle: post.map(|t| be32(t, 4) as i32 as f32 / 65536.0).unwrap_or(0.0),
            num_glyphs,
            advances,
            rev_cmap,
            shaper,
            data,
        })
    }

    /// hmtx advance in font units: what a PDF viewer advances by after showing the glyph.
    pub fn advance(&self, gid: u16) -> u16 {
        self.advances.get(gid as usize).copied().unwrap_or(0)
    }

    /// Font units -> PDF glyph space (1/1000 em).
    pub fn k(&self, units: f64) -> f64 {
        units * 1000.0 / self.upem as f64
    }

    pub fn glyph_name(&self, gid: u16) -> String {
        harfrust::ShaperFont::new(&self.shaper)
            .glyph_name(harfrust::GlyphId::new(gid as u32))
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("gid{gid}"))
    }
}
