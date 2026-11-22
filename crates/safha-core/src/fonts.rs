//! The fonts bundled with the engine for writing text (spec §6.3): open
//! licence (OFL 1.1, `assets/fonts/`), compiled in, and embedded in the
//! user's PDFs as subsets.
//!
//! - **Sans:** IBM Plex Sans (instanced at weight 400) and IBM Plex Sans
//!   Arabic, the UI's own families (ADR 0019).
//! - **Naskh:** Amiri, a classical Naskh with matching Latin letters.
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
}

fn bundled() -> Result<&'static Bundled> {
    static FACES: OnceLock<std::result::Result<Bundled, &'static str>> = OnceLock::new();
    FACES
        .get_or_init(|| {
            let face = |data: &'static [u8]| FontFace::from_bytes(data.to_vec(), 0).map_err(|err| err.code());
            let plex = face(include_bytes!("../../../assets/fonts/IBMPlexSans-VF.ttf"))?
                .with_variations(&[(*b"wght", 400.0), (*b"wdth", 100.0)])
                .map_err(|err| err.code())?;
            Ok(Bundled {
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

/// The faces to lay `text` out in `family`, the first tried first, each with
/// the PostScript name it is embedded under.
pub fn faces_for(family: FontFamily, text: &str) -> Result<Vec<(crate::typeset::FontFace, &'static str)>> {
    Ok(for_text(family, text)?.into_iter().map(|font| (font.face, font.name)).collect())
}
