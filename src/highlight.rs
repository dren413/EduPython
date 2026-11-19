//! Python syntax highlighting for the code editor.

use eframe::egui::{
    Color32, FontId,
    text::{ByteIndex, LayoutJob, LayoutSection, TextFormat},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tok {
    Text,
    Keyword,
    Builtin,
    Str,
    Number,
    Comment,
    DefName,
    Decorator,
    SelfKw,
}

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
    "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "match",
    "case",
];

const BUILTINS: &[&str] = &[
    "abs", "all", "any", "bool", "chr", "dict", "enumerate", "filter", "float", "format", "input", "int",
    "isinstance", "len", "list", "map", "max", "min", "open", "ord", "print", "range", "reversed", "round",
    "set", "sorted", "str", "sum", "super", "tuple", "type", "zip", "Exception", "ValueError",
    "TypeError", "KeyError", "IndexError", "ZeroDivisionError",
];

struct Palette {
    text: Color32,
    keyword: Color32,
    builtin: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    def_name: Color32,
    decorator: Color32,
    self_kw: Color32,
}

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            text: Color32::from_rgb(0xAB, 0xB2, 0xBF),
            keyword: Color32::from_rgb(0xC6, 0x78, 0xDD),
            builtin: Color32::from_rgb(0x56, 0xB6, 0xC2),
            string: Color32::from_rgb(0x98, 0xC3, 0x79),
            number: Color32::from_rgb(0xD1, 0x9A, 0x66),
            comment: Color32::from_rgb(0x7F, 0x84, 0x8E),
            def_name: Color32::from_rgb(0x61, 0xAF, 0xEF),
            decorator: Color32::from_rgb(0xE5, 0xC0, 0x7B),
            self_kw: Color32::from_rgb(0xE0, 0x6C, 0x75),
        }
    } else {
        Palette {
            text: Color32::from_rgb(0x24, 0x29, 0x2E),
            keyword: Color32::from_rgb(0xA6, 0x26, 0xA4),
            builtin: Color32::from_rgb(0x0E, 0x6E, 0x8C),
            string: Color32::from_rgb(0x2E, 0x7D, 0x32),
            number: Color32::from_rgb(0xB0, 0x55, 0x00),
            comment: Color32::from_rgb(0x6A, 0x73, 0x7D),
            def_name: Color32::from_rgb(0x00, 0x5C, 0xC5),
            decorator: Color32::from_rgb(0x8A, 0x6D, 0x00),
            self_kw: Color32::from_rgb(0xC6, 0x28, 0x28),
        }
    }
}

/// Tokenize the whole text into (byte range, token) spans.
fn tokenize(text: &str) -> Vec<(usize, usize, Tok)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<(usize, usize, Tok)> = Vec::new();
    let mut i = 0;
    let mut prev_word: &str = "";

    let push = |out: &mut Vec<(usize, usize, Tok)>, s: usize, e: usize, t: Tok| {
        if let Some(last) = out.last_mut() {
            if last.2 == t && last.1 == s {
                last.1 = e;
                return;
            }
        }
        out.push((s, e, t));
    };

    while i < n {
        let c = b[i];
        if c == b'#' {
            let s = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            push(&mut out, s, i, Tok::Comment);
            continue;
        }
        if c == b'"' || c == b'\'' {
            let s = i;
            i = skip_string(b, i);
            push(&mut out, s, i, Tok::Str);
            prev_word = "";
            continue;
        }
        if c == b'@' && (i == 0 || line_is_blank_before(b, i)) {
            let s = i;
            i += 1;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            push(&mut out, s, i, Tok::Decorator);
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && i + 1 < n && b[i + 1].is_ascii_digit()) {
            let prev_ident = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
            if !prev_ident {
                let s = i;
                while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                    i += 1;
                }
                push(&mut out, s, i, Tok::Number);
                continue;
            }
        }
        if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            let s = i;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
                i += 1;
            }
            // string prefix: f"…", rb'…'
            if i < n && (b[i] == b'"' || b[i] == b'\'') && i - s <= 2 {
                let prefix = &text[s..i];
                if prefix.chars().all(|ch| "rRbBfFuU".contains(ch)) {
                    i = skip_string(b, i);
                    push(&mut out, s, i, Tok::Str);
                    prev_word = "";
                    continue;
                }
            }
            let word = &text[s..i];
            let tok = if prev_word == "def" || prev_word == "class" {
                Tok::DefName
            } else if KEYWORDS.contains(&word) {
                Tok::Keyword
            } else if word == "self" || word == "cls" {
                Tok::SelfKw
            } else if BUILTINS.contains(&word) && !(s > 0 && b[s - 1] == b'.') {
                Tok::Builtin
            } else {
                Tok::Text
            };
            push(&mut out, s, i, tok);
            prev_word = word;
            continue;
        }
        // any other char (operators, whitespace, multibyte handled above)
        let s = i;
        i += 1;
        while i < n && !b[i].is_ascii() && b[i] < 0xC0 {
            i += 1; // stay on a char boundary
        }
        if !c.is_ascii_whitespace() {
            prev_word = "";
        }
        push(&mut out, s, i, Tok::Text);
    }
    out
}

fn line_is_blank_before(b: &[u8], i: usize) -> bool {
    let mut j = i;
    while j > 0 {
        j -= 1;
        match b[j] {
            b'\n' => return true,
            b' ' | b'\t' => continue,
            _ => return false,
        }
    }
    true
}

/// Returns the index just after the string literal that starts at `i`.
fn skip_string(b: &[u8], mut i: usize) -> usize {
    let n = b.len();
    let q = b[i];
    let triple = i + 2 < n && b[i + 1] == q && b[i + 2] == q;
    i += if triple { 3 } else { 1 };
    while i < n {
        let c = b[i];
        if c == b'\\' {
            i += 2;
            continue;
        }
        if !triple && c == b'\n' {
            return i;
        }
        if c == q {
            if !triple {
                return i + 1;
            }
            if i + 2 < n && b[i + 1] == q && b[i + 2] == q {
                return i + 3;
            }
        }
        i += 1;
    }
    n.min(b.len())
}

/// Build a highlighted, non-wrapping layout job for the editor.
pub fn highlight(text: &str, font: FontId, dark: bool) -> LayoutJob {
    let p = palette(dark);
    let mut job = LayoutJob { text: text.to_owned(), ..Default::default() };
    for (s, e, tok) in tokenize(text) {
        let mut format = TextFormat::simple(font.clone(), p.text);
        format.color = match tok {
            Tok::Text => p.text,
            Tok::Keyword => p.keyword,
            Tok::Builtin => p.builtin,
            Tok::Str => p.string,
            Tok::Number => p.number,
            Tok::Comment => p.comment,
            Tok::DefName => p.def_name,
            Tok::Decorator => p.decorator,
            Tok::SelfKw => p.self_kw,
        };
        if tok == Tok::Comment {
            format.italics = true;
        }
        job.sections.push(LayoutSection { leading_space: 0.0, byte_range: ByteIndex::from(s)..ByteIndex::from(e.min(text.len())), format });
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_whole_text() {
        let src = "@dataclass\nclass A:\n    def f(self, x=1.5):  # hi\n        return f\"{x}\" + 'é' + \"\"\"\nmulti\"\"\"\n";
        let toks = tokenize(src);
        let mut pos = 0;
        for (s, e, _) in &toks {
            assert_eq!(*s, pos);
            assert!(src.is_char_boundary(*e));
            pos = *e;
        }
        assert_eq!(pos, src.len());
        assert!(toks.iter().any(|t| t.2 == Tok::DefName && &src[t.0..t.1] == "A"));
        assert!(toks.iter().any(|t| t.2 == Tok::Decorator));
    }
}
