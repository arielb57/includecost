//! Lexical scan of one C/C++ source file: significant bytes, include
//! directives, and include-guard / `#pragma once` detection.
//!
//! This is not a preprocessor. Macros are not expanded and conditions are not
//! evaluated; every `#include` is assumed to be possibly taken.

/// How a file protects itself against being expanded twice in one TU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    None,
    /// `#ifndef X` / `#define X` ... `#endif` enclosing every significant line.
    Macro,
    PragmaOnce,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncludeDirective {
    pub spelling: String,
    pub angled: bool,
    /// Inside an `#if`/`#ifdef`/`#ifndef` block other than the include guard.
    pub conditional: bool,
    /// 1-based physical line of the directive.
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannedFile {
    /// Bytes left after removing comments, trimming each logical line and
    /// dropping blank ones; every kept line also counts its newline.
    pub weight: u64,
    pub guard: Guard,
    pub includes: Vec<IncludeDirective>,
    /// Lines of `#include` forms that cannot be resolved without macro
    /// expansion, e.g. `#include CONFIG_HEADER`.
    pub computed_includes: Vec<u32>,
}

impl ScannedFile {
    /// True when a second inclusion in the same TU expands to nothing.
    pub fn once(&self) -> bool {
        self.guard != Guard::None
    }
}

struct Line {
    number: u32,
    text: Vec<u8>,
}

pub fn scan(src: &[u8]) -> ScannedFile {
    let lines = logical_lines(src);
    let weight = lines
        .iter()
        .map(|l| {
            let t = trim(&l.text);
            if t.is_empty() {
                0
            } else {
                t.len() as u64 + 1
            }
        })
        .sum();

    let significant: Vec<&Line> = lines.iter().filter(|l| !trim(&l.text).is_empty()).collect();
    let macro_guarded = detect_macro_guard(&significant);

    let mut includes = Vec::new();
    let mut computed_includes = Vec::new();
    let mut depth: u32 = 0;
    let mut pragma_once = false;
    let base = u32::from(macro_guarded);
    for line in &significant {
        let Some((name, rest)) = directive(&line.text) else {
            continue;
        };
        match name {
            b"if" | b"ifdef" | b"ifndef" => depth += 1,
            b"endif" => depth = depth.saturating_sub(1),
            b"pragma" if trim(rest) == b"once" => pragma_once = true,
            b"include" => match parse_header_name(rest) {
                Some((spelling, angled)) => includes.push(IncludeDirective {
                    spelling,
                    angled,
                    conditional: depth > base,
                    line: line.number,
                }),
                None => computed_includes.push(line.number),
            },
            _ => {}
        }
    }

    let guard = if macro_guarded {
        Guard::Macro
    } else if pragma_once {
        Guard::PragmaOnce
    } else {
        Guard::None
    };
    ScannedFile {
        weight,
        guard,
        includes,
        computed_includes,
    }
}

/// A guard only counts when nothing significant sits outside it: the first
/// significant line opens it, the second defines the macro, and the matching
/// `#endif` is the last significant line with no `#else`/`#elif` in between.
fn detect_macro_guard(sig: &[&Line]) -> bool {
    if sig.len() < 3 {
        return false;
    }
    let Some(guard_macro) = guard_condition(&sig[0].text) else {
        return false;
    };
    match directive(&sig[1].text) {
        Some((b"define", rest)) if identifier(trim(rest)) == guard_macro => {}
        _ => return false,
    }
    let mut depth: u32 = 1;
    for (i, line) in sig.iter().enumerate().skip(1) {
        let Some((name, _)) = directive(&line.text) else {
            continue;
        };
        match name {
            b"if" | b"ifdef" | b"ifndef" => depth += 1,
            b"else" | b"elif" | b"elifdef" | b"elifndef" if depth == 1 => return false,
            b"endif" => {
                depth -= 1;
                if depth == 0 {
                    return i == sig.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// Returns the macro name for `#ifndef X`, `#if !defined(X)` or `#if !defined X`.
fn guard_condition(text: &[u8]) -> Option<&[u8]> {
    let (name, rest) = directive(text)?;
    let rest = trim(rest);
    match name {
        b"ifndef" => {
            let id = identifier(rest);
            (!id.is_empty() && trim(&rest[id.len()..]).is_empty()).then_some(id)
        }
        b"if" => {
            let rest = trim(rest.strip_prefix(b"!")?);
            let rest = trim(rest.strip_prefix(b"defined")?);
            let (inner, tail) = match rest.strip_prefix(b"(") {
                Some(r) => {
                    let close = r.iter().position(|&c| c == b')')?;
                    (trim(&r[..close]), &r[close + 1..])
                }
                None => {
                    let id = identifier(rest);
                    (id, &rest[id.len()..])
                }
            };
            let id = identifier(inner);
            (!id.is_empty() && id.len() == inner.len() && trim(tail).is_empty()).then_some(id)
        }
        _ => None,
    }
}

/// Splits `# name rest` into `(name, rest)`.
fn directive(text: &[u8]) -> Option<(&[u8], &[u8])> {
    let t = trim(text);
    let after_hash = trim_start(t.strip_prefix(b"#")?);
    let name = identifier(after_hash);
    if name.is_empty() {
        return None;
    }
    Some((name, &after_hash[name.len()..]))
}

fn parse_header_name(rest: &[u8]) -> Option<(String, bool)> {
    let rest = trim(rest);
    let (close, angled) = match rest.first()? {
        b'"' => (b'"', false),
        b'<' => (b'>', true),
        _ => return None,
    };
    let body = &rest[1..];
    let end = body.iter().position(|&c| c == close)?;
    if end == 0 {
        return None;
    }
    Some((String::from_utf8_lossy(&body[..end]).into_owned(), angled))
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn identifier(s: &[u8]) -> &[u8] {
    if s.first().is_some_and(|c| c.is_ascii_digit()) {
        return &[];
    }
    let end = s.iter().position(|&c| !is_ident_byte(c)).unwrap_or(s.len());
    &s[..end]
}

fn trim_start(s: &[u8]) -> &[u8] {
    let start = s
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(s.len());
    &s[start..]
}

fn trim(s: &[u8]) -> &[u8] {
    let s = trim_start(s);
    let end = s
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    &s[..end]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Code,
    LineComment,
    BlockComment,
    Str,
    Char,
}

/// Translation phases 2-3, approximately: splices backslash-newlines and
/// replaces comments with a space while respecting string, character and raw
/// string literals. Returns logical lines tagged with their first physical line.
fn logical_lines(src: &[u8]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut cur = Vec::new();
    let mut line_no: u32 = 1;
    let mut start_line: u32 = 1;
    let mut state = State::Code;
    let mut in_number = false;
    let mut i = 0;
    let n = src.len();

    macro_rules! end_line {
        () => {{
            lines.push(Line {
                number: start_line,
                text: std::mem::take(&mut cur),
            });
            line_no += 1;
            start_line = line_no;
        }};
    }

    while i < n {
        let c = src[i];
        if c == b'\\' && state != State::BlockComment {
            let skip = if src.get(i + 1) == Some(&b'\n') {
                2
            } else if src.get(i + 1) == Some(&b'\r') && src.get(i + 2) == Some(&b'\n') {
                3
            } else {
                0
            };
            if skip > 0 {
                i += skip;
                line_no += 1;
                continue;
            }
        }
        if c == b'\n' {
            if matches!(state, State::LineComment | State::Str | State::Char) {
                state = State::Code;
            }
            in_number = false;
            end_line!();
            i += 1;
            continue;
        }
        match state {
            State::Code => {
                let next = src.get(i + 1).copied();
                if c == b'/' && next == Some(b'*') {
                    state = State::BlockComment;
                    cur.push(b' ');
                    in_number = false;
                    i += 2;
                    continue;
                }
                if c == b'/' && next == Some(b'/') {
                    state = State::LineComment;
                    i += 2;
                    continue;
                }
                if c == b'"' && raw_string_prefix(&cur) {
                    if let Some(consumed) = raw_string_len(&src[i..]) {
                        for &b in &src[i..i + consumed] {
                            if b == b'\n' {
                                end_line!();
                            } else {
                                cur.push(b);
                            }
                        }
                        i += consumed;
                        continue;
                    }
                }
                let prev = cur.last().copied();
                if c.is_ascii_digit() && !prev.is_some_and(is_ident_byte) {
                    in_number = true;
                } else if in_number && !(is_ident_byte(c) || c == b'.' || c == b'\'') {
                    in_number = false;
                }
                match c {
                    b'"' => state = State::Str,
                    // In a pp-number like 1'000'000 the quote is a digit separator.
                    b'\'' if !in_number => state = State::Char,
                    _ => {}
                }
                cur.push(c);
                i += 1;
            }
            State::LineComment => i += 1,
            State::BlockComment => {
                if c == b'*' && src.get(i + 1) == Some(&b'/') {
                    state = State::Code;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            State::Str | State::Char => {
                cur.push(c);
                if c == b'\\' {
                    if let Some(&esc) = src.get(i + 1) {
                        if esc != b'\n' {
                            cur.push(esc);
                            i += 2;
                            continue;
                        }
                    }
                } else if (state == State::Str && c == b'"') || (state == State::Char && c == b'\'')
                {
                    state = State::Code;
                }
                i += 1;
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(Line {
            number: start_line,
            text: cur,
        });
    }
    lines
}

/// `R`, `LR`, `uR`, `UR` or `u8R` immediately before the quote, not glued to
/// a longer identifier.
fn raw_string_prefix(cur: &[u8]) -> bool {
    for prefix in [&b"u8R"[..], b"LR", b"uR", b"UR", b"R"] {
        if cur.ends_with(prefix) {
            let before = cur.len() - prefix.len();
            if before == 0 || !is_ident_byte(cur[before - 1]) {
                return true;
            }
        }
    }
    false
}

/// Length of `"delim( ... )delim"` starting at the opening quote.
fn raw_string_len(s: &[u8]) -> Option<usize> {
    let open = s.iter().take(18).position(|&c| c == b'(')?;
    let delim = &s[1..open];
    if delim
        .iter()
        .any(|&c| c == b' ' || c == b'\\' || c == b')' || c == b'\n')
    {
        return None;
    }
    let mut closing = Vec::with_capacity(delim.len() + 2);
    closing.push(b')');
    closing.extend_from_slice(delim);
    closing.push(b'"');
    let body = &s[open + 1..];
    let pos = body
        .windows(closing.len())
        .position(|w| w == closing.as_slice())?;
    Some(open + 1 + pos + closing.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_ignores_comments_and_blank_lines() {
        let src = b"// header comment\n\nint a;   \n/* block\n comment */\n  int b; // trailing\n";
        let f = scan(src);
        // "int a;" + "int b;" plus a newline each.
        assert_eq!(f.weight, 14);
    }

    #[test]
    fn comment_markers_inside_literals_are_code() {
        let f = scan(b"const char* s = \"// not a comment /*\";\nchar c = '/';\n");
        assert_eq!(
            f.weight,
            ("const char* s = \"// not a comment /*\";".len() + 1 + "char c = '/';".len() + 1)
                as u64
        );
    }

    #[test]
    fn raw_strings_and_digit_separators() {
        let src = b"auto r = R\"x(/* keep */ \" // keep)x\";\nint big = 1'000'000; // gone\n";
        let f = scan(src);
        let expected =
            "auto r = R\"x(/* keep */ \" // keep)x\";".len() + 1 + "int big = 1'000'000;".len() + 1;
        assert_eq!(f.weight, expected as u64);
    }

    #[test]
    fn line_splices_join_directives() {
        let f = scan(b"#include \\\n  \"spliced.h\"\n#  include <sys/x.h> // c\n#include CONFIG\n");
        assert_eq!(f.includes.len(), 2);
        assert_eq!(f.includes[0].spelling, "spliced.h");
        assert_eq!(f.includes[0].line, 1);
        assert!(!f.includes[0].angled);
        assert_eq!(f.includes[1].spelling, "sys/x.h");
        assert_eq!(f.includes[1].line, 3);
        assert!(f.includes[1].angled);
        assert_eq!(f.computed_includes, vec![4]);
    }

    #[test]
    fn include_inside_comment_is_ignored() {
        let f = scan(b"/*\n#include \"a.h\"\n*/\n// #include \"b.h\"\n");
        assert!(f.includes.is_empty());
    }
}
