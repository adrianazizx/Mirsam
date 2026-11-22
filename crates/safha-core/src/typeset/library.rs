//! Installed fonts (spec §6.3: "use system fonts too"), and any other folder
//! of fonts such as the app's bundled ones: found once, indexed by what font
//! matching needs, and read in full only when one is chosen.
//!
//! Indexing reads just the small tables matching looks at — `name`, `OS/2`,
//! `post`, `cmap`, `head`, `hhea`, `maxp`, and the variation tables `fvar`,
//! `avar` and `STAT` — with seeks, never the outlines. A file that cannot be
//! read or parsed is counted in [`FontLibrary::skipped`] and otherwise ignored.
//! Fonts whose family starts with a dot are left out: macOS's convention for
//! fonts hidden from documents.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use icu_properties::CodePointMapData;
use icu_properties::props::Script;

use super::matching::{FontMatch, FontRequest, FontTraits, MatchKind, Wanted, match_font, needed_chars};
use super::{FontFace, TypesetError};

/// How many of the best-named fonts [`FontLibrary::find`] reads in full to
/// check every character and the widths.
const LOAD_LIMIT: usize = 6;
/// Folder depth searched under each root.
const MAX_DEPTH: usize = 8;
/// Faces read from one collection file.
const MAX_FACES: u32 = 256;
/// Largest table indexing will read (a CJK `cmap` is a few hundred KB).
const MAX_TABLE: u32 = 32 << 20;
const INDEX_TABLES: [&[u8; 4]; 10] =
    [b"head", b"hhea", b"maxp", b"name", b"OS/2", b"post", b"cmap", b"fvar", b"avar", b"STAT"];

/// Writing systems a font can set: every basic letter of the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Coverage(u16);

impl Coverage {
    pub const LATIN: Coverage = Coverage(1);
    pub const ARABIC: Coverage = Coverage(1 << 1);
    /// The letters Persian and Urdu add to Arabic (پ چ ژ گ ک ی ٹ ڈ ڑ ں ہ ے).
    pub const ARABIC_EXTENDED: Coverage = Coverage(1 << 2);
    pub const HEBREW: Coverage = Coverage(1 << 3);
    pub const GREEK: Coverage = Coverage(1 << 4);
    pub const CYRILLIC: Coverage = Coverage(1 << 5);
    pub const CJK: Coverage = Coverage(1 << 6);

    pub fn contains(self, other: Coverage) -> bool {
        self.0 & other.0 == other.0
    }

    fn with(self, other: Coverage) -> Coverage {
        Coverage(self.0 | other.0)
    }
}

const EXTENDED_ARABIC: &str = "پچژگکیٹڈڑںہے";

/// Characters a font must have for each [`Coverage`] flag.
fn probes() -> [(Coverage, Vec<char>); 7] {
    let range = |a: char, b: char| (a..=b).collect::<Vec<char>>();
    [
        (Coverage::LATIN, [range('A', 'Z'), range('a', 'z')].concat()),
        // Hamza to ghain and feh to yeh: the 28 letters and their hamza forms.
        (Coverage::ARABIC, [range('\u{0621}', '\u{063A}'), range('\u{0641}', '\u{064A}')].concat()),
        (Coverage::ARABIC_EXTENDED, EXTENDED_ARABIC.chars().collect()),
        (Coverage::HEBREW, range('\u{05D0}', '\u{05EA}')),
        (Coverage::GREEK, [range('\u{0391}', '\u{03A1}'), range('\u{03B1}', '\u{03C9}')].concat()),
        (Coverage::CYRILLIC, range('\u{0410}', '\u{044F}')),
        (Coverage::CJK, vec!['一', '中', '日', '本', '語', 'の', 'ア']),
    ]
}

/// The writing systems `text` needs.
fn needed_coverage(text: &str) -> Coverage {
    let scripts = CodePointMapData::<Script>::new();
    text.chars().fold(Coverage::default(), |acc, c| {
        let flag = match scripts.get(c) {
            Script::Latin => Coverage::LATIN,
            Script::Arabic if EXTENDED_ARABIC.contains(c) => Coverage::ARABIC_EXTENDED,
            Script::Arabic => Coverage::ARABIC,
            Script::Hebrew => Coverage::HEBREW,
            Script::Greek => Coverage::GREEK,
            Script::Cyrillic => Coverage::CYRILLIC,
            Script::Han | Script::Hiragana | Script::Katakana => Coverage::CJK,
            _ => Coverage::default(),
        };
        acc.with(flag)
    })
}

/// One face in the index.
#[derive(Debug, Clone, PartialEq)]
pub struct FontEntry {
    pub path: PathBuf,
    /// Face index inside the file (0 unless it is a collection).
    pub index: u32,
    /// At the font's default instance.
    pub traits: FontTraits,
    pub coverage: Coverage,
    /// The `wght` axis of a variable font: minimum and maximum.
    pub weight_range: Option<(f32, f32)>,
}

/// Fonts found in a set of folders.
#[derive(Debug, Clone, Default)]
pub struct FontLibrary {
    entries: Vec<FontEntry>,
    skipped: usize,
}

/// Where this platform keeps installed fonts, system-wide and per user.
/// Folders that do not exist are left out.
pub fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        dirs.extend(home.map(|h| h.join("Library/Fonts")));
    } else if cfg!(target_os = "windows") {
        let windir = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot"));
        dirs.extend(windir.map(|w| PathBuf::from(w).join("Fonts")));
        dirs.extend(
            std::env::var_os("LOCALAPPDATA")
                .map(|l| PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts")),
        );
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        dirs.extend(data_home.map(|d| d.join("fonts")));
        dirs.extend(home.map(|h| h.join(".fonts")));
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

impl FontLibrary {
    /// Indexes every font file (`.ttf`, `.otf`, `.ttc`, `.otc`) under `dirs`.
    pub fn scan(dirs: &[PathBuf]) -> FontLibrary {
        let mut library = FontLibrary::default();
        let mut files = Vec::new();
        for dir in dirs {
            collect_font_files(dir, 0, &mut files);
        }
        files.sort();
        files.dedup();
        for path in files {
            match index_file(&path) {
                // A file of hidden faces only indexes nothing and is not an error.
                Ok(faces) => library.entries.extend(faces),
                Err(_) => library.skipped += 1,
            }
        }
        library
    }

    /// The fonts installed on this machine ([`system_font_dirs`]).
    pub fn system() -> FontLibrary {
        FontLibrary::scan(&system_font_dirs())
    }

    pub fn entries(&self) -> &[FontEntry] {
        &self.entries
    }

    /// Font files that could not be read or parsed.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// Reads a face in full.
    pub fn load(&self, entry: &FontEntry) -> Result<FontFace, TypesetError> {
        let data = std::fs::read(&entry.path).map_err(|_| TypesetError::FontParse)?;
        FontFace::from_bytes(data, entry.index)
    }

    /// The installed font closest to `request` that can set `text`.
    ///
    /// Ranks the index by name and style among faces that cover `text`'s
    /// writing systems, reads the best few in full, and picks among them with
    /// [`match_font`] (every character, and the widths when the request has
    /// them). A variable face with a `wght` axis is instanced at the requested
    /// weight. Returns the face, its index entry, and the match.
    pub fn find(&self, request: &FontRequest, text: &str) -> Option<(FontFace, FontEntry, FontMatch)> {
        let wanted = Wanted::new(request);
        let needed = needed_coverage(text);
        let covering: Vec<&FontEntry> = self.entries.iter().filter(|e| e.coverage.contains(needed)).collect();
        let pool = if covering.is_empty() { self.entries.iter().collect() } else { covering };
        let mut ranked: Vec<(f32, &FontEntry)> = pool
            .into_iter()
            .map(|e| {
                let mut traits = e.traits.clone();
                if let Some((min, max)) = e.weight_range {
                    traits.weight = wanted.weight().clamp(min, max);
                }
                (wanted.rank(&traits).0, e)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));

        let mut faces = Vec::new();
        let mut entries = Vec::new();
        for (_, entry) in ranked.into_iter() {
            if faces.len() == LOAD_LIMIT {
                break;
            }
            let Ok(face) = self.load(entry) else { continue };
            let face = match entry.weight_range {
                Some((min, max)) => {
                    face.with_variations(&[(*b"wght", wanted.weight().clamp(min, max))]).unwrap_or(face)
                }
                None => face,
            };
            faces.push(face);
            entries.push(entry);
        }
        let found = match_font(request, text, &faces)?;
        let entry = entries[found.index];
        let face = faces.swap_remove(found.index);
        let kind = if needed_chars(text).iter().all(|c| face.covers(*c)) {
            found.kind
        } else {
            MatchKind::Substitute
        };
        // The match's index is the entry's place in this library.
        let index = self.entries.iter().position(|e| std::ptr::eq(e, entry)).unwrap_or(0);
        Some((face, entry.clone(), FontMatch { index, kind, covers_text: found.covers_text }))
    }
}

fn collect_font_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        // Follows links to files, not to folders (no loops).
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            collect_font_files(&path, depth + 1, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["ttf", "otf", "ttc", "otc"].contains(&e.to_ascii_lowercase().as_str()))
        {
            out.push(path);
        }
    }
}

/// Every face of one font file, from its small tables only. Fails only when
/// no face could be read.
fn index_file(path: &Path) -> std::io::Result<Vec<FontEntry>> {
    let mut file = File::open(path)?;
    let header = read_at(&mut file, 0, 12)?;
    let offsets: Vec<u32> = if &header[..4] == b"ttcf" {
        let count = be32(&header, 8).min(MAX_FACES);
        let table = read_at(&mut file, 12, count * 4)?;
        (0..count as usize).map(|i| be32(&table, i * 4)).collect()
    } else {
        vec![0]
    };
    let mut entries = Vec::new();
    let mut failure = None;
    for (index, offset) in offsets.into_iter().enumerate() {
        // One bad face in a collection does not lose the others.
        match index_face(&mut file, path, index as u32, offset) {
            Ok(Some(entry)) => entries.push(entry),
            Ok(None) => {}
            Err(e) => failure = Some(e),
        }
    }
    match failure {
        Some(e) if entries.is_empty() => Err(e),
        _ => Ok(entries),
    }
}

/// `Ok(None)` for a face hidden by convention.
fn index_face(file: &mut File, path: &Path, index: u32, offset: u32) -> std::io::Result<Option<FontEntry>> {
    let head = read_at(file, offset, 12)?;
    let num_tables = u32::from(be16(&head, 4));
    let directory = read_at(file, offset + 12, num_tables * 16)?;
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for i in 0..num_tables as usize {
        let record = &directory[i * 16..i * 16 + 16];
        let tag = [record[0], record[1], record[2], record[3]];
        if !INDEX_TABLES.contains(&&tag) {
            continue;
        }
        let (table_offset, length) = (be32(record, 8), be32(record, 12));
        if length > MAX_TABLE {
            continue;
        }
        tables.push((tag, read_at(file, table_offset, length)?));
    }
    let sfnt_version = [head[0], head[1], head[2], head[3]];
    let face = FontFace::from_bytes(rebuild_sfnt(sfnt_version, &tables), 0)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let traits = face.traits();
    // macOS hides its UI fonts from documents by a leading dot (".SF Arabic"),
    // and so its LastResort font, whose cmap claims every character.
    if traits.family.starts_with('.') {
        return Ok(None);
    }
    let coverage = probes()
        .into_iter()
        .filter(|(_, chars)| chars.iter().all(|c| face.covers(*c)))
        .fold(Coverage::default(), |acc, (flag, _)| acc.with(flag));
    Ok(Some(FontEntry {
        path: path.to_path_buf(),
        index,
        traits,
        coverage,
        weight_range: face.weight_axis(),
    }))
}

/// A minimal sfnt holding only `tables`: enough for names, style and cmap.
fn rebuild_sfnt(version: [u8; 4], tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let count = tables.len();
    let mut out = Vec::new();
    out.extend_from_slice(&version);
    out.extend_from_slice(&(count as u16).to_be_bytes());
    // searchRange, entrySelector, rangeShift: computed as the spec says.
    let power = if count == 0 { 0 } else { usize::BITS - 1 - count.leading_zeros() };
    let search = (1u16 << power) * 16;
    out.extend_from_slice(&search.to_be_bytes());
    out.extend_from_slice(&(power as u16).to_be_bytes());
    out.extend_from_slice(&((count as u16) * 16).saturating_sub(search).to_be_bytes());
    let mut data_offset = 12 + 16 * count;
    let mut data = Vec::new();
    for (tag, bytes) in tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&0u32.to_be_bytes()); // checksum: not verified by readers
        out.extend_from_slice(&(data_offset as u32).to_be_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        data.extend_from_slice(bytes);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        data_offset = 12 + 16 * count + data.len();
    }
    out.extend_from_slice(&data);
    out
}

fn read_at(file: &mut File, offset: u32, len: u32) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut buf = vec![0; len as usize];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn be16(d: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([d[at], d[at + 1]])
}

fn be32(d: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_needs_the_writing_systems_it_uses() {
        let c = needed_coverage("Hello مرحبا 2026");
        assert!(c.contains(Coverage::LATIN) && c.contains(Coverage::ARABIC));
        assert!(!c.contains(Coverage::ARABIC_EXTENDED));
        assert!(needed_coverage("گفتگو").contains(Coverage::ARABIC_EXTENDED));
        assert_eq!(
            needed_coverage("2026, ٢٠٢٦"),
            Coverage(Coverage::ARABIC.0),
            "Arabic-Indic digits are Arabic"
        );
    }

    #[test]
    fn rebuilt_sfnt_has_a_valid_directory() {
        let tables = vec![(*b"head", vec![1, 2, 3]), (*b"name", vec![4; 8])];
        let sfnt = rebuild_sfnt([0, 1, 0, 0], &tables);
        assert_eq!(be16(&sfnt, 4), 2);
        assert_eq!(be32(&sfnt, 12 + 8), 44); // first table right after 12 + 2 * 16
        assert_eq!(be32(&sfnt, 28 + 8), 48); // 3 bytes padded to 4
        assert_eq!(sfnt.len(), 56);
    }
}
