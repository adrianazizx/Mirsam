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
                ("IBMPlexSans", NamedFont { face: weight(&plex_vf, 700.0, true)?, name: "IBMPlexSans-Bold" }),
                (
                    "IBMPlexSansArabic",
                    NamedFont {
                        face: face(include_bytes!("../../../assets/fonts/IBMPlexSansArabic-SemiBold.ttf"))?,
                        name: "IBMPlexSansArabic-SemiBold",
                    },
                ),
                (
                    "Amiri-Regular",
                    NamedFont {
                        face: face(include_bytes!("../../../assets/fonts/Amiri-Bold.ttf"))?,
                        name: "Amiri-Bold",
                    },
                ),
                (
                    "NotoNaskhArabic-Regular",
                    NamedFont { face: weight(&naskh_vf, 700.0, false)?, name: "NotoNaskhArabic-Bold" },
                ),
                (
                    "NotoSansArabic-Regular",
                    NamedFont { face: weight(&sans_arabic_vf, 700.0, true)?, name: "NotoSansArabic-Bold" },
                ),
            ];
            Ok(Bundled {
                bold,
                noto_naskh: NamedFont { face: noto_naskh, name: "NotoNaskhArabic-Regular" },
                noto_sans_arabic: NamedFont { face: noto_sans_arabic, name: "NotoSansArabic-Regular" },
                plex: NamedFont { face: plex, name: "IBMPlexSans" },
                plex_arabic: NamedFont {
                    face: face(include_bytes!("../../../assets/fonts/IBMPlexSansArabic-Regular.ttf"))?,
                    name: "IBMPlexSansArabic",
                },
                amiri: NamedFont {
                    face: face(include_bytes!("../../../assets/fonts/Amiri-Regular.ttf"))?,
                    name: "Amiri-Regular",
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
        .find(|(regular, _)| *regular == font.name)
        .map_or_else(|| font.clone(), |(_, bold)| bold.clone()))
}

/// A bundled face under an oblique name, for runs drawn slanted (no italic
/// face is bundled): the slant is in the drawing, the name tells a reader
/// of the page that the run is italic, so the next edit slants it again.
pub(crate) fn oblique_of(font: &NamedFont) -> NamedFont {
    let name = match font.name {
        "IBMPlexSans" => "IBMPlexSans-Oblique",
        "IBMPlexSans-Bold" => "IBMPlexSans-BoldOblique",
        "IBMPlexSansArabic" => "IBMPlexSansArabic-Oblique",
        "IBMPlexSansArabic-SemiBold" => "IBMPlexSansArabic-SemiBoldOblique",
        "Amiri-Regular" => "Amiri-Oblique",
        "Amiri-Bold" => "Amiri-BoldOblique",
        "NotoNaskhArabic-Regular" => "NotoNaskhArabic-Oblique",
        "NotoNaskhArabic-Bold" => "NotoNaskhArabic-BoldOblique",
        "NotoSansArabic-Regular" => "NotoSansArabic-Oblique",
        "NotoSansArabic-Bold" => "NotoSansArabic-BoldOblique",
        other => other,
    };
    NamedFont { face: font.face.clone(), name }
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
