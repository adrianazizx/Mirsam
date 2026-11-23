//! The fonts bundled with the engine for writing text (spec §6.3): open
//! licence (OFL 1.1, `assets/fonts/`), compiled in, and embedded in the
//! user's PDFs as subsets.
//!
//! - **Sans:** IBM Plex Sans (instanced at weight 400) and IBM Plex Sans
//!   Arabic, the UI's own families (ADR 0019).
//! - **Naskh:** Amiri, a classical Naskh with matching Latin letters.
//!
//! Noto Naskh Arabic and Noto Sans Arabic are bundled too: candidates, with
//! the faces above, when a paragraph edited in place needs a substitute for
//! its own font (ADR 0023).
//!
//! Each face has a bold: the variable fonts instanced at weight 700, Amiri
//! Bold, and IBM Plex Sans Arabic SemiBold (its heaviest bundled weight), for
//! bold runs of an edited paragraph (ADR 0024).
//!
//! Each family lists the text's own script's face first and falls back to
//! the others for characters it lacks.

use std::borrow::Cow;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pdf::text::NamedFont;
use crate::typeset::FontFace;

/// A bundled family to write text in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontFamily {
    #[default]
    Sans,
    Naskh,
}

struct Bundled {
    plex: NamedFont,
    plex_arabic: NamedFont,
    amiri: NamedFont,
    noto_naskh: NamedFont,
    noto_sans_arabic: NamedFont,
    /// The bold of each face above, by the regular's name.
    bold: Vec<(&'static str, NamedFont)>,
}

fn bundled() -> Result<&'static Bundled> {
    static FACES: OnceLock<std::result::Result<Bundled, &'static str>> = OnceLock::new();
    FACES
        .get_or_init(|| {
            let face = |data: &'static [u8]| FontFace::from_bytes(data.to_vec(), 0).map_err(|err| err.code());
            let weight = |face: &FontFace, wght: f32, wdth: bool| {
                let axes: &[([u8; 4], f32)] =
                    if wdth { &[(*b"wght", wght), (*b"wdth", 100.0)] } else { &[(*b"wght", wght)] };
                face.with_variations(axes).map_err(|err| err.code())
            };
            let plex_vf = face(include_bytes!("../../../assets/fonts/IBMPlexSans-VF.ttf"))?;
            let naskh_vf = face(include_bytes!("../../../assets/fonts/NotoNaskhArabic-VF.ttf"))?;
            let sans_arabic_vf = face(include_bytes!("../../../assets/fonts/NotoSansArabic-VF.ttf"))?;
            let plex = weight(&plex_vf, 400.0, true)?;
            let noto_naskh = weight(&naskh_vf, 400.0, false)?;
            let noto_sans_arabic = weight(&sans_arabic_vf, 400.0, true)?;
            let bold = vec![
                (
                    "IBMPlexSans",
                    NamedFont {
                        face: weight(&plex_vf, 700.0, true)?,
                        name: Cow::Borrowed("IBMPlexSans-Bold"),
                    },
                ),
                (
                    "IBMPlexSansArabic",
                    NamedFont {
                        face: face(include_bytes!("../../../assets/fonts/IBMPlexSansArabic-SemiBold.ttf"))?,
                        name: Cow::Borrowed("IBMPlexSansArabic-SemiBold"),
                    },
                ),
                (
                    "Amiri-Regular",
                    NamedFont {
                        face: face(include_bytes!("../../../assets/fonts/Amiri-Bold.ttf"))?,
                        name: Cow::Borrowed("Amiri-Bold"),
                    },
                ),
                (
                    "NotoNaskhArabic-Regular",
                    NamedFont {
                        face: weight(&naskh_vf, 700.0, false)?,
                        name: Cow::Borrowed("NotoNaskhArabic-Bold"),
                    },
                ),
                (
                    "NotoSansArabic-Regular",
                    NamedFont {
                        face: weight(&sans_arabic_vf, 700.0, true)?,
                        name: Cow::Borrowed("NotoSansArabic-Bold"),
                    },
                ),
            ];
            Ok(Bundled {
                bold,
                noto_naskh: NamedFont { face: noto_naskh, name: Cow::Borrowed("NotoNaskhArabic-Regular") },
                noto_sans_arabic: NamedFont {
                    face: noto_sans_arabic,
                    name: Cow::Borrowed("NotoSansArabic-Regular"),
                },
                plex: NamedFont { face: plex, name: Cow::Borrowed("IBMPlexSans") },
                plex_arabic: NamedFont {
                    face: face(include_bytes!("../../../assets/fonts/IBMPlexSansArabic-Regular.ttf"))?,
                    name: Cow::Borrowed("IBMPlexSansArabic"),
                },
                amiri: NamedFont {
                    face: face(include_bytes!("../../../assets/fonts/Amiri-Regular.ttf"))?,
                    name: Cow::Borrowed("Amiri-Regular"),
                },
            })
        })
        .as_ref()
        .map_err(|code| Error::Typeset(code))
}

/// Every bundled face, for matching a PDF's font against (ADR 0023).
pub(crate) fn candidates() -> Result<Vec<NamedFont>> {
    let fonts = bundled()?;
    Ok([&fonts.plex, &fonts.plex_arabic, &fonts.amiri, &fonts.noto_naskh, &fonts.noto_sans_arabic]
        .into_iter()
        .cloned()
        .collect())
}

/// The faces that fill gaps in any other: IBM Plex Sans and Plex Sans Arabic.
pub(crate) fn fallbacks() -> Result<Vec<NamedFont>> {
    let fonts = bundled()?;
    Ok(vec![fonts.plex.clone(), fonts.plex_arabic.clone()])
}

/// The bold of a bundled face (by its name); the face itself when it is
/// already a bold or has none.
pub(crate) fn bold_of(font: &NamedFont) -> Result<NamedFont> {
    let fonts = bundled()?;
    Ok(fonts
        .bold
        .iter()
        .find(|(regular, _)| *regular == font.name.as_ref())
        .map_or_else(|| font.clone(), |(_, bold)| bold.clone()))
}

/// Each bundled face (by its name) and the name its slanted runs are
/// embedded under. The names are neutral (owner item 14, 2026-11-23): a
/// name we make must not carry a Reserved Font Name such as "Plex", so no
/// oblique name contains a family's own name. Each says its weight and that
/// it is oblique, so a reader of the page sees bold and italic.
const OBLIQUE_NAMES: [(&str, &str); 10] = [
    ("IBMPlexSans", "MirsamSans-Oblique"),
    ("IBMPlexSans-Bold", "MirsamSans-BoldOblique"),
    ("IBMPlexSansArabic", "MirsamSansArabic-Oblique"),
    ("IBMPlexSansArabic-SemiBold", "MirsamSansArabic-SemiBoldOblique"),
    ("Amiri-Regular", "MirsamNaskh-Oblique"),
    ("Amiri-Bold", "MirsamNaskh-BoldOblique"),
    ("NotoNaskhArabic-Regular", "MirsamNaskhArabic-Oblique"),
    ("NotoNaskhArabic-Bold", "MirsamNaskhArabic-BoldOblique"),
    ("NotoSansArabic-Regular", "MirsamKufi-Oblique"),
    ("NotoSansArabic-Bold", "MirsamKufi-BoldOblique"),
];

/// The oblique names written before the rename (ADR 0024 as first built),
/// still recognised in files saved then.
const LEGACY_OBLIQUE_NAMES: [(&str, &str); 10] = [
    ("IBMPlexSans", "IBMPlexSans-Oblique"),
    ("IBMPlexSans-Bold", "IBMPlexSans-BoldOblique"),
    ("IBMPlexSansArabic", "IBMPlexSansArabic-Oblique"),
    ("IBMPlexSansArabic-SemiBold", "IBMPlexSansArabic-SemiBoldOblique"),
    ("Amiri-Regular", "Amiri-Oblique"),
    ("Amiri-Bold", "Amiri-BoldOblique"),
    ("NotoNaskhArabic-Regular", "NotoNaskhArabic-Oblique"),
    ("NotoNaskhArabic-Bold", "NotoNaskhArabic-BoldOblique"),
    ("NotoSansArabic-Regular", "NotoSansArabic-Oblique"),
    ("NotoSansArabic-Bold", "NotoSansArabic-BoldOblique"),
];

/// A bundled face under an oblique name, for runs drawn slanted (no italic
/// face is bundled): the slant is in the drawing, the name tells a reader
/// of the page that the run is italic, so the next edit slants it again.
pub(crate) fn oblique_of(font: &NamedFont) -> NamedFont {
    let name: Cow<'static, str> = match OBLIQUE_NAMES.iter().find(|(upright, _)| *upright == font.name) {
        Some((_, oblique)) => Cow::Borrowed(oblique),
        None => Cow::Owned(format!("MirsamOblique-{}", font.name)),
    };
    NamedFont { face: font.face.clone(), name }
}

/// The bundled face a page font is an oblique of, when it is one we wrote
/// (`MirsamSans-Oblique`, or a name from before the rename): its upright
/// face's name. The PDF name may carry a subset tag (`ABCDEF+`).
pub(crate) fn upright_of_oblique(pdf_name: &str) -> Option<&'static str> {
    let name = match pdf_name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => pdf_name,
    };
    OBLIQUE_NAMES
        .iter()
        .chain(&LEGACY_OBLIQUE_NAMES)
        .find(|(_, oblique)| *oblique == name)
        .map(|(upright, _)| *upright)
}

/// The family a bundled face belongs with: Naskh for the Naskh faces (and
/// Amiri), Sans for the rest.
pub(crate) fn family_of(name: &str) -> FontFamily {
    if name.starts_with("Amiri") || name.starts_with("NotoNaskh") {
        FontFamily::Naskh
    } else {
        FontFamily::Sans
    }
}

/// Whether the text's first strong character is right to left (UAX #9 P2).
pub(crate) fn starts_rtl(text: &str) -> bool {
    matches!(unicode_bidi::get_base_direction(text), unicode_bidi::Direction::Rtl)
}

/// The faces to lay `text` out in `family`, the first tried first.
pub(crate) fn for_text(family: FontFamily, text: &str) -> Result<Vec<NamedFont>> {
    let fonts = bundled()?;
    Ok(match family {
        FontFamily::Sans if starts_rtl(text) => vec![fonts.plex_arabic.clone(), fonts.plex.clone()],
        FontFamily::Sans => vec![fonts.plex.clone(), fonts.plex_arabic.clone()],
        // Amiri covers both scripts; Plex fills any gap.
        FontFamily::Naskh => vec![fonts.amiri.clone(), fonts.plex.clone(), fonts.plex_arabic.clone()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_face_and_its_bold_has_a_neutral_oblique_name() {
        for face in candidates().expect("bundled") {
            let bold = bold_of(&face).expect("bold");
            for font in [face, bold] {
                let oblique = oblique_of(&font).name;
                assert!(OBLIQUE_NAMES.iter().any(|(_, o)| *o == oblique), "{} has no entry", font.name);
                assert!(oblique.starts_with("Mirsam") && oblique.ends_with("Oblique"), "{oblique}");
                // No Reserved Font Name, nor any family's own name, in a name we made.
                for family in ["Plex", "IBM", "Amiri", "Noto"] {
                    assert!(!oblique.contains(family), "{oblique}");
                }
                assert_eq!(upright_of_oblique(&oblique), Some(font.name.as_ref()));
                assert_eq!(upright_of_oblique(&format!("QWERTY+{oblique}")), Some(font.name.as_ref()));
            }
        }
    }

    #[test]
    fn the_names_from_before_the_rename_are_still_ours() {
        assert_eq!(upright_of_oblique("ABCDEF+IBMPlexSans-BoldOblique"), Some("IBMPlexSans-Bold"));
        assert_eq!(upright_of_oblique("Amiri-Oblique"), Some("Amiri-Regular"));
        // Another producer's oblique, and our upright faces, are not.
        assert_eq!(upright_of_oblique("Helvetica-Oblique"), None);
        assert_eq!(upright_of_oblique("ABCDEF+IBMPlexSans"), None);
        assert_eq!(upright_of_oblique("abcdef+MirsamSans-Oblique"), None);
    }
}
