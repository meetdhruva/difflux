//! Turning bytes into comparable text.
//!
//! * [`decode`] detects binary content, BOMs, UTF-16 without a BOM and legacy
//!   encodings, and builds a line index in one `memchr` pass.
//! * [`normalize`] applies [`TextOptions`] to a single line for *matching only*;
//!   the displayed text is never changed.
//!
//! No I/O happens here: callers read the bytes and hand them over.

use std::borrow::Cow;
use std::ops::Range;

use difflux_core::{LineIdx, TextOptions, WhitespaceMode};
use schemars::JsonSchema;
use serde::Serialize;

/// How many leading bytes are inspected for binary detection (same window as git).
const SNIFF_LEN: usize = 8000;

/// Line terminators found in a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
    Mixed,
    /// Single line without any terminator.
    None,
}

impl LineEnding {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::Cr => "CR",
            Self::Mixed => "Mixed",
            Self::None => "None",
        }
    }
}

/// A line's location inside [`Document::text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    /// Byte offset of the first character.
    pub start: usize,
    /// Byte offset one past the last character, terminator excluded.
    pub end: usize,
    /// Terminator length in bytes: 0, 1 (LF or CR) or 2 (CRLF).
    pub terminator: u8,
}

/// Decoded text plus everything needed to show it faithfully.
#[derive(Debug, Clone)]
pub struct Document {
    text: String,
    lines: Vec<LineSpan>,
    /// Encoding name as registered in the WHATWG Encoding Standard, e.g. `UTF-8`.
    pub encoding: &'static str,
    pub has_bom: bool,
    pub line_ending: LineEnding,
    /// Some bytes were invalid in the detected encoding and were replaced with U+FFFD.
    pub lossy: bool,
    pub byte_len: u64,
}

/// Result of [`decode`].
#[derive(Debug, Clone)]
pub enum Content {
    Text(Document),
    Binary { byte_len: u64 },
}

impl Document {
    /// Builds a document from text that is already decoded (used for in-memory compares).
    #[must_use]
    pub fn from_text(text: String) -> Self {
        let byte_len = text.len() as u64;
        let (lines, line_ending) = index_lines(&text);
        Self { text, lines, encoding: "UTF-8", has_bom: false, line_ending, lossy: false, byte_len }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    pub fn spans(&self) -> &[LineSpan] {
        &self.lines
    }

    /// Line content without its terminator.
    ///
    /// # Panics
    /// If `idx` is out of range; indices come from this document's own line index.
    #[must_use]
    pub fn line(&self, idx: LineIdx) -> &str {
        let span = self.lines[idx.as_usize()];
        &self.text[span.start..span.end]
    }

    /// Line content including its terminator.
    #[must_use]
    pub fn line_with_terminator(&self, idx: LineIdx) -> &str {
        let span = self.lines[idx.as_usize()];
        &self.text[span.start..span.end + span.terminator as usize]
    }

    /// Iterates over all lines without terminators.
    pub fn lines(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        self.lines.iter().map(|s| &self.text[s.start..s.end])
    }

    /// The terminator of the given line (`""`, `"\n"`, `"\r\n"` or `"\r"`).
    #[must_use]
    pub fn terminator(&self, idx: LineIdx) -> &str {
        let span = self.lines[idx.as_usize()];
        &self.text[span.end..span.end + span.terminator as usize]
    }

    /// Whether the last line ends with a terminator.
    #[must_use]
    pub fn ends_with_newline(&self) -> bool {
        self.lines.last().is_none_or(|s| s.terminator > 0)
    }
}

/// Detects the encoding of `bytes`, decodes them and indexes lines.
#[must_use]
pub fn decode(bytes: &[u8]) -> Content {
    let byte_len = bytes.len() as u64;

    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, lossy) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        return Content::Text(build(text.into_owned(), encoding.name(), true, lossy, byte_len));
    }

    let sniff = &bytes[..bytes.len().min(SNIFF_LEN)];
    if memchr::memchr(0, sniff).is_some() {
        return match utf16_without_bom(sniff) {
            Some(encoding) => {
                let (text, lossy) = encoding.decode_without_bom_handling(bytes);
                Content::Text(build(text.into_owned(), encoding.name(), false, lossy, byte_len))
            }
            None => Content::Binary { byte_len },
        };
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return Content::Text(build(text.to_owned(), "UTF-8", false, false, byte_len));
    }

    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);
    let (text, lossy) = encoding.decode_without_bom_handling(bytes);
    Content::Text(build(text.into_owned(), encoding.name(), false, lossy, byte_len))
}

fn build(text: String, encoding: &'static str, has_bom: bool, lossy: bool, byte_len: u64) -> Document {
    let (lines, line_ending) = index_lines(&text);
    Document { text, lines, encoding, has_bom, line_ending, lossy, byte_len }
}

/// Recognizes UTF-16 text without a BOM: ASCII-heavy UTF-16 has NUL in every
/// other byte, which random binary data almost never does.
fn utf16_without_bom(sniff: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    if sniff.len() < 4 || !sniff.len().is_multiple_of(2) {
        return None;
    }
    let pairs = sniff.len() / 2;
    let (mut even, mut odd) = (0usize, 0usize);
    for pair in sniff.chunks_exact(2) {
        even += usize::from(pair[0] == 0);
        odd += usize::from(pair[1] == 0);
    }
    let mostly = |n: usize| n * 10 >= pairs * 4; // at least 40 %
    let rarely = |n: usize| n * 20 <= pairs; // at most 5 %
    if mostly(odd) && rarely(even) {
        Some(encoding_rs::UTF_16LE)
    } else if mostly(even) && rarely(odd) {
        Some(encoding_rs::UTF_16BE)
    } else {
        None
    }
}

/// Splits text into lines, recognizing LF, CRLF and lone CR terminators.
fn index_lines(text: &str) -> (Vec<LineSpan>, LineEnding) {
    let bytes = text.as_bytes();
    let mut lines = Vec::with_capacity(bytes.len() / 40 + 1);
    let (mut lf, mut crlf, mut cr) = (false, false, false);
    let mut start = 0;
    let mut search = 0;
    while let Some(pos) = memchr::memchr2(b'\n', b'\r', &bytes[search..]).map(|p| p + search) {
        let terminator = if bytes[pos] == b'\r' {
            if bytes.get(pos + 1) == Some(&b'\n') {
                crlf = true;
                2
            } else {
                cr = true;
                1
            }
        } else {
            lf = true;
            1
        };
        lines.push(LineSpan { start, end: pos, terminator });
        start = pos + terminator as usize;
        search = start;
    }
    if start < bytes.len() {
        lines.push(LineSpan { start, end: bytes.len(), terminator: 0 });
    }
    let ending = match (lf, crlf, cr) {
        (false, false, false) => LineEnding::None,
        (true, false, false) => LineEnding::Lf,
        (false, true, false) => LineEnding::CrLf,
        (false, false, true) => LineEnding::Cr,
        _ => LineEnding::Mixed,
    };
    (lines, ending)
}

/// Applies comparison rules to one line. Returns the input unchanged (borrowed)
/// when no rule applies, so the common case allocates nothing.
#[must_use]
pub fn normalize<'a>(line: &'a str, opts: &TextOptions) -> Cow<'a, str> {
    let line: Cow<'a, str> = match opts.whitespace {
        WhitespaceMode::Exact => Cow::Borrowed(line),
        WhitespaceMode::Trailing => Cow::Borrowed(line.trim_end()),
        WhitespaceMode::Change => {
            let trimmed = line.trim_end();
            if has_whitespace_run_or_tab(trimmed) { Cow::Owned(collapse_whitespace(trimmed)) } else { Cow::Borrowed(trimmed) }
        }
        WhitespaceMode::All => {
            if line.chars().any(char::is_whitespace) {
                Cow::Owned(line.chars().filter(|c| !c.is_whitespace()).collect())
            } else {
                Cow::Borrowed(line)
            }
        }
    };
    if opts.ignore_case && line.chars().any(char::is_uppercase) { Cow::Owned(line.to_lowercase()) } else { line }
}

fn has_whitespace_run_or_tab(s: &str) -> bool {
    let mut prev_ws = false;
    for c in s.chars() {
        let ws = c.is_whitespace();
        if ws && (prev_ws || c != ' ') {
            return true;
        }
        prev_ws = ws;
    }
    false
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(c);
            prev_ws = false;
        }
    }
    out
}

/// Splits a line into word, whitespace and punctuation tokens for inline diffs.
/// Returns byte ranges into `line`.
#[must_use]
pub fn word_tokens(line: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut tokens = Vec::new();
    let mut iter = line.char_indices().peekable();
    while let Some((start, c)) = iter.next() {
        let k = class(c);
        let mut end = start + c.len_utf8();
        if k != Class::Other {
            while let Some(&(i, next)) = iter.peek() {
                if class(next) != k {
                    break;
                }
                end = i + next.len_utf8();
                iter.next();
            }
        }
        tokens.push(start..end);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(content: Content) -> Document {
        match content {
            Content::Text(doc) => doc,
            Content::Binary { .. } => panic!("expected text"),
        }
    }

    #[test]
    fn indexes_mixed_line_endings() {
        let doc = text(decode(b"a\r\nb\nc\rd"));
        assert_eq!(doc.lines().collect::<Vec<_>>(), ["a", "b", "c", "d"]);
        assert_eq!(doc.line_ending, LineEnding::Mixed);
        assert_eq!(doc.terminator(LineIdx(0)), "\r\n");
        assert!(!doc.ends_with_newline());
    }

    #[test]
    fn trailing_newline_does_not_create_an_empty_line() {
        let doc = text(decode(b"a\nb\n"));
        assert_eq!(doc.line_count(), 2);
        assert!(doc.ends_with_newline());
        assert_eq!(doc.line_ending, LineEnding::Lf);
    }

    #[test]
    fn empty_input_has_no_lines() {
        let doc = text(decode(b""));
        assert_eq!(doc.line_count(), 0);
        assert_eq!(doc.line_ending, LineEnding::None);
    }

    #[test]
    fn detects_utf16_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for c in "hé\nyo".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        let doc = text(decode(&bytes));
        assert_eq!(doc.encoding, "UTF-16LE");
        assert!(doc.has_bom);
        assert_eq!(doc.lines().collect::<Vec<_>>(), ["hé", "yo"]);
    }

    #[test]
    fn detects_utf16_without_bom() {
        let bytes: Vec<u8> = "plain ascii text\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let doc = text(decode(&bytes));
        assert_eq!(doc.encoding, "UTF-16LE");
        assert_eq!(doc.line(LineIdx(0)), "plain ascii text");
    }

    #[test]
    fn nul_bytes_mean_binary() {
        assert!(matches!(decode(b"\x7fELF\x02\x01\x01\x00\x00\x00\x00"), Content::Binary { byte_len: 11 }));
    }

    #[test]
    fn falls_back_to_legacy_encoding() {
        // "café" in Windows-1252
        let doc = text(decode(b"caf\xe9 au lait, cr\xe8me br\xfbl\xe9e"));
        assert!(doc.text().starts_with("café"), "{}", doc.text());
        assert!(!doc.lossy);
    }

    #[test]
    fn normalize_respects_rules() {
        let mut o = TextOptions::default();
        assert!(matches!(normalize("  a  b ", &o), Cow::Borrowed("  a  b ")));
        o.whitespace = WhitespaceMode::Trailing;
        assert_eq!(normalize("  a  b ", &o), "  a  b");
        o.whitespace = WhitespaceMode::Change;
        assert_eq!(normalize("  a \t b ", &o), " a b");
        o.whitespace = WhitespaceMode::All;
        o.ignore_case = true;
        assert_eq!(normalize(" A b ", &o), "ab");
    }

    #[test]
    fn word_tokens_split_words_spaces_and_punctuation() {
        let line = "let x_1 = f(a,  b);";
        let toks: Vec<&str> = word_tokens(line).into_iter().map(|r| &line[r]).collect();
        assert_eq!(toks, ["let", " ", "x_1", " ", "=", " ", "f", "(", "a", ",", "  ", "b", ")", ";"]);
    }

    proptest::proptest! {
        #[test]
        fn lines_and_terminators_reconstruct_input(s in "[a-c\r\n ]{0,64}") {
            let doc = text(decode(s.as_bytes()));
            let rebuilt: String = (0..doc.line_count() as u32)
                .map(|i| doc.line_with_terminator(LineIdx(i)))
                .collect();
            proptest::prop_assert_eq!(rebuilt, s);
        }

        #[test]
        fn word_tokens_cover_the_line(s in "\\PC{0,40}") {
            let joined: String = word_tokens(&s).into_iter().map(|r| &s[r]).collect();
            proptest::prop_assert_eq!(joined, s);
        }
    }
}
