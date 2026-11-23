# Bundled fonts

Open-licence fonts (SIL Open Font License 1.1) used by the fixture generators,
the OCR evaluation set, and — from Milestone 3 — the text engine's bundled set
(docs/SPEC.md section 6.3). Each `*-OFL.txt` is that family's licence and must
ship alongside the font.

Downloaded from `github.com/google/fonts` at commit
`06bfe0d62a98375cb09c942fa18e68bd32cff057` (directory `ofl/`).

| File | Family | Style in spec terms | SHA-256 |
|---|---|---|---|
| `Amiri-Regular.ttf` | Amiri | Naskh (classical, serif-like) | `ab391c4147d054c48976e98322ad0eefe1427aa0e0502a12a4c75d80a70cfcd7` |
| `Amiri-Bold.ttf` | Amiri | | `cfccb794268e7d573d857e6d6a67f89cf8a053e8ffd85dfa0c8ec1bb36fc4827` |
| `NotoNaskhArabic-VF.ttf` | Noto Naskh Arabic (variable, wght) | Naskh | `67b5a525a661b607971fbd3f96a81b89d3a768e74534fca84f18ac97e6fab72f` |
| `NotoSansArabic-VF.ttf` | Noto Sans Arabic (variable, wdth + wght) | Kufi-style sans | `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e` |
| `IBMPlexSansArabic-Regular.ttf` | IBM Plex Sans Arabic | Kufi-style sans | `6f611412270a132bbac838da9259d4c68569b4175f3b3b8fa3fa36a30b56dab9` |
| `IBMPlexSansArabic-SemiBold.ttf` | IBM Plex Sans Arabic | | `597bd5502e5997be4414e4c9c88834b30ff3784250c84f20bce2b20e53ebd467` |
| `IBMPlexSans-VF.ttf` | IBM Plex Sans (variable, wdth + wght) | Latin sans | `3b031aa4216174205bd8471f88a49b91f093169e9e87bd5262242bc5967fe2e3` |

The UI itself loads IBM Plex Sans and IBM Plex Sans Arabic from the
`@fontsource` npm packages (also OFL-1.1), not from this folder.

"Plex" is a Reserved Font Name: the Plex files must not be modified and
redistributed under that name. Subsetting for PDF embedding is embedding, not
redistribution of a modified font, but re-check this when the Milestone 3
subsetter lands.

**Embedded now (ADR 0019).** Text boxes embed subsets of `IBMPlexSans-VF.ttf`
(instanced at weight 400) and `IBMPlexSansArabic-Regular.ttf` in the user's
PDFs, under their own names with a subset tag. Both files are compiled into the
engine. **The project owner decided (2026-11-23) to keep IBM Plex** for subsets
embedded under the fonts' own names (`PROGRESS.md`, owner item 14).

**Embedded from Milestone 3 (ADR 0021).** Text added to pages embeds subsets of
the same two Plex files (the Sans family) or of `Amiri-Regular.ttf` (the Naskh
family), also compiled into the engine.

**Embedded since ADRs 0023 and 0024.** A paragraph edited in place that needs a
substitute font embeds subsets of the closest of five faces: the three files
above, `NotoNaskhArabic-VF.ttf` (instanced at weight 400) and
`NotoSansArabic-VF.ttf` (weight 400, width 100). Its bold runs use the same
faces' bolds:
- the variable fonts instanced at weight 700;
- `Amiri-Bold.ttf`;
- `IBMPlexSansArabic-SemiBold.ttf`.

All of these are now compiled into the engine. Italic runs are drawn slanted
from the upright faces. The font programs are not changed.

**Italic runs use neutral names (owner item 14, decided 2026-11-23).** A name we
make carries no Reserved Font Name, and no family's own name. Each oblique is
embedded as `Mirsam` plus a style word (ADR 0024's amendment):
- IBM Plex Sans → `MirsamSans-Oblique` / `-BoldOblique`;
- IBM Plex Sans Arabic → `MirsamSansArabic-Oblique` / `-SemiBoldOblique`;
- Amiri → `MirsamNaskh-…`;
- Noto Naskh Arabic → `MirsamNaskhArabic-…`;
- Noto Sans Arabic → `MirsamKufi-…`.

Files saved earlier with `IBMPlexSans-Oblique`-style names are still recognised
when edited again.
