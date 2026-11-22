//! Arabic letters as presentation forms (ADR 0026): what HarfBuzz calls its
//! Arabic fallback shaper.
//!
//! A font embedded in a PDF is usually a subset with no `GSUB`, so a shaper
//! cannot pick a letter's joined form from it. Producers that keep a `cmap`
//! map the forms they drew to Unicode's presentation-form characters
//! (U+FB50–U+FEFF) instead. Choosing each letter's form here, from its
//! joining type (Unicode's `Joining_Type`, through ICU), and asking the font
//! for that character draws the same glyphs with no `GSUB`. Lam followed by
//! alef becomes its ligature, as Arabic requires.
//!
//! What is written still says the letters, not the forms: [`logical_text`]
//! folds presentation forms back for `/ToUnicode` and `/ActualText`.

use std::ops::Range;

use icu_properties::CodePointMapData;
use icu_properties::props::JoiningType;
use unicode_normalization::UnicodeNormalization;

/// Generated from Unicode 13's compatibility decompositions of U+FB50–U+FDFF
/// and U+FE70–U+FEFF: each letter's isolated, final, initial and medial
/// forms (0 where Unicode encodes none).
const FORMS: &[(char, [u32; 4])] = &[
    ('\u{0621}', [0xFE80, 0x0000, 0x0000, 0x0000]),
    ('\u{0622}', [0xFE81, 0xFE82, 0x0000, 0x0000]),
    ('\u{0623}', [0xFE83, 0xFE84, 0x0000, 0x0000]),
    ('\u{0624}', [0xFE85, 0xFE86, 0x0000, 0x0000]),
    ('\u{0625}', [0xFE87, 0xFE88, 0x0000, 0x0000]),
    ('\u{0626}', [0xFE89, 0xFE8A, 0xFE8B, 0xFE8C]),
    ('\u{0627}', [0xFE8D, 0xFE8E, 0x0000, 0x0000]),
    ('\u{0628}', [0xFE8F, 0xFE90, 0xFE91, 0xFE92]),
    ('\u{0629}', [0xFE93, 0xFE94, 0x0000, 0x0000]),
    ('\u{062A}', [0xFE95, 0xFE96, 0xFE97, 0xFE98]),
    ('\u{062B}', [0xFE99, 0xFE9A, 0xFE9B, 0xFE9C]),
    ('\u{062C}', [0xFE9D, 0xFE9E, 0xFE9F, 0xFEA0]),
    ('\u{062D}', [0xFEA1, 0xFEA2, 0xFEA3, 0xFEA4]),
    ('\u{062E}', [0xFEA5, 0xFEA6, 0xFEA7, 0xFEA8]),
    ('\u{062F}', [0xFEA9, 0xFEAA, 0x0000, 0x0000]),
    ('\u{0630}', [0xFEAB, 0xFEAC, 0x0000, 0x0000]),
    ('\u{0631}', [0xFEAD, 0xFEAE, 0x0000, 0x0000]),
    ('\u{0632}', [0xFEAF, 0xFEB0, 0x0000, 0x0000]),
    ('\u{0633}', [0xFEB1, 0xFEB2, 0xFEB3, 0xFEB4]),
    ('\u{0634}', [0xFEB5, 0xFEB6, 0xFEB7, 0xFEB8]),
    ('\u{0635}', [0xFEB9, 0xFEBA, 0xFEBB, 0xFEBC]),
    ('\u{0636}', [0xFEBD, 0xFEBE, 0xFEBF, 0xFEC0]),
    ('\u{0637}', [0xFEC1, 0xFEC2, 0xFEC3, 0xFEC4]),
    ('\u{0638}', [0xFEC5, 0xFEC6, 0xFEC7, 0xFEC8]),
    ('\u{0639}', [0xFEC9, 0xFECA, 0xFECB, 0xFECC]),
    ('\u{063A}', [0xFECD, 0xFECE, 0xFECF, 0xFED0]),
    ('\u{0641}', [0xFED1, 0xFED2, 0xFED3, 0xFED4]),
    ('\u{0642}', [0xFED5, 0xFED6, 0xFED7, 0xFED8]),
    ('\u{0643}', [0xFED9, 0xFEDA, 0xFEDB, 0xFEDC]),
    ('\u{0644}', [0xFEDD, 0xFEDE, 0xFEDF, 0xFEE0]),
    ('\u{0645}', [0xFEE1, 0xFEE2, 0xFEE3, 0xFEE4]),
    ('\u{0646}', [0xFEE5, 0xFEE6, 0xFEE7, 0xFEE8]),
    ('\u{0647}', [0xFEE9, 0xFEEA, 0xFEEB, 0xFEEC]),
    ('\u{0648}', [0xFEED, 0xFEEE, 0x0000, 0x0000]),
    ('\u{0649}', [0xFEEF, 0xFEF0, 0xFBE8, 0xFBE9]),
    ('\u{064A}', [0xFEF1, 0xFEF2, 0xFEF3, 0xFEF4]),
    ('\u{0671}', [0xFB50, 0xFB51, 0x0000, 0x0000]),
    ('\u{0677}', [0xFBDD, 0x0000, 0x0000, 0x0000]),
    ('\u{0679}', [0xFB66, 0xFB67, 0xFB68, 0xFB69]),
    ('\u{067A}', [0xFB5E, 0xFB5F, 0xFB60, 0xFB61]),
    ('\u{067B}', [0xFB52, 0xFB53, 0xFB54, 0xFB55]),
    ('\u{067E}', [0xFB56, 0xFB57, 0xFB58, 0xFB59]),
    ('\u{067F}', [0xFB62, 0xFB63, 0xFB64, 0xFB65]),
    ('\u{0680}', [0xFB5A, 0xFB5B, 0xFB5C, 0xFB5D]),
    ('\u{0683}', [0xFB76, 0xFB77, 0xFB78, 0xFB79]),
    ('\u{0684}', [0xFB72, 0xFB73, 0xFB74, 0xFB75]),
    ('\u{0686}', [0xFB7A, 0xFB7B, 0xFB7C, 0xFB7D]),
    ('\u{0687}', [0xFB7E, 0xFB7F, 0xFB80, 0xFB81]),
    ('\u{0688}', [0xFB88, 0xFB89, 0x0000, 0x0000]),
    ('\u{068C}', [0xFB84, 0xFB85, 0x0000, 0x0000]),
    ('\u{068D}', [0xFB82, 0xFB83, 0x0000, 0x0000]),
    ('\u{068E}', [0xFB86, 0xFB87, 0x0000, 0x0000]),
    ('\u{0691}', [0xFB8C, 0xFB8D, 0x0000, 0x0000]),
    ('\u{0698}', [0xFB8A, 0xFB8B, 0x0000, 0x0000]),
    ('\u{06A4}', [0xFB6A, 0xFB6B, 0xFB6C, 0xFB6D]),
    ('\u{06A6}', [0xFB6E, 0xFB6F, 0xFB70, 0xFB71]),
    ('\u{06A9}', [0xFB8E, 0xFB8F, 0xFB90, 0xFB91]),
    ('\u{06AD}', [0xFBD3, 0xFBD4, 0xFBD5, 0xFBD6]),
    ('\u{06AF}', [0xFB92, 0xFB93, 0xFB94, 0xFB95]),
    ('\u{06B1}', [0xFB9A, 0xFB9B, 0xFB9C, 0xFB9D]),
    ('\u{06B3}', [0xFB96, 0xFB97, 0xFB98, 0xFB99]),
    ('\u{06BA}', [0xFB9E, 0xFB9F, 0x0000, 0x0000]),
    ('\u{06BB}', [0xFBA0, 0xFBA1, 0xFBA2, 0xFBA3]),
    ('\u{06BE}', [0xFBAA, 0xFBAB, 0xFBAC, 0xFBAD]),
    ('\u{06C0}', [0xFBA4, 0xFBA5, 0x0000, 0x0000]),
    ('\u{06C1}', [0xFBA6, 0xFBA7, 0xFBA8, 0xFBA9]),
    ('\u{06C5}', [0xFBE0, 0xFBE1, 0x0000, 0x0000]),
    ('\u{06C6}', [0xFBD9, 0xFBDA, 0x0000, 0x0000]),
    ('\u{06C7}', [0xFBD7, 0xFBD8, 0x0000, 0x0000]),
    ('\u{06C8}', [0xFBDB, 0xFBDC, 0x0000, 0x0000]),
    ('\u{06C9}', [0xFBE2, 0xFBE3, 0x0000, 0x0000]),
    ('\u{06CB}', [0xFBDE, 0xFBDF, 0x0000, 0x0000]),
    ('\u{06CC}', [0xFBFC, 0xFBFD, 0xFBFE, 0xFBFF]),
    ('\u{06D0}', [0xFBE4, 0xFBE5, 0xFBE6, 0xFBE7]),
    ('\u{06D2}', [0xFBAE, 0xFBAF, 0x0000, 0x0000]),
    ('\u{06D3}', [0xFBB0, 0xFBB1, 0x0000, 0x0000]),
];
const LAM_ALEF: &[(char, [u32; 2])] = &[
    ('\u{0622}', [0xFEF5, 0xFEF6]),
    ('\u{0623}', [0xFEF7, 0xFEF8]),
    ('\u{0625}', [0xFEF9, 0xFEFA]),
    ('\u{0627}', [0xFEFB, 0xFEFC]),
];

/// Optional ligatures of two or three letters, from the same data: the
/// letters, and the ligature's isolated, final, initial and medial forms.
const LIGATURES: &[(&str, [u32; 4])] = &[
    ("\u{0628}\u{062D}\u{064A}", [0x0000, 0xFDC2, 0x0000, 0x0000]),
    ("\u{0628}\u{062E}\u{064A}", [0x0000, 0xFD9E, 0x0000, 0x0000]),
    ("\u{062A}\u{062C}\u{0645}", [0x0000, 0x0000, 0xFD50, 0x0000]),
    ("\u{062A}\u{062C}\u{0649}", [0x0000, 0xFDA0, 0x0000, 0x0000]),
    ("\u{062A}\u{062C}\u{064A}", [0x0000, 0xFD9F, 0x0000, 0x0000]),
    ("\u{062A}\u{062D}\u{062C}", [0x0000, 0xFD51, 0xFD52, 0x0000]),
    ("\u{062A}\u{062D}\u{0645}", [0x0000, 0x0000, 0xFD53, 0x0000]),
    ("\u{062A}\u{062E}\u{0645}", [0x0000, 0x0000, 0xFD54, 0x0000]),
    ("\u{062A}\u{062E}\u{0649}", [0x0000, 0xFDA2, 0x0000, 0x0000]),
    ("\u{062A}\u{062E}\u{064A}", [0x0000, 0xFDA1, 0x0000, 0x0000]),
    ("\u{062A}\u{0645}\u{062C}", [0x0000, 0x0000, 0xFD55, 0x0000]),
    ("\u{062A}\u{0645}\u{062D}", [0x0000, 0x0000, 0xFD56, 0x0000]),
    ("\u{062A}\u{0645}\u{062E}", [0x0000, 0x0000, 0xFD57, 0x0000]),
    ("\u{062A}\u{0645}\u{0649}", [0x0000, 0xFDA4, 0x0000, 0x0000]),
    ("\u{062A}\u{0645}\u{064A}", [0x0000, 0xFDA3, 0x0000, 0x0000]),
    ("\u{062C}\u{062D}\u{0649}", [0x0000, 0xFDA6, 0x0000, 0x0000]),
    ("\u{062C}\u{062D}\u{064A}", [0x0000, 0xFDBE, 0x0000, 0x0000]),
    ("\u{062C}\u{0645}\u{062D}", [0x0000, 0xFD58, 0xFD59, 0x0000]),
    ("\u{062C}\u{0645}\u{0649}", [0x0000, 0xFDA7, 0x0000, 0x0000]),
    ("\u{062C}\u{0645}\u{064A}", [0x0000, 0xFDA5, 0x0000, 0x0000]),
    ("\u{062D}\u{062C}\u{064A}", [0x0000, 0xFDBF, 0x0000, 0x0000]),
    ("\u{062D}\u{0645}\u{0649}", [0x0000, 0xFD5B, 0x0000, 0x0000]),
    ("\u{062D}\u{0645}\u{064A}", [0x0000, 0xFD5A, 0x0000, 0x0000]),
    ("\u{0633}\u{062C}\u{062D}", [0x0000, 0x0000, 0xFD5D, 0x0000]),
    ("\u{0633}\u{062C}\u{0649}", [0x0000, 0xFD5E, 0x0000, 0x0000]),
    ("\u{0633}\u{062D}\u{062C}", [0x0000, 0x0000, 0xFD5C, 0x0000]),
    ("\u{0633}\u{062E}\u{0649}", [0x0000, 0xFDA8, 0x0000, 0x0000]),
    ("\u{0633}\u{062E}\u{064A}", [0x0000, 0xFDC6, 0x0000, 0x0000]),
    ("\u{0633}\u{0645}\u{062C}", [0x0000, 0x0000, 0xFD61, 0x0000]),
    ("\u{0633}\u{0645}\u{062D}", [0x0000, 0xFD5F, 0xFD60, 0x0000]),
    ("\u{0633}\u{0645}\u{0645}", [0x0000, 0xFD62, 0xFD63, 0x0000]),
    ("\u{0634}\u{062C}\u{064A}", [0x0000, 0xFD69, 0x0000, 0x0000]),
    ("\u{0634}\u{062D}\u{0645}", [0x0000, 0xFD67, 0xFD68, 0x0000]),
    ("\u{0634}\u{062D}\u{064A}", [0x0000, 0xFDAA, 0x0000, 0x0000]),
    ("\u{0634}\u{0645}\u{062E}", [0x0000, 0xFD6A, 0xFD6B, 0x0000]),
    ("\u{0634}\u{0645}\u{0645}", [0x0000, 0xFD6C, 0xFD6D, 0x0000]),
    ("\u{0635}\u{062D}\u{062D}", [0x0000, 0xFD64, 0xFD65, 0x0000]),
    ("\u{0635}\u{062D}\u{064A}", [0x0000, 0xFDA9, 0x0000, 0x0000]),
    ("\u{0635}\u{0644}\u{0649}", [0xFDF9, 0x0000, 0x0000, 0x0000]),
    ("\u{0635}\u{0644}\u{06D2}", [0xFDF0, 0x0000, 0x0000, 0x0000]),
    ("\u{0635}\u{0645}\u{0645}", [0x0000, 0xFD66, 0xFDC5, 0x0000]),
    ("\u{0636}\u{062D}\u{0649}", [0x0000, 0xFD6E, 0x0000, 0x0000]),
    ("\u{0636}\u{062D}\u{064A}", [0x0000, 0xFDAB, 0x0000, 0x0000]),
    ("\u{0636}\u{062E}\u{0645}", [0x0000, 0xFD6F, 0xFD70, 0x0000]),
    ("\u{0637}\u{0645}\u{062D}", [0x0000, 0xFD71, 0xFD72, 0x0000]),
    ("\u{0637}\u{0645}\u{0645}", [0x0000, 0x0000, 0xFD73, 0x0000]),
    ("\u{0637}\u{0645}\u{064A}", [0x0000, 0xFD74, 0x0000, 0x0000]),
    ("\u{0639}\u{062C}\u{0645}", [0x0000, 0xFD75, 0xFDC4, 0x0000]),
    ("\u{0639}\u{0645}\u{0645}", [0x0000, 0xFD76, 0xFD77, 0x0000]),
    ("\u{0639}\u{0645}\u{0649}", [0x0000, 0xFD78, 0x0000, 0x0000]),
    ("\u{0639}\u{0645}\u{064A}", [0x0000, 0xFDB6, 0x0000, 0x0000]),
    ("\u{063A}\u{0645}\u{0645}", [0x0000, 0xFD79, 0x0000, 0x0000]),
    ("\u{063A}\u{0645}\u{0649}", [0x0000, 0xFD7B, 0x0000, 0x0000]),
    ("\u{063A}\u{0645}\u{064A}", [0x0000, 0xFD7A, 0x0000, 0x0000]),
    ("\u{0641}\u{062E}\u{0645}", [0x0000, 0xFD7C, 0xFD7D, 0x0000]),
    ("\u{0641}\u{0645}\u{064A}", [0x0000, 0xFDC1, 0x0000, 0x0000]),
    ("\u{0642}\u{0644}\u{06D2}", [0xFDF1, 0x0000, 0x0000, 0x0000]),
    ("\u{0642}\u{0645}\u{062D}", [0x0000, 0xFD7E, 0xFDB4, 0x0000]),
    ("\u{0642}\u{0645}\u{0645}", [0x0000, 0xFD7F, 0x0000, 0x0000]),
    ("\u{0642}\u{0645}\u{064A}", [0x0000, 0xFDB2, 0x0000, 0x0000]),
    ("\u{0643}\u{0645}\u{0645}", [0x0000, 0xFDBB, 0xFDC3, 0x0000]),
    ("\u{0643}\u{0645}\u{064A}", [0x0000, 0xFDB7, 0x0000, 0x0000]),
    ("\u{0644}\u{062C}\u{062C}", [0x0000, 0xFD84, 0xFD83, 0x0000]),
    ("\u{0644}\u{062C}\u{0645}", [0x0000, 0xFDBC, 0xFDBA, 0x0000]),
    ("\u{0644}\u{062C}\u{064A}", [0x0000, 0xFDAC, 0x0000, 0x0000]),
    ("\u{0644}\u{062D}\u{0645}", [0x0000, 0xFD80, 0xFDB5, 0x0000]),
    ("\u{0644}\u{062D}\u{0649}", [0x0000, 0xFD82, 0x0000, 0x0000]),
    ("\u{0644}\u{062D}\u{064A}", [0x0000, 0xFD81, 0x0000, 0x0000]),
    ("\u{0644}\u{062E}\u{0645}", [0x0000, 0xFD85, 0xFD86, 0x0000]),
    ("\u{0644}\u{0645}\u{062D}", [0x0000, 0xFD87, 0xFD88, 0x0000]),
    ("\u{0644}\u{0645}\u{064A}", [0x0000, 0xFDAD, 0x0000, 0x0000]),
    ("\u{0645}\u{062C}\u{062D}", [0x0000, 0x0000, 0xFD8C, 0x0000]),
    ("\u{0645}\u{062C}\u{062E}", [0x0000, 0x0000, 0xFD92, 0x0000]),
    ("\u{0645}\u{062C}\u{0645}", [0x0000, 0x0000, 0xFD8D, 0x0000]),
    ("\u{0645}\u{062C}\u{064A}", [0x0000, 0xFDC0, 0x0000, 0x0000]),
    ("\u{0645}\u{062D}\u{062C}", [0x0000, 0x0000, 0xFD89, 0x0000]),
    ("\u{0645}\u{062D}\u{0645}", [0x0000, 0x0000, 0xFD8A, 0x0000]),
    ("\u{0645}\u{062D}\u{064A}", [0x0000, 0xFD8B, 0x0000, 0x0000]),
    ("\u{0645}\u{062E}\u{062C}", [0x0000, 0x0000, 0xFD8E, 0x0000]),
    ("\u{0645}\u{062E}\u{0645}", [0x0000, 0x0000, 0xFD8F, 0x0000]),
    ("\u{0645}\u{062E}\u{064A}", [0x0000, 0xFDB9, 0x0000, 0x0000]),
    ("\u{0645}\u{0645}\u{064A}", [0x0000, 0xFDB1, 0x0000, 0x0000]),
    ("\u{0646}\u{062C}\u{062D}", [0x0000, 0xFDBD, 0xFDB8, 0x0000]),
    ("\u{0646}\u{062C}\u{0645}", [0x0000, 0xFD97, 0xFD98, 0x0000]),
    ("\u{0646}\u{062C}\u{0649}", [0x0000, 0xFD99, 0x0000, 0x0000]),
    ("\u{0646}\u{062C}\u{064A}", [0x0000, 0xFDC7, 0x0000, 0x0000]),
    ("\u{0646}\u{062D}\u{0645}", [0x0000, 0x0000, 0xFD95, 0x0000]),
    ("\u{0646}\u{062D}\u{0649}", [0x0000, 0xFD96, 0x0000, 0x0000]),
    ("\u{0646}\u{062D}\u{064A}", [0x0000, 0xFDB3, 0x0000, 0x0000]),
    ("\u{0646}\u{0645}\u{0649}", [0x0000, 0xFD9B, 0x0000, 0x0000]),
    ("\u{0646}\u{0645}\u{064A}", [0x0000, 0xFD9A, 0x0000, 0x0000]),
    ("\u{0647}\u{0645}\u{062C}", [0x0000, 0x0000, 0xFD93, 0x0000]),
    ("\u{0647}\u{0645}\u{0645}", [0x0000, 0x0000, 0xFD94, 0x0000]),
    ("\u{064A}\u{062C}\u{064A}", [0x0000, 0xFDAF, 0x0000, 0x0000]),
    ("\u{064A}\u{062D}\u{064A}", [0x0000, 0xFDAE, 0x0000, 0x0000]),
    ("\u{064A}\u{0645}\u{0645}", [0x0000, 0xFD9C, 0xFD9D, 0x0000]),
    ("\u{064A}\u{0645}\u{064A}", [0x0000, 0xFDB0, 0x0000, 0x0000]),
    ("\u{0626}\u{0627}", [0xFBEA, 0xFBEB, 0x0000, 0x0000]),
    ("\u{0626}\u{062C}", [0xFC00, 0x0000, 0xFC97, 0x0000]),
    ("\u{0626}\u{062D}", [0xFC01, 0x0000, 0xFC98, 0x0000]),
    ("\u{0626}\u{062E}", [0x0000, 0x0000, 0xFC99, 0x0000]),
    ("\u{0626}\u{0631}", [0x0000, 0xFC64, 0x0000, 0x0000]),
    ("\u{0626}\u{0632}", [0x0000, 0xFC65, 0x0000, 0x0000]),
    ("\u{0626}\u{0645}", [0xFC02, 0xFC66, 0xFC9A, 0xFCDF]),
    ("\u{0626}\u{0646}", [0x0000, 0xFC67, 0x0000, 0x0000]),
    ("\u{0626}\u{0647}", [0x0000, 0x0000, 0xFC9B, 0xFCE0]),
    ("\u{0626}\u{0648}", [0xFBEE, 0xFBEF, 0x0000, 0x0000]),
    ("\u{0626}\u{0649}", [0xFBF9, 0xFBFA, 0xFBFB, 0x0000]),
    ("\u{0626}\u{064A}", [0xFC04, 0xFC69, 0x0000, 0x0000]),
    ("\u{0626}\u{06C6}", [0xFBF2, 0xFBF3, 0x0000, 0x0000]),
    ("\u{0626}\u{06C7}", [0xFBF0, 0xFBF1, 0x0000, 0x0000]),
    ("\u{0626}\u{06C8}", [0xFBF4, 0xFBF5, 0x0000, 0x0000]),
    ("\u{0626}\u{06D0}", [0xFBF6, 0xFBF7, 0xFBF8, 0x0000]),
    ("\u{0626}\u{06D5}", [0xFBEC, 0xFBED, 0x0000, 0x0000]),
    ("\u{0628}\u{062C}", [0xFC05, 0x0000, 0xFC9C, 0x0000]),
    ("\u{0628}\u{062D}", [0xFC06, 0x0000, 0xFC9D, 0x0000]),
    ("\u{0628}\u{062E}", [0xFC07, 0x0000, 0xFC9E, 0x0000]),
    ("\u{0628}\u{0631}", [0x0000, 0xFC6A, 0x0000, 0x0000]),
    ("\u{0628}\u{0632}", [0x0000, 0xFC6B, 0x0000, 0x0000]),
    ("\u{0628}\u{0645}", [0xFC08, 0xFC6C, 0xFC9F, 0xFCE1]),
    ("\u{0628}\u{0646}", [0x0000, 0xFC6D, 0x0000, 0x0000]),
    ("\u{0628}\u{0647}", [0x0000, 0x0000, 0xFCA0, 0xFCE2]),
    ("\u{0628}\u{0649}", [0xFC09, 0xFC6E, 0x0000, 0x0000]),
    ("\u{0628}\u{064A}", [0xFC0A, 0xFC6F, 0x0000, 0x0000]),
    ("\u{062A}\u{062C}", [0xFC0B, 0x0000, 0xFCA1, 0x0000]),
    ("\u{062A}\u{062D}", [0xFC0C, 0x0000, 0xFCA2, 0x0000]),
    ("\u{062A}\u{062E}", [0xFC0D, 0x0000, 0xFCA3, 0x0000]),
    ("\u{062A}\u{0631}", [0x0000, 0xFC70, 0x0000, 0x0000]),
    ("\u{062A}\u{0632}", [0x0000, 0xFC71, 0x0000, 0x0000]),
    ("\u{062A}\u{0645}", [0xFC0E, 0xFC72, 0xFCA4, 0xFCE3]),
    ("\u{062A}\u{0646}", [0x0000, 0xFC73, 0x0000, 0x0000]),
    ("\u{062A}\u{0647}", [0x0000, 0x0000, 0xFCA5, 0xFCE4]),
    ("\u{062A}\u{0649}", [0xFC0F, 0xFC74, 0x0000, 0x0000]),
    ("\u{062A}\u{064A}", [0xFC10, 0xFC75, 0x0000, 0x0000]),
    ("\u{062B}\u{062C}", [0xFC11, 0x0000, 0x0000, 0x0000]),
    ("\u{062B}\u{0631}", [0x0000, 0xFC76, 0x0000, 0x0000]),
    ("\u{062B}\u{0632}", [0x0000, 0xFC77, 0x0000, 0x0000]),
    ("\u{062B}\u{0645}", [0xFC12, 0xFC78, 0xFCA6, 0xFCE5]),
    ("\u{062B}\u{0646}", [0x0000, 0xFC79, 0x0000, 0x0000]),
    ("\u{062B}\u{0647}", [0x0000, 0x0000, 0x0000, 0xFCE6]),
    ("\u{062B}\u{0649}", [0xFC13, 0xFC7A, 0x0000, 0x0000]),
    ("\u{062B}\u{064A}", [0xFC14, 0xFC7B, 0x0000, 0x0000]),
    ("\u{062C}\u{062D}", [0xFC15, 0x0000, 0xFCA7, 0x0000]),
    ("\u{062C}\u{0645}", [0xFC16, 0x0000, 0xFCA8, 0x0000]),
    ("\u{062C}\u{0649}", [0xFD01, 0xFD1D, 0x0000, 0x0000]),
    ("\u{062C}\u{064A}", [0xFD02, 0xFD1E, 0x0000, 0x0000]),
    ("\u{062D}\u{062C}", [0xFC17, 0x0000, 0xFCA9, 0x0000]),
    ("\u{062D}\u{0645}", [0xFC18, 0x0000, 0xFCAA, 0x0000]),
    ("\u{062D}\u{0649}", [0xFCFF, 0xFD1B, 0x0000, 0x0000]),
    ("\u{062D}\u{064A}", [0xFD00, 0xFD1C, 0x0000, 0x0000]),
    ("\u{062E}\u{062C}", [0xFC19, 0x0000, 0xFCAB, 0x0000]),
    ("\u{062E}\u{062D}", [0xFC1A, 0x0000, 0x0000, 0x0000]),
    ("\u{062E}\u{0645}", [0xFC1B, 0x0000, 0xFCAC, 0x0000]),
    ("\u{062E}\u{0649}", [0xFD03, 0xFD1F, 0x0000, 0x0000]),
    ("\u{062E}\u{064A}", [0xFD04, 0xFD20, 0x0000, 0x0000]),
    ("\u{0633}\u{062C}", [0xFC1C, 0x0000, 0xFCAD, 0xFD34]),
    ("\u{0633}\u{062D}", [0xFC1D, 0x0000, 0xFCAE, 0xFD35]),
    ("\u{0633}\u{062E}", [0xFC1E, 0x0000, 0xFCAF, 0xFD36]),
    ("\u{0633}\u{0631}", [0xFD0E, 0xFD2A, 0x0000, 0x0000]),
    ("\u{0633}\u{0645}", [0xFC1F, 0x0000, 0xFCB0, 0xFCE7]),
    ("\u{0633}\u{0647}", [0x0000, 0x0000, 0xFD31, 0xFCE8]),
    ("\u{0633}\u{0649}", [0xFCFB, 0xFD17, 0x0000, 0x0000]),
    ("\u{0633}\u{064A}", [0xFCFC, 0xFD18, 0x0000, 0x0000]),
    ("\u{0634}\u{062C}", [0xFD09, 0xFD25, 0xFD2D, 0xFD37]),
    ("\u{0634}\u{062D}", [0xFD0A, 0xFD26, 0xFD2E, 0xFD38]),
    ("\u{0634}\u{062E}", [0xFD0B, 0xFD27, 0xFD2F, 0xFD39]),
    ("\u{0634}\u{0631}", [0xFD0D, 0xFD29, 0x0000, 0x0000]),
    ("\u{0634}\u{0645}", [0xFD0C, 0xFD28, 0xFD30, 0xFCE9]),
    ("\u{0634}\u{0647}", [0x0000, 0x0000, 0xFD32, 0xFCEA]),
    ("\u{0634}\u{0649}", [0xFCFD, 0xFD19, 0x0000, 0x0000]),
    ("\u{0634}\u{064A}", [0xFCFE, 0xFD1A, 0x0000, 0x0000]),
    ("\u{0635}\u{062D}", [0xFC20, 0x0000, 0xFCB1, 0x0000]),
    ("\u{0635}\u{062E}", [0x0000, 0x0000, 0xFCB2, 0x0000]),
    ("\u{0635}\u{0631}", [0xFD0F, 0xFD2B, 0x0000, 0x0000]),
    ("\u{0635}\u{0645}", [0xFC21, 0x0000, 0xFCB3, 0x0000]),
    ("\u{0635}\u{0649}", [0xFD05, 0xFD21, 0x0000, 0x0000]),
    ("\u{0635}\u{064A}", [0xFD06, 0xFD22, 0x0000, 0x0000]),
    ("\u{0636}\u{062C}", [0xFC22, 0x0000, 0xFCB4, 0x0000]),
    ("\u{0636}\u{062D}", [0xFC23, 0x0000, 0xFCB5, 0x0000]),
    ("\u{0636}\u{062E}", [0xFC24, 0x0000, 0xFCB6, 0x0000]),
    ("\u{0636}\u{0631}", [0xFD10, 0xFD2C, 0x0000, 0x0000]),
    ("\u{0636}\u{0645}", [0xFC25, 0x0000, 0xFCB7, 0x0000]),
    ("\u{0636}\u{0649}", [0xFD07, 0xFD23, 0x0000, 0x0000]),
    ("\u{0636}\u{064A}", [0xFD08, 0xFD24, 0x0000, 0x0000]),
    ("\u{0637}\u{062D}", [0xFC26, 0x0000, 0xFCB8, 0x0000]),
    ("\u{0637}\u{0645}", [0xFC27, 0x0000, 0xFD33, 0xFD3A]),
    ("\u{0637}\u{0649}", [0xFCF5, 0xFD11, 0x0000, 0x0000]),
    ("\u{0637}\u{064A}", [0xFCF6, 0xFD12, 0x0000, 0x0000]),
    ("\u{0638}\u{0645}", [0xFC28, 0x0000, 0xFCB9, 0xFD3B]),
    ("\u{0639}\u{062C}", [0xFC29, 0x0000, 0xFCBA, 0x0000]),
    ("\u{0639}\u{0645}", [0xFC2A, 0x0000, 0xFCBB, 0x0000]),
    ("\u{0639}\u{0649}", [0xFCF7, 0xFD13, 0x0000, 0x0000]),
    ("\u{0639}\u{064A}", [0xFCF8, 0xFD14, 0x0000, 0x0000]),
    ("\u{063A}\u{062C}", [0xFC2B, 0x0000, 0xFCBC, 0x0000]),
    ("\u{063A}\u{0645}", [0xFC2C, 0x0000, 0xFCBD, 0x0000]),
    ("\u{063A}\u{0649}", [0xFCF9, 0xFD15, 0x0000, 0x0000]),
    ("\u{063A}\u{064A}", [0xFCFA, 0xFD16, 0x0000, 0x0000]),
    ("\u{0641}\u{062C}", [0xFC2D, 0x0000, 0xFCBE, 0x0000]),
    ("\u{0641}\u{062D}", [0xFC2E, 0x0000, 0xFCBF, 0x0000]),
    ("\u{0641}\u{062E}", [0xFC2F, 0x0000, 0xFCC0, 0x0000]),
    ("\u{0641}\u{0645}", [0xFC30, 0x0000, 0xFCC1, 0x0000]),
    ("\u{0641}\u{0649}", [0xFC31, 0xFC7C, 0x0000, 0x0000]),
    ("\u{0641}\u{064A}", [0xFC32, 0xFC7D, 0x0000, 0x0000]),
    ("\u{0642}\u{062D}", [0xFC33, 0x0000, 0xFCC2, 0x0000]),
    ("\u{0642}\u{0645}", [0xFC34, 0x0000, 0xFCC3, 0x0000]),
    ("\u{0642}\u{0649}", [0xFC35, 0xFC7E, 0x0000, 0x0000]),
    ("\u{0642}\u{064A}", [0xFC36, 0xFC7F, 0x0000, 0x0000]),
    ("\u{0643}\u{0627}", [0xFC37, 0xFC80, 0x0000, 0x0000]),
    ("\u{0643}\u{062C}", [0xFC38, 0x0000, 0xFCC4, 0x0000]),
    ("\u{0643}\u{062D}", [0xFC39, 0x0000, 0xFCC5, 0x0000]),
    ("\u{0643}\u{062E}", [0xFC3A, 0x0000, 0xFCC6, 0x0000]),
    ("\u{0643}\u{0644}", [0xFC3B, 0xFC81, 0xFCC7, 0xFCEB]),
    ("\u{0643}\u{0645}", [0xFC3C, 0xFC82, 0xFCC8, 0xFCEC]),
    ("\u{0643}\u{0649}", [0xFC3D, 0xFC83, 0x0000, 0x0000]),
    ("\u{0643}\u{064A}", [0xFC3E, 0xFC84, 0x0000, 0x0000]),
    ("\u{0644}\u{0622}", [0xFEF5, 0xFEF6, 0x0000, 0x0000]),
    ("\u{0644}\u{0623}", [0xFEF7, 0xFEF8, 0x0000, 0x0000]),
    ("\u{0644}\u{0625}", [0xFEF9, 0xFEFA, 0x0000, 0x0000]),
    ("\u{0644}\u{0627}", [0xFEFB, 0xFEFC, 0x0000, 0x0000]),
    ("\u{0644}\u{062C}", [0xFC3F, 0x0000, 0xFCC9, 0x0000]),
    ("\u{0644}\u{062D}", [0xFC40, 0x0000, 0xFCCA, 0x0000]),
    ("\u{0644}\u{062E}", [0xFC41, 0x0000, 0xFCCB, 0x0000]),
    ("\u{0644}\u{0645}", [0xFC42, 0xFC85, 0xFCCC, 0xFCED]),
    ("\u{0644}\u{0647}", [0x0000, 0x0000, 0xFCCD, 0x0000]),
    ("\u{0644}\u{0649}", [0xFC43, 0xFC86, 0x0000, 0x0000]),
    ("\u{0644}\u{064A}", [0xFC44, 0xFC87, 0x0000, 0x0000]),
    ("\u{0645}\u{0627}", [0x0000, 0xFC88, 0x0000, 0x0000]),
    ("\u{0645}\u{062C}", [0xFC45, 0x0000, 0xFCCE, 0x0000]),
    ("\u{0645}\u{062D}", [0xFC46, 0x0000, 0xFCCF, 0x0000]),
    ("\u{0645}\u{062E}", [0xFC47, 0x0000, 0xFCD0, 0x0000]),
    ("\u{0645}\u{0645}", [0xFC48, 0xFC89, 0xFCD1, 0x0000]),
    ("\u{0645}\u{0649}", [0xFC49, 0x0000, 0x0000, 0x0000]),
    ("\u{0645}\u{064A}", [0xFC4A, 0x0000, 0x0000, 0x0000]),
    ("\u{0646}\u{062C}", [0xFC4B, 0x0000, 0xFCD2, 0x0000]),
    ("\u{0646}\u{062D}", [0xFC4C, 0x0000, 0xFCD3, 0x0000]),
    ("\u{0646}\u{062E}", [0xFC4D, 0x0000, 0xFCD4, 0x0000]),
    ("\u{0646}\u{0631}", [0x0000, 0xFC8A, 0x0000, 0x0000]),
    ("\u{0646}\u{0632}", [0x0000, 0xFC8B, 0x0000, 0x0000]),
    ("\u{0646}\u{0645}", [0xFC4E, 0xFC8C, 0xFCD5, 0xFCEE]),
    ("\u{0646}\u{0646}", [0x0000, 0xFC8D, 0x0000, 0x0000]),
    ("\u{0646}\u{0647}", [0x0000, 0x0000, 0xFCD6, 0xFCEF]),
    ("\u{0646}\u{0649}", [0xFC4F, 0xFC8E, 0x0000, 0x0000]),
    ("\u{0646}\u{064A}", [0xFC50, 0xFC8F, 0x0000, 0x0000]),
    ("\u{0647}\u{062C}", [0xFC51, 0x0000, 0xFCD7, 0x0000]),
    ("\u{0647}\u{0645}", [0xFC52, 0x0000, 0xFCD8, 0x0000]),
    ("\u{0647}\u{0649}", [0xFC53, 0x0000, 0x0000, 0x0000]),
    ("\u{0647}\u{064A}", [0xFC54, 0x0000, 0x0000, 0x0000]),
    ("\u{064A}\u{062C}", [0xFC55, 0x0000, 0xFCDA, 0x0000]),
    ("\u{064A}\u{062D}", [0xFC56, 0x0000, 0xFCDB, 0x0000]),
    ("\u{064A}\u{062E}", [0xFC57, 0x0000, 0xFCDC, 0x0000]),
    ("\u{064A}\u{0631}", [0x0000, 0xFC91, 0x0000, 0x0000]),
    ("\u{064A}\u{0632}", [0x0000, 0xFC92, 0x0000, 0x0000]),
    ("\u{064A}\u{0645}", [0xFC58, 0xFC93, 0xFCDD, 0xFCF0]),
    ("\u{064A}\u{0646}", [0x0000, 0xFC94, 0x0000, 0x0000]),
    ("\u{064A}\u{0647}", [0x0000, 0x0000, 0xFCDE, 0xFCF1]),
    ("\u{064A}\u{0649}", [0xFC59, 0xFC95, 0x0000, 0x0000]),
    ("\u{064A}\u{064A}", [0xFC5A, 0xFC96, 0x0000, 0x0000]),
];

const ISOLATED: usize = 0;
const FINAL: usize = 1;
const INITIAL: usize = 2;
const MEDIAL: usize = 3;
const LAM: char = '\u{0644}';

fn forms_of(c: char) -> Option<&'static [u32; 4]> {
    FORMS.binary_search_by_key(&c, |(base, _)| *base).ok().map(|i| &FORMS[i].1)
}

/// `text` with its Arabic letters in their joined presentation forms, and
/// for each character of the result the byte range of `text` it stands for
/// (two letters for a lam-alef ligature). A letter whose form Unicode does
/// not encode stays as it is.
pub fn presentation_forms(text: &str) -> (String, Vec<Range<usize>>) {
    forms(text, &|_| true, false)
}

/// [`presentation_forms`] for a font that has the characters `covers`
/// says, as a subset made from shaped text has them:
///
/// - where it lacks a letter's isolated form but has the letter, the letter
///   stays, its own glyph being that form;
/// - where it has one of Unicode's optional ligatures of the letters (yeh
///   and noon, U+FC94, …), that ligature: the producer's shaper drew it, so
///   the subset has it and not the separate forms.
pub fn presentation_forms_in(text: &str, covers: &dyn Fn(char) -> bool) -> (String, Vec<Range<usize>>) {
    forms(text, covers, true)
}

fn forms(text: &str, covers: &dyn Fn(char) -> bool, optional_ligatures: bool) -> (String, Vec<Range<usize>>) {
    let joining = CodePointMapData::<JoiningType>::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let kind = |i: usize| joining.get(chars[i].1);
    let transparent = |i: usize| kind(i) == JoiningType::Transparent;
    // Joins the character after it / the one before it.
    let joins_on_left = |t: JoiningType| {
        matches!(t, JoiningType::DualJoining | JoiningType::LeftJoining | JoiningType::JoinCausing)
    };
    let joins_on_right = |t: JoiningType| {
        matches!(t, JoiningType::DualJoining | JoiningType::RightJoining | JoiningType::JoinCausing)
    };
    let previous = |i: usize| (0..i).rev().find(|&j| !transparent(j));
    let next = |i: usize| (i + 1..chars.len()).find(|&j| !transparent(j));
    let connects_before =
        |i: usize| previous(i).is_some_and(|p| joins_on_left(kind(p)) && joins_on_right(kind(i)));
    let connects_after =
        |i: usize| next(i).is_some_and(|n| joins_on_left(kind(i)) && joins_on_right(kind(n)));

    let mut out = String::with_capacity(text.len());
    let mut sources = Vec::with_capacity(chars.len());
    let mut i = 0;
    'letters: while i < chars.len() {
        let (at, c) = chars[i];
        let end = |j: usize| chars.get(j + 1).map_or(text.len(), |&(next_at, _)| next_at);
        // Longest first: the ligatures a font made from shaped text has.
        if optional_ligatures && forms_of(c).is_some() {
            for (letters, ligature) in LIGATURES {
                let n = letters.chars().count();
                if i + n > chars.len() || !chars[i..i + n].iter().map(|&(_, l)| l).eq(letters.chars()) {
                    continue;
                }
                let last = i + n - 1;
                let index = match (connects_before(i), connects_after(last)) {
                    (true, true) => MEDIAL,
                    (true, false) => FINAL,
                    (false, true) => INITIAL,
                    (false, false) => ISOLATED,
                };
                if let Some(form) = char::from_u32(ligature[index]).filter(|f| *f != '\0' && covers(*f)) {
                    out.push(form);
                    sources.push(at..end(last));
                    i += n;
                    continue 'letters;
                }
            }
        }
        // Lam then alef, side by side: one ligature.
        if c == LAM
            && let Some(&(_, alef)) = chars.get(i + 1)
            && let Some((_, ligature)) = LAM_ALEF.iter().find(|(a, _)| *a == alef)
        {
            let form = ligature[usize::from(connects_before(i))];
            if let Some(form) = char::from_u32(form) {
                out.push(form);
                sources.push(at..end(i + 1));
                i += 2;
                continue;
            }
        }
        let form = forms_of(c).and_then(|forms| {
            let index = match (connects_before(i), connects_after(i)) {
                (true, true) => MEDIAL,
                (true, false) => FINAL,
                (false, true) => INITIAL,
                (false, false) => ISOLATED,
            };
            let form = char::from_u32(forms[index]).filter(|f| *f != '\0')?;
            (index != ISOLATED || covers(form) || !covers(c)).then_some(form)
        });
        out.push(form.unwrap_or(c));
        sources.push(at..end(i));
        i += 1;
    }
    (out, sources)
}

/// `text` with presentation forms folded back to the letters they show
/// (compatibility decomposition), for `/ToUnicode` and `/ActualText`: a
/// reader copying or searching the text gets the letters, in their order.
pub fn logical_text(text: &str) -> String {
    let presentation = |c: char| matches!(c, '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}');
    if !text.chars().any(presentation) {
        return text.to_string();
    }
    text.chars().fold(String::with_capacity(text.len()), |mut out, c| {
        if presentation(c) {
            out.extend(c.to_string().nfkc());
        } else {
            out.push(c);
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(text: &str) -> Vec<u32> {
        presentation_forms(text).0.chars().map(u32::from).collect()
    }

    #[test]
    fn letters_take_their_joined_forms() {
        // Beh alone, then two, then three in a row.
        assert_eq!(codes("ب"), vec![0xFE8F]);
        assert_eq!(codes("بب"), vec![0xFE91, 0xFE90]);
        assert_eq!(codes("ببب"), vec![0xFE91, 0xFE92, 0xFE90]);
        // Alef joins only to its right: the beh after it starts again.
        assert_eq!(codes("اب"), vec![0xFE8D, 0xFE8F]);
        // A space breaks joining; Latin is left alone.
        assert_eq!(codes("ب ب A"), vec![0xFE8F, 0x20, 0xFE8F, 0x20, 0x41]);
    }

    #[test]
    fn marks_are_skipped_for_joining_and_kept() {
        // Beh, fatha, beh: still initial and final around the mark.
        let (text, sources) = presentation_forms("بَب");
        assert_eq!(text.chars().map(u32::from).collect::<Vec<_>>(), vec![0xFE91, 0x064E, 0xFE90]);
        assert_eq!(sources.len(), 3);
    }

    #[test]
    fn lam_alef_becomes_one_ligature_standing_for_both_letters() {
        assert_eq!(codes("لا"), vec![0xFEFB]);
        // After a joining letter: its final form.
        assert_eq!(codes("بلا"), vec![0xFE91, 0xFEFC]);
        let (_, sources) = presentation_forms("سلام");
        // Seen, lam-alef (two letters), meem.
        assert_eq!(sources, vec![0..2, 2..6, 6..8]);
    }

    #[test]
    fn a_font_with_the_letter_but_not_its_isolated_form_keeps_the_letter() {
        let lacks_isolated = |c: char| c != '\u{FE8F}';
        let (text, _) = presentation_forms_in("ب بب", &lacks_isolated);
        assert_eq!(text.chars().map(u32::from).collect::<Vec<_>>(), vec![0x0628, 0x20, 0xFE91, 0xFE90]);
    }

    #[test]
    fn a_ligature_the_font_has_replaces_the_letters_it_joins() {
        // "فين" with yeh and noon as one final ligature, as Chrome's subset has it.
        let has = |c: char| matches!(c, '\u{FED3}' | '\u{FC94}');
        let (text, sources) = presentation_forms_in("فين", &has);
        assert_eq!(text.chars().map(u32::from).collect::<Vec<_>>(), vec![0xFED3, 0xFC94]);
        assert_eq!(sources, vec![0..2, 2..6]);
        assert_eq!(logical_text(&text), "فين");
        // Without the font's word, no optional ligature.
        assert_eq!(codes("فين"), vec![0xFED3, 0xFEF4, 0xFEE6]);
    }

    #[test]
    fn written_text_reads_back_as_letters() {
        let (forms, _) = presentation_forms("سلام عليكم");
        assert_ne!(forms, "سلام عليكم");
        assert_eq!(logical_text(&forms), "سلام عليكم");
        // Other text is untouched.
        assert_eq!(logical_text("ﬁ 2²"), "ﬁ 2²");
    }
}
