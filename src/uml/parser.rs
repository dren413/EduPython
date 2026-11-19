//! A small, forgiving Python "parser" that extracts classes for the UML view.
//!
//! It does not need a Python interpreter and never fails: code that is being
//! typed (and is therefore often broken) still produces a useful diagram.
//!
//! What it understands:
//! * `class Name(Base1, Base2):` (also nested in `if` blocks, decorators, …)
//! * methods with parameters, annotations, defaults and `-> return` types
//! * instance attributes from `self.x = …`, `self.x: T = …`, `self.a, self.b = …`
//! * class attributes (`count = 0`) and `@dataclass` fields (`x: int`)
//! * simple type inference: literals, constructor calls, parameters, locals,
//!   arithmetic, comparisons, common `pygame` / `random` / `math` calls.

use std::collections::HashMap;

use super::model::{Attribute, ClassInfo, Method, Param};

// ─────────────────────────────────────────────────────────────────────────────
// Logical lines
// ─────────────────────────────────────────────────────────────────────────────

/// A logical Python line: physical lines joined across brackets and `\`,
/// comments removed and string contents replaced by an empty placeholder
/// (`"x, y"` → `""`). Short identifier-like strings are kept (`"Ball"`) so
/// forward-reference annotations still work.
#[derive(Debug, Clone)]
struct LogicalLine {
    indent: usize,
    text: String,
    line: usize,
}

fn is_ident_like(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

fn logical_lines(src: &str) -> Vec<LogicalLine> {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;

    while i < n {
        let start_line = line;
        let mut indent = 0;
        while i < n && matches!(chars[i], ' ' | '\t' | '\x0c') {
            indent += if chars[i] == '\t' { 8 - indent % 8 } else { 1 };
            i += 1;
        }

        let mut text = String::new();
        let mut depth: i32 = 0;
        while i < n {
            let c = chars[i];
            match c {
                '\n' => {
                    line += 1;
                    i += 1;
                    if depth > 0 {
                        text.push(' ');
                        continue;
                    }
                    break;
                }
                '\r' => i += 1,
                '\\' if i + 1 < n && (chars[i + 1] == '\n' || chars[i + 1] == '\r') => {
                    i += 1;
                    if chars[i] == '\r' {
                        i += 1;
                    }
                    if i < n && chars[i] == '\n' {
                        i += 1;
                    }
                    line += 1;
                    text.push(' ');
                }
                '#' => {
                    while i < n && chars[i] != '\n' {
                        i += 1;
                    }
                }
                '"' | '\'' => {
                    let triple = i + 2 < n && chars[i + 1] == c && chars[i + 2] == c;
                    let q = if triple { 3 } else { 1 };
                    i += q;
                    let content_start = i;
                    let mut content_end = i;
                    while i < n {
                        let d = chars[i];
                        if d == '\\' {
                            if i + 1 < n && chars[i + 1] == '\n' {
                                line += 1;
                            }
                            i += 2;
                            continue;
                        }
                        if d == '\n' {
                            if !triple {
                                content_end = i;
                                break; // unterminated string: stop at end of line
                            }
                            line += 1;
                        }
                        if d == c && (!triple || (i + 2 < n && chars[i + 1] == c && chars[i + 2] == c)) {
                            content_end = i;
                            i += q;
                            break;
                        }
                        i += 1;
                        content_end = i;
                    }
                    let content: String = chars[content_start..content_end.min(n)].iter().collect();
                    text.push(c);
                    if content.len() <= 60 && is_ident_like(&content) {
                        text.push_str(&content);
                    }
                    text.push(c);
                }
                '(' | '[' | '{' => {
                    depth += 1;
                    text.push(c);
                    i += 1;
                }
                ')' | ']' | '}' => {
                    depth = (depth - 1).max(0);
                    text.push(c);
                    i += 1;
                }
                _ => {
                    text.push(c);
                    i += 1;
                }
            }
        }

        let t = text.trim();
        if !t.is_empty() {
            out.push(LogicalLine { indent, text: t.to_string(), line: start_line });
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Small string helpers (all work on placeholder-stripped text, so brackets
// inside strings can't confuse them)
// ─────────────────────────────────────────────────────────────────────────────

/// Split `s` at `sep` characters that are not nested in brackets.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ if c == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Byte index of the first top-level occurrence of `ch`.
fn find_top(s: &str, ch: char) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        if c == ch && depth == 0 {
            return Some(i);
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ if c == ch && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Index of the bracket matching the opening bracket at byte `open`.
fn matching_close(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s[open..].char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Find the top-level assignment `=` (not `==`, `<=`, `+=`, `:=` …).
/// Returns all positions for chained assignments (`a = b = 0`).
fn assignment_positions(s: &str) -> Vec<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut out = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 => {
                let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                let prev = if i > 0 { bytes[i - 1] } else { b' ' };
                if next == b'=' || b"=!<>+-*/%&|^@:".contains(&prev) {
                    continue;
                }
                out.push(i);
            }
            _ => {}
        }
    }
    out
}

fn is_augmented_assignment(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 && i > 0 => {
                let prev = bytes[i - 1];
                let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                if next != b'=' && b"+-*/%&|^@".contains(&prev) {
                    return true;
                }
                // `**=`, `//=`, `>>=`, `<<=`
                if next != b'=' && (prev == b'>' || prev == b'<') && i > 1 && bytes[i - 2] == prev {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Leading identifier of `s` and the rest.
fn take_ident(s: &str) -> (&str, &str) {
    let end = s
        .char_indices()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

fn strip_keyword<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(kw)?;
    if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || c == '(' || c == ':') {
        Some(rest.trim_start())
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Type formatting
// ─────────────────────────────────────────────────────────────────────────────

/// Turn a Python annotation into the compact UML notation used in class.
/// `tuple[int, int]` → `(int,int)`, `pygame.Surface` → `Surface`,
/// `List[str]` → `list[str]`, `"Ball"` → `Ball`.
pub fn format_type(annotation: &str) -> String {
    let a: String = annotation.chars().filter(|c| !c.is_whitespace()).collect();
    format_type_inner(&a)
}

fn format_type_inner(a: &str) -> String {
    let a = a.trim_matches(|c| c == '"' || c == '\'');
    if a.is_empty() {
        return String::new();
    }
    // Unions: `int|None`
    let union = split_top(a, '|');
    if union.len() > 1 {
        return union.iter().map(|p| format_type_inner(p)).collect::<Vec<_>>().join(" | ");
    }
    if let Some(open) = find_top(a, '[') {
        let head = short_name(&a[..open]);
        let close = matching_close(a, open).unwrap_or(a.len());
        let inner = &a[open + 1..close.min(a.len())];
        let args: Vec<String> = split_top(inner, ',')
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(format_type_inner)
            .collect();
        return match head.as_str() {
            "tuple" | "Tuple" => format!("({})", args.join(",")),
            "List" => format!("list[{}]", args.join(",")),
            "Dict" => format!("dict[{}]", args.join(",")),
            "Set" => format!("set[{}]", args.join(",")),
            _ => format!("{}[{}]", head, args.join(",")),
        };
    }
    if a.starts_with('(') && a.ends_with(')') {
        let args: Vec<String> = split_top(&a[1..a.len() - 1], ',')
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(format_type_inner)
            .collect();
        return format!("({})", args.join(","));
    }
    match short_name(a).as_str() {
        "List" => "list".into(),
        "Dict" => "dict".into(),
        "Tuple" => "tuple".into(),
        "Set" => "set".into(),
        other => other.to_string(),
    }
}

/// `pygame.sprite.Sprite` → `Sprite`.
fn short_name(dotted: &str) -> String {
    dotted.rsplit('.').next().unwrap_or(dotted).to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Type inference
// ─────────────────────────────────────────────────────────────────────────────

struct Env<'a> {
    locals: &'a HashMap<String, String>,
    self_name: &'a str,
    class: Option<&'a ClassInfo>,
}

fn is_number(s: &str) -> Option<&'static str> {
    let s = s.trim_start_matches(['-', '+']);
    if s.is_empty() || !s.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    if s == "." {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("0x") || lower.starts_with("0o") || lower.starts_with("0b") {
        return if lower[2..].chars().all(|c| c.is_ascii_hexdigit() || c == '_') { Some("int") } else { None };
    }
    if lower.ends_with('j') {
        return Some("complex");
    }
    let valid = lower.chars().all(|c| c.is_ascii_digit() || matches!(c, '_' | '.' | 'e' | '+' | '-'));
    if !valid {
        return None;
    }
    if lower.contains('.') || lower.contains('e') {
        Some("float")
    } else {
        Some("int")
    }
}

/// `""`, `f""`, `rb''`, `"" ""` → Some("str"/"bytes").
fn string_literal_type(s: &str) -> Option<&'static str> {
    let prefix_end = s.find(['"', '\'']).unwrap_or(0);
    let prefix = &s[..prefix_end];
    if prefix.len() > 2 || !prefix.chars().all(|c| "rRbBfFuU".contains(c)) {
        return None;
    }
    let body = &s[prefix_end..];
    if body.len() < 2 {
        return None;
    }
    let q = body.chars().next()?;
    if !body.ends_with(q) {
        return None;
    }
    // Make sure it's a single literal (or implicit concatenation), not `"" + x`.
    let mut in_str = false;
    for c in body.chars() {
        if c == '"' || c == '\'' {
            in_str = !in_str;
        } else if !in_str && !c.is_whitespace() && !"rRbBfFuU".contains(c) {
            return None;
        }
    }
    Some(if prefix.contains(['b', 'B']) { "bytes" } else { "str" })
}

fn combine_numeric(a: &str, b: &str, op: &str) -> Option<String> {
    let numeric = |t: &str| t == "int" || t == "float" || t == "bool";
    if op == "/" && numeric(a) && numeric(b) {
        return Some("float".into());
    }
    if a == b {
        return Some(a.to_string());
    }
    if numeric(a) && numeric(b) {
        return Some(if a == "float" || b == "float" { "float" } else { "int" }.into());
    }
    if op == "*" && ((a == "str" && b == "int") || (a == "int" && b == "str")) {
        return Some("str".into());
    }
    if op == "+" && (a == "str" || b == "str") {
        return Some("str".into());
    }
    if a.starts_with("list") && b.starts_with("list") {
        return Some("list".into());
    }
    None
}

/// Split at top-level binary operators of one precedence level.
/// Returns (operand, operator-before-it) pairs.
fn split_binary<'a>(s: &'a str, ops: &[&str]) -> Option<Vec<(&'a str, &'a str)>> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut cuts: Vec<(usize, usize)> = Vec::new(); // (start, len) of operators
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'"' | b'\'' => {
                // skip placeholder strings
                let q = b;
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
            }
            _ if depth == 0 => {
                for op in ops {
                    if s[i..].starts_with(op) {
                        // `**` is not `*`; `//` handled as its own op.
                        if *op == "*" && (s[i..].starts_with("**") || (i > 0 && bytes[i - 1] == b'*')) {
                            continue;
                        }
                        if *op == "/" && (s[i..].starts_with("//") || (i > 0 && bytes[i - 1] == b'/')) {
                            continue;
                        }
                        // unary operator: nothing meaningful before it
                        let before = s[..i].trim_end();
                        let prev = before.chars().last();
                        let unary = match prev {
                            None => true,
                            Some(c) => "+-*/%(,=<>!&|^~[{:".contains(c),
                        };
                        // exponent in a float literal: 1e-5
                        let exponent = (op == &"+" || op == &"-")
                            && before.len() >= 2
                            && before.ends_with(['e', 'E'])
                            && before[..before.len() - 1].ends_with(|c: char| c.is_ascii_digit());
                        if !unary && !exponent {
                            cuts.push((i, op.len()));
                            i += op.len() - 1;
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    if cuts.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    let mut start = 0;
    let mut prev_op = "";
    for (pos, len) in cuts {
        parts.push((s[start..pos].trim(), prev_op));
        prev_op = &s[pos..pos + len];
        start = pos + len;
    }
    parts.push((s[start..].trim(), prev_op));
    Some(parts)
}

fn has_top_level_word(s: &str, word: &str) -> bool {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ if depth == 0 && s[i..].starts_with(word) => {
                let before_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
                let after = i + word.len();
                let after_ok = after >= bytes.len() || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
                if before_ok && after_ok {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn has_top_level_comparison(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'<' | b'>' if depth == 0 => {
                let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                let prev = if i > 0 { bytes[i - 1] } else { b' ' };
                // shifts `<<` / `>>` and `->` are not comparisons
                if next != b && prev != b && prev != b'-' {
                    return true;
                }
            }
            b'=' if depth == 0 => {
                if bytes.get(i + 1) == Some(&b'=') {
                    return true;
                }
            }
            b'!' if depth == 0 && bytes.get(i + 1) == Some(&b'=') => return true,
            _ => {}
        }
    }
    has_top_level_word(s, "in") || has_top_level_word(s, "is")
}

/// Known function → return type (common in school code).
fn known_call_type(callee: &str, args: &str) -> Option<String> {
    let last = callee.rsplit('.').next().unwrap_or(callee);
    let t = match callee {
        "int" | "len" | "ord" | "hash" | "id" => "int",
        "float" => "float",
        "str" | "input" | "chr" | "repr" | "hex" | "bin" | "format" => "str",
        "bool" | "isinstance" | "callable" | "any" | "all" => "bool",
        "list" | "sorted" => "list",
        "dict" => "dict",
        "set" => "set",
        "tuple" => "tuple",
        "range" => "range",
        "round" => {
            if split_top(args, ',').len() > 1 {
                "float"
            } else {
                "int"
            }
        }
        "random.randint" | "randint" | "random.randrange" | "randrange" => "int",
        "random.random" | "random.uniform" | "uniform" => "float",
        "math.floor" | "math.ceil" | "math.factorial" | "math.gcd" => "int",
        "time.time" | "time.perf_counter" => "float",
        "pygame.display.set_mode" | "pygame.image.load" | "pygame.Surface" => "Surface",
        "pygame.font.SysFont" | "pygame.font.Font" => "Font",
        "pygame.key.get_pressed" => "ScancodeWrapper",
        "pygame.mouse.get_pos" => "(int,int)",
        "pygame.time.get_ticks" => "int",
        _ => "",
    };
    if !t.is_empty() {
        return Some(t.to_string());
    }
    if callee.starts_with("math.") {
        return Some("float".into());
    }
    if callee.starts_with("pygame.transform.") {
        return Some("Surface".into());
    }
    match last {
        "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace" | "join" | "title" | "capitalize"
        | "format" => return Some("str".into()),
        "split" | "splitlines" | "keys" | "values" | "items" => {
            return Some(if last.starts_with("split") { "list[str]" } else { "list" }.into());
        }
        "get_rect" => return Some("Rect".into()),
        "render" | "convert" | "convert_alpha" | "subsurface" => return Some("Surface".into()),
        "get_width" | "get_height" | "count" | "index" | "find" => return Some("int".into()),
        "colliderect" | "collidepoint" | "startswith" | "endswith" | "isdigit" | "isalpha" => {
            return Some("bool".into());
        }
        _ => {}
    }
    // Constructor call: `Ball(…)`, `pygame.Rect(…)`.
    if last.starts_with(|c: char| c.is_uppercase()) && is_identifier(last) {
        return Some(last.to_string());
    }
    None
}

fn infer(expr: &str, env: &Env) -> Option<String> {
    let e = expr.trim();
    if e.is_empty() {
        return None;
    }
    if e.starts_with("lambda") || e.starts_with("await ") || e.starts_with("yield") {
        return None;
    }
    if strip_keyword(e, "not").is_some() {
        return Some("bool".into());
    }
    // `a if cond else b`
    if has_top_level_word(e, "if") && has_top_level_word(e, "else") {
        let cut = e.find(" if ").unwrap_or(e.len());
        return infer(&e[..cut], env);
    }
    if has_top_level_word(e, "or") || has_top_level_word(e, "and") {
        return None;
    }
    if has_top_level_comparison(e) {
        return Some("bool".into());
    }
    // Bare tuple: `0, 0`
    let parts = split_top(e, ',');
    if parts.len() > 1 {
        return Some(tuple_type(&parts, env));
    }
    for level in [&["+", "-"][..], &["*", "//", "/", "%"][..], &["**"][..]] {
        if let Some(operands) = split_binary(e, level) {
            let mut acc: Option<String> = None;
            for (operand, op) in operands {
                let t = infer(operand, env)?;
                acc = Some(match acc {
                    None => t,
                    Some(prev) => combine_numeric(&prev, &t, op)?,
                });
            }
            return acc;
        }
    }
    if let Some(t) = is_number(e) {
        return Some(t.into());
    }
    if let Some(t) = string_literal_type(e) {
        return Some(t.into());
    }
    match e {
        "True" | "False" => return Some("bool".into()),
        "None" | "..." => return None,
        _ => {}
    }
    let first = e.chars().next()?;
    let last = e.chars().last()?;
    // Whole expression wrapped in brackets?
    if matches!(first, '(' | '[' | '{') && matching_close(e, 0) == Some(e.len() - 1) {
        let inner = e[1..e.len() - 1].trim();
        let comprehension = has_top_level_word(inner, "for");
        return match first {
            '[' => {
                if comprehension || inner.is_empty() {
                    Some("list".into())
                } else {
                    Some(homogeneous("list", &split_top(inner, ','), env))
                }
            }
            '{' => {
                if inner.is_empty() || find_top(inner, ':').is_some() {
                    Some("dict".into())
                } else if comprehension {
                    Some("set".into())
                } else {
                    Some(homogeneous("set", &split_top(inner, ','), env))
                }
            }
            _ => {
                if inner.is_empty() {
                    Some("tuple".into())
                } else if comprehension {
                    None
                } else {
                    let parts = split_top(inner, ',');
                    if parts.len() > 1 {
                        Some(tuple_type(&parts, env))
                    } else {
                        infer(inner, env)
                    }
                }
            }
        };
    }
    // Call: `callee(args)`
    if last == ')' {
        if let Some(open) = find_call_open(e) {
            let callee = e[..open].trim();
            let args = &e[open + 1..e.len() - 1];
            if callee.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                if let Some(t) = known_call_type(callee, args) {
                    return Some(t);
                }
                // `self.method()` → the method's declared return type is unknown here.
                return None;
            }
            // `"".join(…)`, `"{}".format(…)`
            if let Some(dot) = callee.rfind('.') {
                let method = &callee[dot + 1..];
                return known_call_type(method, args);
            }
        }
        return None;
    }
    // Names and `self.attr`
    if is_identifier(e) {
        return env.locals.get(e).cloned();
    }
    if let Some(attr) = e.strip_prefix(env.self_name).and_then(|r| r.strip_prefix('.')) {
        if is_identifier(attr) {
            return env.class.and_then(|c| c.attribute_type(attr)).map(str::to_string);
        }
    }
    None
}

/// Opening paren of the final call in `e` (which ends with `)`).
fn find_call_open(e: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in e.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                depth -= 1;
                if depth == 0 {
                    return if c == '(' && i > 0 { Some(i) } else { None };
                }
            }
            _ => {}
        }
    }
    None
}

fn tuple_type(parts: &[&str], env: &Env) -> String {
    let types: Vec<Option<String>> = parts
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| infer(p, env))
        .collect();
    if !types.is_empty() && types.iter().all(Option::is_some) {
        let names: Vec<String> = types.into_iter().map(Option::unwrap).collect();
        format!("({})", names.join(","))
    } else {
        "tuple".into()
    }
}

fn homogeneous(kind: &str, parts: &[&str], env: &Env) -> String {
    let mut elem: Option<String> = None;
    for p in parts.iter().filter(|p| !p.trim().is_empty()) {
        match (infer(p, env), &elem) {
            (Some(t), None) => elem = Some(t),
            (Some(t), Some(prev)) if &t == prev => {}
            _ => return kind.to_string(),
        }
    }
    match elem {
        Some(t) => format!("{kind}[{t}]"),
        None => kind.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Walking the file
// ─────────────────────────────────────────────────────────────────────────────

enum Ctx {
    Class {
        indent: usize,
        idx: usize,
        dataclass: bool,
    },
    Def {
        indent: usize,
        /// Class this function belongs to (also for nested helper functions).
        class_idx: Option<usize>,
        /// Index of the method in the class, only for the method itself.
        method_idx: Option<usize>,
        self_name: String,
        locals: HashMap<String, String>,
        returns: Vec<Option<String>>,
    },
    /// Function outside any class – we only need to skip its body.
    Other { indent: usize },
}

impl Ctx {
    fn indent(&self) -> usize {
        match self {
            Ctx::Class { indent, .. } | Ctx::Def { indent, .. } | Ctx::Other { indent } => *indent,
        }
    }
}

fn parse_params(params: &str, skip_first: bool, locals: &mut HashMap<String, String>) -> (Vec<Param>, String) {
    let mut out = Vec::new();
    let mut self_name = String::new();
    let mut first = true;
    for raw in split_top(params, ',') {
        let p = raw.trim();
        if p.is_empty() || p == "/" || p == "*" {
            continue;
        }
        let (name_part, default) = match find_top(p, '=') {
            Some(i) => (p[..i].trim(), Some(p[i + 1..].trim())),
            None => (p, None),
        };
        let (name, ann) = match find_top(name_part, ':') {
            Some(i) => (name_part[..i].trim(), Some(name_part[i + 1..].trim())),
            None => (name_part, None),
        };
        if first && skip_first {
            first = false;
            self_name = name.to_string();
            continue;
        }
        first = false;
        let empty = HashMap::new();
        let env = Env { locals: &empty, self_name: "self", class: None };
        let ty = ann
            .map(format_type)
            .filter(|t| !t.is_empty())
            .or_else(|| default.and_then(|d| infer(d, &env)));
        let clean = name.trim_start_matches('*');
        if let Some(t) = &ty {
            locals.insert(clean.to_string(), t.clone());
        }
        out.push(Param { name: name.to_string(), ty });
    }
    (out, self_name)
}

/// Header of a `def`: name, params text, return annotation, body after `:`.
fn split_def(rest: &str) -> Option<(&str, &str, Option<&str>, &str)> {
    let (name, after) = take_ident(rest);
    if name.is_empty() {
        return None;
    }
    let after = after.trim_start();
    if !after.starts_with('(') {
        return None;
    }
    let open = rest.len() - after.len();
    let close = matching_close(rest, open).unwrap_or(rest.len().saturating_sub(1));
    let params = rest.get(open + 1..close).unwrap_or("");
    let tail = rest.get(close + 1..).unwrap_or("");
    let colon = find_top(tail, ':');
    let header = match colon {
        Some(c) => &tail[..c],
        None => tail,
    };
    let body = colon.map(|c| tail[c + 1..].trim()).unwrap_or("");
    let ret = header.trim().strip_prefix("->").map(str::trim).filter(|s| !s.is_empty());
    Some((name, params, ret, body))
}

fn parse_class_header(rest: &str) -> Option<(String, Vec<String>, &str)> {
    let (name, after) = take_ident(rest);
    if name.is_empty() {
        return None;
    }
    let after = after.trim_start();
    let mut bases = Vec::new();
    let mut tail = after;
    if after.starts_with('(') {
        let open = rest.len() - after.len();
        let close = matching_close(rest, open).unwrap_or(rest.len().saturating_sub(1));
        for b in split_top(rest.get(open + 1..close).unwrap_or(""), ',') {
            let b = b.trim();
            if b.is_empty() || b.contains('=') || b == "object" {
                continue;
            }
            bases.push(b.to_string());
        }
        tail = rest.get(close + 1..).unwrap_or("");
    }
    let body = find_top(tail, ':').map(|c| tail[c + 1..].trim()).unwrap_or("");
    Some((name.to_string(), bases, body))
}

/// Parse one file and append its classes to `classes`.
pub fn parse_file(file: &str, src: &str, classes: &mut Vec<ClassInfo>) {
    let lines = logical_lines(src);
    let mut stack: Vec<Ctx> = Vec::new();
    let mut decorators: Vec<String> = Vec::new();

    let mut queue: Vec<LogicalLine> = Vec::new();
    let mut iter = lines.into_iter();
    loop {
        let ll = match queue.pop() {
            Some(l) => l,
            None => match iter.next() {
                Some(l) => l,
                None => break,
            },
        };

        // Close blocks we've left.
        while let Some(top) = stack.last() {
            if ll.indent <= top.indent() {
                let ctx = stack.pop().unwrap();
                finish_ctx(ctx, classes);
            } else {
                break;
            }
        }

        let text = ll.text.as_str();

        if let Some(dec) = text.strip_prefix('@') {
            decorators.push(dec.trim().to_string());
            continue;
        }
        let decos = std::mem::take(&mut decorators);

        // Single-line compound statement body: `def f(self): return 1`
        let push_body = |body: &str, queue: &mut Vec<LogicalLine>| {
            if !body.is_empty() {
                queue.push(LogicalLine { indent: ll.indent + 1, text: body.to_string(), line: ll.line });
            }
        };

        if let Some(rest) = strip_keyword(text, "class") {
            if let Some((name, bases, body)) = parse_class_header(rest) {
                let dataclass = decos.iter().any(|d| d.contains("dataclass"));
                classes.push(ClassInfo {
                    name,
                    bases,
                    attributes: Vec::new(),
                    methods: Vec::new(),
                    file: file.to_string(),
                    line: ll.line,
                });
                stack.push(Ctx::Class { indent: ll.indent, idx: classes.len() - 1, dataclass });
                push_body(body, &mut queue);
            }
            continue;
        }

        let def_rest = strip_keyword(text, "def")
            .or_else(|| strip_keyword(text, "async").and_then(|r| strip_keyword(r, "def")));
        if let Some(rest) = def_rest {
            let Some((name, params, ret, body)) = split_def(rest) else { continue };
            match stack.last() {
                Some(Ctx::Class { idx, .. }) => {
                    let idx = *idx;
                    let is_static = decos.iter().any(|d| d == "staticmethod");
                    let mut locals = HashMap::new();
                    let (params, self_name) = parse_params(params, !is_static, &mut locals);
                    let ret = ret.map(format_type).filter(|r| r != "None" && !r.is_empty());
                    let explicit_ret = ret.is_some();
                    classes[idx].methods.push(Method {
                        name: name.to_string(),
                        params,
                        ret,
                        is_static: is_static || decos.iter().any(|d| d == "classmethod"),
                        line: ll.line,
                    });
                    let method_idx = classes[idx].methods.len() - 1;
                    stack.push(Ctx::Def {
                        indent: ll.indent,
                        class_idx: Some(idx),
                        method_idx: if explicit_ret { None } else { Some(method_idx) },
                        self_name: if is_static { String::new() } else { self_name },
                        locals,
                        returns: Vec::new(),
                    });
                }
                Some(Ctx::Def { class_idx: Some(ci), self_name, .. }) => {
                    // nested helper function inside a method: keep tracking `self`
                    let (ci, self_name) = (*ci, self_name.clone());
                    let mut locals = HashMap::new();
                    parse_params(params, false, &mut locals);
                    stack.push(Ctx::Def {
                        indent: ll.indent,
                        class_idx: Some(ci),
                        method_idx: None,
                        self_name,
                        locals,
                        returns: Vec::new(),
                    });
                }
                _ => stack.push(Ctx::Other { indent: ll.indent }),
            }
            push_body(body, &mut queue);
            continue;
        }

        match stack.last_mut() {
            Some(Ctx::Class { idx, dataclass, .. }) => {
                class_body_statement(text, &mut classes[*idx], *dataclass);
            }
            Some(Ctx::Def { class_idx: Some(ci), self_name, locals, returns, method_idx, .. }) => {
                let ci = *ci;
                let track_returns = method_idx.is_some();
                method_statement(text, &mut classes[ci], self_name, locals, returns, track_returns);
            }
            _ => {}
        }
    }
    while let Some(ctx) = stack.pop() {
        finish_ctx(ctx, classes);
    }
}

fn finish_ctx(ctx: Ctx, classes: &mut [ClassInfo]) {
    if let Ctx::Def { class_idx: Some(ci), method_idx: Some(mi), returns, .. } = ctx {
        let typed: Vec<&String> = returns.iter().flatten().collect();
        let any_unknown = returns.iter().any(Option::is_none);
        if !typed.is_empty() && !any_unknown && typed.iter().all(|t| *t == typed[0]) {
            if let Some(m) = classes[ci].methods.get_mut(mi) {
                m.ret = Some(typed[0].clone());
            }
        }
    }
}

const STATEMENT_KEYWORDS: &[&str] = &[
    "if", "elif", "else", "for", "while", "with", "try", "except", "finally", "return", "pass", "break",
    "continue", "raise", "import", "from", "global", "nonlocal", "del", "assert", "print", "match", "case",
];

fn starts_with_keyword(text: &str) -> bool {
    STATEMENT_KEYWORDS.iter().any(|kw| strip_keyword(text, kw).is_some())
}

fn class_body_statement(text: &str, class: &mut ClassInfo, dataclass: bool) {
    if starts_with_keyword(text) || is_augmented_assignment(text) {
        return;
    }
    let assigns = assignment_positions(text);
    let (target, value) = match assigns.last() {
        Some(&pos) => (text[..assigns[0]].trim(), Some(text[pos + 1..].trim())),
        None => (text.trim(), None),
    };
    let (name, ann) = match find_top(target, ':') {
        Some(i) => (target[..i].trim(), Some(target[i + 1..].trim())),
        None => (target, None),
    };
    if value.is_none() && ann.is_none() {
        return; // expression statement / docstring
    }
    let empty = HashMap::new();
    let env = Env { locals: &empty, self_name: "self", class: None };
    let ty = ann
        .map(format_type)
        .filter(|t| !t.is_empty() && !t.starts_with("ClassVar"))
        .or_else(|| value.and_then(|v| infer(v, &env)));
    let is_static = !dataclass && !(ann.is_some() && value.is_none());
    for n in split_top(name, ',') {
        let n = n.trim();
        if !is_identifier(n) || class.attributes.iter().any(|a| a.name == n) {
            continue;
        }
        class.attributes.push(Attribute { name: n.to_string(), ty: ty.clone(), is_static });
    }
}

fn method_statement(
    text: &str,
    class: &mut ClassInfo,
    self_name: &str,
    locals: &mut HashMap<String, String>,
    returns: &mut Vec<Option<String>>,
    track_returns: bool,
) {
    if let Some(rest) = strip_keyword(text, "return") {
        if track_returns && !rest.is_empty() && rest != "None" {
            let env = Env { locals, self_name, class: Some(class) };
            returns.push(infer(rest, &env));
        }
        return;
    }
    if starts_with_keyword(text) || is_augmented_assignment(text) {
        return;
    }
    let assigns = assignment_positions(text);
    let Some(&last) = assigns.last() else {
        // bare annotation: `self.x: int`
        if let Some(i) = find_top(text, ':') {
            record_target(text[..i].trim(), Some(text[i + 1..].trim()), None, class, self_name, locals);
        }
        return;
    };
    let value = text[last + 1..].trim();
    let mut start = 0;
    for &pos in &assigns {
        let target = text[start..pos].trim();
        start = pos + 1;
        let (target, ann) = match find_top(target, ':') {
            Some(i) => (target[..i].trim(), Some(target[i + 1..].trim())),
            None => (target, None),
        };
        let targets = split_top(target, ',');
        if targets.len() > 1 {
            // `self.x, self.y = 0, 0`
            let values = split_top(value, ',');
            let values = if values.len() == 1 {
                let v = value.trim();
                if v.starts_with('(') && matching_close(v, 0) == Some(v.len() - 1) {
                    split_top(&v[1..v.len() - 1], ',')
                } else {
                    values
                }
            } else {
                values
            };
            for (i, t) in targets.iter().enumerate() {
                let v = if values.len() == targets.len() { Some(values[i]) } else { None };
                record_target(t.trim(), None, v, class, self_name, locals);
            }
        } else {
            record_target(target, ann, Some(value), class, self_name, locals);
        }
    }
}

fn record_target(
    target: &str,
    ann: Option<&str>,
    value: Option<&str>,
    class: &mut ClassInfo,
    self_name: &str,
    locals: &mut HashMap<String, String>,
) {
    let ty = {
        let env = Env { locals, self_name, class: Some(class) };
        ann.map(format_type)
            .filter(|t| !t.is_empty())
            .or_else(|| value.and_then(|v| infer(v, &env)))
    };
    if !self_name.is_empty() {
        if let Some(attr) = target.strip_prefix(self_name).and_then(|r| r.strip_prefix('.')) {
            if !is_identifier(attr) {
                return;
            }
            match class.attribute_mut(attr) {
                Some(existing) => {
                    if existing.ty.is_none() {
                        existing.ty = ty;
                    }
                    // `self.count = 0` in a method shadows a class attribute.
                    existing.is_static = false;
                }
                None => class.attributes.push(Attribute { name: attr.to_string(), ty, is_static: false }),
            }
            return;
        }
    }
    if is_identifier(target) {
        match ty {
            Some(t) => {
                locals.insert(target.to_string(), t);
            }
            None => {
                locals.remove(target);
            }
        }
    }
}

/// Parse all files of a project. Duplicate class names keep the first one.
pub fn parse_project<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    for (name, src) in files {
        parse_file(name, src, &mut classes);
    }
    let mut seen = std::collections::HashSet::new();
    classes.retain(|c| seen.insert(c.name.clone()));
    classes
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Vec<ClassInfo> {
        parse_project([("test.py", src)])
    }

    fn attrs(c: &ClassInfo) -> Vec<String> {
        c.attributes.iter().map(Attribute::display).collect()
    }

    fn methods(c: &ClassInfo) -> Vec<String> {
        c.methods.iter().map(Method::display).collect()
    }

    #[test]
    fn basic_class() {
        let src = r#"
class Point:
    """A point."""
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
        self.name = "P"
        self.visible = True

    def move(self, p: tuple[int, int]) -> None:
        self.x += p[0]
        self.y += p[1]

    def distance(self, other: "Point") -> float:
        return ((self.x - other.x) ** 2 + (self.y - other.y) ** 2) ** 0.5
"#;
        let c = parse(src);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "Point");
        assert_eq!(attrs(&c[0]), ["x: int", "y: int", "name: str", "visible: bool"]);
        assert_eq!(
            methods(&c[0]),
            ["__init__(x:int, y:int)", "move(p:(int,int))", "distance(other:Point): float"]
        );
    }

    #[test]
    fn inference_and_inheritance() {
        let src = r#"
import pygame
import random

class Ball(pygame.sprite.Sprite):
    count = 0

    def __init__(self, x, y, radius=10):
        super().__init__()
        self.pos = (x, y)
        self.radius = radius
        self.speed = (random.randint(1, 5), 3)
        self.color = (255, 200, 0)
        self.image = pygame.Surface((radius * 2, radius * 2))
        self.rect = self.image.get_rect()
        self.items = []
        self.scores = [1, 2, 3]
        self.mass = self.radius * 1.5
        self.label = "Ball " + str(Ball.count)
        self.target = None
        Ball.count += 1

    def is_big(self):
        return self.radius > 20

    def area(self):
        return 3.14 * self.radius ** 2

class BigBall(Ball):
    def __init__(self):
        super().__init__(0, 0, 50)
        self.a, self.b = 1, "x"
"#;
        let c = parse(src);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].bases, ["pygame.sprite.Sprite"]);
        assert_eq!(
            attrs(&c[0]),
            [
                "count: int",
                "pos: tuple",
                "radius: int",
                "speed: (int,int)",
                "color: (int,int,int)",
                "image: Surface",
                "rect: Rect",
                "items: list",
                "scores: list[int]",
                "mass: float",
                "label: str",
                "target",
            ]
        );
        assert!(c[0].attributes[0].is_static);
        assert_eq!(methods(&c[0]), ["__init__(x, y, radius:int)", "is_big(): bool", "area(): float"]);
        assert_eq!(c[1].bases, ["Ball"]);
        assert_eq!(attrs(&c[1]), ["a: int", "b: str"]);
    }

    #[test]
    fn dataclass_and_decorators() {
        let src = r#"
from dataclasses import dataclass

@dataclass
class Item:
    name: str
    price: float = 0.0

    @staticmethod
    def create(name: str) -> "Item":
        return Item(name)

    @property
    def label(self) -> str:
        return self.name
"#;
        let c = parse(src);
        assert_eq!(attrs(&c[0]), ["name: str", "price: float"]);
        assert!(!c[0].attributes[1].is_static);
        assert_eq!(methods(&c[0]), ["create(name:str): Item", "label(): str"]);
    }

    #[test]
    fn broken_code_does_not_panic() {
        let src = "class A(:\n    def f(self, x: int\n        self.y = \"unterminated\nclass B:\n    def g(self): return 1\n";
        let c = parse(src);
        assert!(c.iter().any(|c| c.name == "A"));
        for s in ["class", "class :", "def", "def (", "self.", "x = (", "'''", "class A:\n  def f(self", "@", "a = b ="] {
            parse(s);
        }
    }

    #[test]
    fn multiline_and_comments() {
        let src = "class Game:\n    def __init__(self,\n                 width: int,   # the width\n                 height: int = 480):\n        self.size = (width,\n                     height)\n        self.balls: list[Ball] = []\n";
        let c = parse(src);
        assert_eq!(methods(&c[0]), ["__init__(width:int, height:int)"]);
        assert_eq!(attrs(&c[0]), ["size: (int,int)", "balls: list[Ball]"]);
        assert_eq!(c[0].methods[0].line, 2);
    }

    #[test]
    fn format_types() {
        assert_eq!(format_type("tuple[int, int]"), "(int,int)");
        assert_eq!(format_type("Tuple[int,str]"), "(int,str)");
        assert_eq!(format_type("List[pygame.Rect]"), "list[Rect]");
        assert_eq!(format_type("dict[str, list[int]]"), "dict[str,list[int]]");
        assert_eq!(format_type("int | None"), "int | None");
        assert_eq!(format_type("'Ball'"), "Ball");
    }
}
