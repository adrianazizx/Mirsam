//! Redaction of crash report text (ADR 0061).
//!
//! A report may carry an error message and a stack, and nothing else written by
//! the user or taken from a document. Messages are where document text, file
//! names and paths could slip in (Rust's own panic messages quote the
//! offending character of a string; our errors name files), so a message goes
//! through three layers:
//!
//! 1. strings known to be private are replaced wherever they appear: the open
//!    documents' paths, file names and names without extension, the home
//!    folder and the user's account name ([`Scrubber`]);
//! 2. anything quoted is dropped: `'…'`, `"…"`, `` `…` ``, `“…”`, `«…»`;
//! 3. each remaining word that looks like a path, a file name, a web or mail
//!    address, a long opaque token, or contains letters outside ASCII (an
//!    Arabic word, an accented name) is replaced by a placeholder.
//!
//! What survives is the shape of the message ("index out of bounds: the len is
//! 3 but the index is 7"), which is what a developer needs. Stack frames are
//! code locations and keep their function names; their source paths are cut
//! down to the part inside the source tree.

/// Placeholder for quoted text.
pub const QUOTED: &str = "<quoted>";
/// Placeholder for a path or file name.
pub const PATH: &str = "<path>";
/// Placeholder for a word in a script other than ASCII, or a known private string.
pub const TEXT: &str = "<text>";
/// Placeholder for a web or mail address.
pub const ADDRESS: &str = "<address>";
/// Placeholder for a long opaque token (base64, hex, ids).
pub const DATA: &str = "<data>";

/// Longest message kept, in characters, before and after redaction.
const MAX_MESSAGE_CHARS: usize = 1_000;
/// A word longer than this, without `::` (a Rust path), is opaque data.
const MAX_WORD_CHARS: usize = 48;

/// Strings to remove from messages wherever they appear: what the app knows is
/// private (open documents' paths and names, the home folder, the account name).
#[derive(Debug, Clone, Default)]
pub struct Scrubber {
    /// Longest first, so a path is replaced before the file name inside it.
    secrets: Vec<String>,
}

impl Scrubber {
    /// A scrubber for these strings. Strings shorter than three characters are
    /// ignored: replacing "a" everywhere would destroy the message, and so short
    /// a name says nothing about anyone.
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        let mut secrets: Vec<String> =
            secrets.into_iter().filter(|secret| secret.trim().chars().count() >= 3).collect();
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        Self { secrets }
    }

    /// The private strings of a file path: the whole path, its file name, and
    /// the file name without its extension.
    pub fn path_secrets(path: &std::path::Path) -> Vec<String> {
        let mut secrets = vec![path.to_string_lossy().into_owned()];
        if let Some(name) = path.file_name() {
            secrets.push(name.to_string_lossy().into_owned());
        }
        if let Some(stem) = path.file_stem() {
            secrets.push(stem.to_string_lossy().into_owned());
        }
        secrets
    }

    /// The private strings of this machine's user: the home folder and the
    /// account name.
    pub fn user_secrets() -> Vec<String> {
        let mut secrets = Vec::new();
        for key in ["HOME", "USERPROFILE"] {
            if let Some(home) = std::env::var_os(key) {
                secrets.push(home.to_string_lossy().into_owned());
            }
        }
        for key in ["USER", "USERNAME", "LOGNAME"] {
            if let Some(name) = std::env::var_os(key) {
                secrets.push(name.to_string_lossy().into_owned());
            }
        }
        secrets
    }

    fn scrub(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for secret in &self.secrets {
            if text.contains(secret.as_str()) {
                text = replace_bounded(&text, secret);
            }
        }
        text
    }
}

/// Replaces `secret` with [`TEXT`] where it stands as a whole word (or more):
/// an account name "max" leaves "maximum" alone.
fn replace_bounded(text: &str, secret: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(found) = text[from..].find(secret) {
        let at = from + found;
        let end = at + secret.len();
        let before = text[..at].chars().next_back();
        let after = text[end..].chars().next();
        let bounded =
            before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric());
        out.push_str(&text[from..at]);
        out.push_str(if bounded { TEXT } else { secret });
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

/// An error message reduced to what a crash report may carry (see the module
/// documentation).
pub fn redact_message(message: &str, scrubber: &Scrubber) -> String {
    let message: String = message.chars().take(MAX_MESSAGE_CHARS * 4).collect();
    let scrubbed = scrubber.scrub(&message);
    let unquoted = drop_quoted(&scrubbed);
    let mut out = String::with_capacity(unquoted.len());
    let mut word = String::new();
    for c in unquoted.chars() {
        if c.is_whitespace() {
            push_word(&mut out, &word);
            word.clear();
            out.push(c);
        } else {
            word.push(c);
        }
    }
    push_word(&mut out, &word);
    truncate_chars(out.trim(), MAX_MESSAGE_CHARS)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max).collect();
    cut.push('…');
    cut
}

/// Appends `word`, or its placeholder, keeping punctuation around it.
fn push_word(out: &mut String, word: &str) {
    if word.is_empty() {
        return;
    }
    const EDGE: &[char] = &['(', ')', '[', ']', '{', '}', ',', ';', ':', '.', '!', '?'];
    let core = word.trim_matches(EDGE);
    // Placeholders already in the word (quoted text dropped, a private string
    // scrubbed) are not judged themselves; the rest of the word is.
    let probe = PLACEHOLDERS.iter().fold(core.to_owned(), |probe, p| probe.replace(p, ""));
    if probe.is_empty() {
        out.push_str(word);
        return;
    }
    let start = word.find(core).unwrap_or(0);
    let (lead, trail) = (&word[..start], &word[start + core.len()..]);
    out.push_str(lead);
    out.push_str(classify(&probe).unwrap_or(core));
    out.push_str(trail);
}

const PLACEHOLDERS: [&str; 5] = [QUOTED, PATH, TEXT, ADDRESS, DATA];

/// The placeholder for a word that must not appear in a report, if it is one.
fn classify(word: &str) -> Option<&'static str> {
    if word.chars().any(|c| !c.is_ascii() && (c.is_alphanumeric() || is_mark(c))) {
        return Some(TEXT);
    }
    if word.contains("://") || (word.contains('@') && word.contains('.')) {
        return Some(ADDRESS);
    }
    if word.contains('\\') || (word.contains('/') && !is_fraction(word)) || word.starts_with('~') {
        return Some(PATH);
    }
    if !word.contains("::") && looks_like_file_name(word) {
        return Some(PATH);
    }
    if !word.contains("::") && word.chars().count() > MAX_WORD_CHARS {
        return Some(DATA);
    }
    None
}

/// Combining marks (Arabic tashkeel and the like) are not alphanumeric, but
/// they are text.
fn is_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}')
}

/// `3/4`, `2026/11/23`: digits and slashes only.
fn is_fraction(word: &str) -> bool {
    word.chars().all(|c| c.is_ascii_digit() || c == '/')
}

/// `Contract.pdf`, `scan_001.JPG`: a name, a dot, and a short alphabetic
/// extension. A decimal number (`1.5`) or a version (`v2.12`) is not one.
fn looks_like_file_name(word: &str) -> bool {
    let Some((name, extension)) = word.rsplit_once('.') else { return false };
    !name.is_empty()
        && (2..=5).contains(&extension.len())
        && extension.chars().all(|c| c.is_ascii_alphabetic())
        && name.chars().any(|c| c.is_ascii_alphabetic())
}

/// Drops every quoted span, quotes included, for [`QUOTED`].
fn drop_quoted(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(close) = closing_quote(c, chars.get(i.wrapping_sub(1)).copied(), i == 0)
            && let Some(end) = find_close(&chars, i + 1, c, close)
        {
            out.push_str(QUOTED);
            i = end + 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The quote that closes `open`, if `open` opens a quotation here. A single
/// quote opens one only at the start of a word, so "can't" stays a word.
fn closing_quote(open: char, before: Option<char>, at_start: bool) -> Option<char> {
    match open {
        '"' => Some('"'),
        '`' => Some('`'),
        '\'' if at_start || before.is_none_or(|b| b.is_whitespace() || "([{=:,".contains(b)) => Some('\''),
        '“' => Some('”'),
        '‘' => Some('’'),
        '«' => Some('»'),
        '‹' => Some('›'),
        '„' => Some('“'),
        _ => None,
    }
}

/// Index of the quote closing a span opened just before `start`. Backslash
/// escapes (Rust's debug strings) do not close it.
fn find_close(chars: &[char], start: usize, open: char, close: char) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && open == '"' {
            i += 2;
            continue;
        }
        if c == close {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// A command's name as the UI sent it (`edit.bold`, `update_viewport ×3`).
/// Names come from a closed list, so anything else is dropped.
pub fn command_name(name: &str) -> String {
    let (id, count) = match name.split_once(" ×") {
        Some((id, count)) if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()) => {
            (id, Some(count))
        }
        _ => (name, None),
    };
    let valid = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    match (valid, count) {
        (true, Some(count)) => format!("{id} ×{count}"),
        (true, None) => id.to_owned(),
        (false, _) => DATA.to_owned(),
    }
}

/// A Rust source location (a panic's, a frame's) cut down to the part inside
/// a source tree: `crates/mirsam-core/src/x.rs`, `tauri-2.12.0/src/app.rs`,
/// `rustc/library/core/src/str/mod.rs`. Anything else keeps only its file name.
pub fn source_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    for marker in ["crates/mirsam-", "apps/desktop/src-tauri/", "fuzz/src/"] {
        if let Some(at) = path.find(marker) {
            return path[at..].to_owned();
        }
    }
    if let Some(at) = path.find("/library/") {
        return format!("rustc{}", &path[at..]);
    }
    // Cargo's registry and git checkouts: `<registry>/<crate>-<version>/src/…`.
    for marker in ["/registry/src/", "/git/checkouts/"] {
        if let Some(at) = path.find(marker) {
            let rest = &path[at + marker.len()..];
            // Skip the registry's own directory (`index.crates.io-<hash>`).
            if let Some((_, inside)) = rest.split_once('/') {
                return inside.to_owned();
            }
        }
    }
    if let Some(rest) = path.strip_prefix("./") {
        return rest.to_owned();
    }
    if !path.starts_with('/') && !path.contains(':') && !path.starts_with('~') {
        // Relative to the workspace already (`src/main.rs`).
        return path;
    }
    path.rsplit('/').next().unwrap_or_default().to_owned()
}

/// A function name from a backtrace, without the crate hashes the symbol
/// table carries (`std[d28d9deb04fda6f8]::panicking` → `std::panicking`).
pub fn function_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c == '[' {
            let rest = &name[at + 1..];
            if let Some(end) = rest.find(']')
                && end >= 8
                && rest[..end].chars().all(|h| h.is_ascii_hexdigit())
            {
                for _ in 0..=end {
                    chars.next();
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// A frame of a JavaScript stack (`fn@tauri://localhost/assets/index-ab12.js:1:2`),
/// with any address other than the app's own reduced to [`ADDRESS`]. The UI's
/// own scripts are served from `tauri://localhost`, `http(s)://tauri.localhost`
/// or, in development, `http://localhost:<port>`.
pub fn ui_frame(frame: &str, scrubber: &Scrubber) -> String {
    let frame = scrubber.scrub(frame.trim());
    let mut out = String::with_capacity(frame.len());
    for (i, part) in frame.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&ui_frame_part(part));
    }
    truncate_chars(&out, 300)
}

fn ui_frame_part(part: &str) -> String {
    let Some(scheme_end) = part.find("://") else {
        return if part.is_ascii() { part.to_owned() } else { TEXT.to_owned() };
    };
    // Keep what precedes the address (`fn@`, `(`).
    let start = part[..scheme_end].rfind(|c: char| !c.is_ascii_alphanumeric()).map_or(0, |at| at + 1);
    let (lead, address) = part.split_at(start);
    let own = ["tauri://localhost/", "http://tauri.localhost/", "https://tauri.localhost/"]
        .iter()
        .find_map(|origin| address.strip_prefix(origin))
        .or_else(|| {
            let rest = address.strip_prefix("http://localhost:")?;
            rest.split_once('/').map(|(_, path)| path)
        });
    match own {
        Some(path) => {
            format!("{lead}{}", path.split(['?', '#']).next().unwrap_or_default()) + &position_suffix(path)
        }
        None => format!("{lead}{ADDRESS}"),
    }
}

/// `:12:34` (and a closing parenthesis) after a script's query string, which
/// [`ui_frame_part`] dropped with the query.
fn position_suffix(path: &str) -> String {
    match path.find(['?', '#']) {
        None => String::new(),
        Some(at) => {
            let rest = &path[at..];
            let tail: String = rest
                .char_indices()
                .find(|(_, c)| *c == ':')
                .map(|(i, _)| rest[i..].to_owned())
                .unwrap_or_default();
            if tail.chars().all(|c| c.is_ascii_digit() || c == ':' || c == ')') {
                tail
            } else {
                String::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(message: &str) -> String {
        redact_message(message, &Scrubber::default())
    }

    #[test]
    fn keeps_the_shape_of_ordinary_panic_messages() {
        assert_eq!(
            redact("index out of bounds: the len is 3 but the index is 7"),
            "index out of bounds: the len is 3 but the index is 7"
        );
        assert_eq!(
            redact("called `Option::unwrap()` on a `None` value"),
            "called <quoted> on a <quoted> value"
        );
        assert_eq!(redact("attempt to subtract with overflow"), "attempt to subtract with overflow");
        assert_eq!(redact("can't render page 4 of 12"), "can't render page 4 of 12");
    }

    #[test]
    fn drops_quoted_document_text() {
        // Rust's own message for slicing a string inside a character.
        let message = "byte index 1 is not a char boundary; it is inside 'ب' (bytes 0..2 of `بسم الله`)";
        let redacted = redact(message);
        assert_eq!(
            redacted,
            "byte index 1 is not a char boundary; it is inside <quoted> (bytes 0..2 of <quoted>)"
        );
        assert_eq!(redact(r#"Err(Typeset("no glyph for \"x\" in font"))"#), "Err(Typeset(<quoted>))");
        assert_eq!(redact("value “Confidential” and «سري»"), "value <quoted> and <quoted>");
    }

    #[test]
    fn replaces_words_that_could_name_a_file_or_a_person() {
        assert_eq!(redact("could not open /Users/someone/Contracts/lease.pdf"), "could not open <path>");
        assert_eq!(redact(r"failed: C:\Users\Some One\x.pdf"), "failed: <path> <path>");
        assert_eq!(redact("cannot read lease_2026.PDF now"), "cannot read <path> now");
        assert_eq!(redact("الإيجار not found"), "<text> not found");
        assert_eq!(redact("naïve résumé"), "<text> <text>");
        assert_eq!(
            redact("mail someone@example.com or see https://example.com/x"),
            "mail <address> or see <address>"
        );
        assert_eq!(redact("token QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9w"), "token <data>");
        // Numbers, ratios and Rust paths are kept.
        assert_eq!(
            redact("ratio 3/4 at 1.5x in mirsam_core::text::layout"),
            "ratio 3/4 at 1.5x in mirsam_core::text::layout"
        );
    }

    #[test]
    fn removes_known_private_strings_anywhere() {
        let scrubber = Scrubber::new(
            Scrubber::path_secrets(std::path::Path::new("/Users/someone/Docs/Board minutes.pdf"))
                .into_iter()
                .chain(["someone".to_string()]),
        );
        let message = "layout failed for Board minutes (page 2); user someone";
        assert_eq!(redact_message(message, &scrubber), "layout failed for <text> (page 2); user <text>");
        // The whole path goes before its pieces.
        assert_eq!(
            redact_message("open /Users/someone/Docs/Board minutes.pdf failed", &scrubber),
            "open <text> failed"
        );
    }

    #[test]
    fn replaces_private_strings_only_as_whole_words() {
        let scrubber = Scrubber::new(["max".to_string(), "ab".to_string()]);
        // "ab" is too short to mean anything; "max" goes only where it stands alone.
        assert_eq!(
            redact_message("maximum reached by max (ab)", &scrubber),
            "maximum reached by <text> (ab)"
        );
    }

    #[test]
    fn bounds_the_message() {
        let long = "word ".repeat(1_000);
        assert!(redact(&long).chars().count() <= MAX_MESSAGE_CHARS + 1);
    }

    #[test]
    fn cuts_source_paths_down_to_the_source_tree() {
        assert_eq!(
            source_path("crates/mirsam-core/src/text/layout.rs"),
            "crates/mirsam-core/src/text/layout.rs"
        );
        assert_eq!(
            source_path("/Users/someone/work/Mirsam/crates/mirsam-core/src/lib.rs"),
            "crates/mirsam-core/src/lib.rs"
        );
        assert_eq!(
            source_path(
                "/Users/someone/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tauri-2.12.0/src/app.rs"
            ),
            "tauri-2.12.0/src/app.rs"
        );
        assert_eq!(
            source_path("/rustc/abc123/library/core/src/str/mod.rs"),
            "rustc/library/core/src/str/mod.rs"
        );
        assert_eq!(
            source_path(r"C:\Users\x\.cargo\registry\src\index-1\ureq-3.5.2\src\lib.rs"),
            "ureq-3.5.2/src/lib.rs"
        );
        assert_eq!(source_path("/Users/someone/secret/build.rs"), "build.rs");
    }

    #[test]
    fn drops_crate_hashes_from_function_names() {
        assert_eq!(
            function_name("<std[d28d9deb04fda6f8]::backtrace::Backtrace>::create"),
            "<std::backtrace::Backtrace>::create"
        );
        assert_eq!(
            function_name("core[e78f7293afa7e62d]::str::slice_error_fail"),
            "core::str::slice_error_fail"
        );
        assert_eq!(function_name("foo::<[u8; 4]>::bar"), "foo::<[u8; 4]>::bar");
    }

    #[test]
    fn keeps_only_the_apps_own_script_addresses_in_ui_stacks() {
        let scrubber = Scrubber::default();
        assert_eq!(
            ui_frame("render@tauri://localhost/assets/index-AbC12.js:1:2345", &scrubber),
            "render@assets/index-AbC12.js:1:2345"
        );
        assert_eq!(
            ui_frame("at onClick (http://localhost:1420/src/App.tsx?t=1700:12:3)", &scrubber),
            "at onClick (src/App.tsx:12:3)"
        );
        assert_eq!(ui_frame("x@https://example.com/a.js:1:1", &scrubber), "x@<address>");
    }
}
