//! UML → code: edits Python source text when the user adds members in the
//! diagram. All functions take the full file text and return the new text
//! plus the (1-based) line the cursor should jump to.

use super::model::Param;

/// Convert a UML-style type (`(int,int)`, `list[Ball]`) to a Python
/// annotation (`tuple[int, int]`, `list[Ball]`).
pub fn to_python_type(uml: &str) -> String {
    let t = uml.trim();
    if t.starts_with('(') && t.ends_with(')') {
        let inner = &t[1..t.len() - 1];
        let parts: Vec<String> = split_commas(inner).iter().map(|p| to_python_type(p)).collect();
        return format!("tuple[{}]", parts.join(", "));
    }
    if let Some(open) = t.find('[') {
        if t.ends_with(']') {
            let head = &t[..open];
            let parts: Vec<String> =
                split_commas(&t[open + 1..t.len() - 1]).iter().map(|p| to_python_type(p)).collect();
            return format!("{head}[{}]", parts.join(", "));
        }
    }
    t.to_string()
}

fn split_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// A sensible starting value for an attribute of the given UML type.
pub fn default_value(uml_type: &str) -> String {
    let t = uml_type.trim();
    if t.starts_with('(') && t.ends_with(')') {
        let parts: Vec<String> = split_commas(&t[1..t.len() - 1]).iter().map(|p| default_value(p)).collect();
        return if parts.len() == 1 { format!("({},)", parts[0]) } else { format!("({})", parts.join(", ")) };
    }
    let head = t.split('[').next().unwrap_or(t);
    match head {
        "int" => "0".into(),
        "float" => "0.0".into(),
        "str" => "\"\"".into(),
        "bool" => "False".into(),
        "list" => "[]".into(),
        "dict" => "{}".into(),
        "set" => "set()".into(),
        "tuple" => "()".into(),
        _ => "None".into(),
    }
}

/// Parse user input like `p:(int,int), name: str` into params.
pub fn parse_param_list(input: &str) -> Vec<Param> {
    split_commas(input)
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| match p.find(':') {
            Some(i) => Param {
                name: p[..i].trim().to_string(),
                ty: Some(p[i + 1..].trim().to_string()).filter(|t| !t.is_empty()),
            },
            None => Param { name: p.trim().to_string(), ty: None },
        })
        .collect()
}

pub fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && !PY_KEYWORDS.contains(&s)
}

const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
    "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

fn python_params(params: &[Param]) -> String {
    params
        .iter()
        .map(|p| match &p.ty {
            Some(t) => format!("{}: {}", p.name, to_python_type(t)),
            None => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ─────────────────────────────────────────────────────────────────────────────
// Source structure helpers (physical lines)
// ─────────────────────────────────────────────────────────────────────────────

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// Line index (0-based) just after the block that starts at `header` —
/// i.e. where code at the header's indentation would continue.
/// Trailing blank lines are not part of the block.
fn block_end(lines: &[&str], header: usize) -> usize {
    let base = indent_of(lines[header]);
    let mut i = header_end(lines, header) + 1;
    let mut last_body = header_end(lines, header);
    while i < lines.len() {
        if !is_blank_or_comment(lines[i]) {
            if indent_of(lines[i]) <= base {
                break;
            }
            last_body = i;
        }
        i += 1;
    }
    last_body + 1
}

/// Last line of a (possibly multi-line) header like `def f(a,\n  b):`.
fn header_end(lines: &[&str], header: usize) -> usize {
    let mut depth = 0i32;
    for (i, line) in lines.iter().enumerate().skip(header) {
        for c in line.chars() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                '#' => break,
                _ => {}
            }
        }
        if depth <= 0 {
            return i;
        }
    }
    header
}

/// Indentation used by the body of the block at `header`.
fn body_indent(lines: &[&str], header: usize) -> usize {
    let base = indent_of(lines[header]);
    for line in lines.iter().skip(header_end(lines, header) + 1) {
        if is_blank_or_comment(line) {
            continue;
        }
        let ind = indent_of(line);
        return if ind > base { ind } else { base + 4 };
    }
    base + 4
}

fn find_method(lines: &[&str], class_idx: usize, name: &str) -> Option<usize> {
    let end = block_end(lines, class_idx);
    let body = body_indent(lines, class_idx);
    (class_idx + 1..end).find(|&i| {
        let t = lines[i].trim_start();
        indent_of(lines[i]) == body
            && (t.strip_prefix("def ").or_else(|| t.strip_prefix("async def ")))
                .map(|r| r.trim_start().starts_with(&format!("{name}(")) || r.trim_start() == name)
                .unwrap_or(false)
    })
}

/// If the block body is only `pass` / `...` / a docstring + pass, return the
/// index of that `pass` line so it can be replaced.
fn lone_pass(lines: &[&str], header: usize) -> Option<usize> {
    let end = block_end(lines, header);
    let body: Vec<usize> =
        (header_end(lines, header) + 1..end).filter(|&i| !is_blank_or_comment(lines[i])).collect();
    match body.as_slice() {
        [only] if matches!(lines[*only].trim(), "pass" | "...") => Some(*only),
        _ => None,
    }
}

fn join(lines: Vec<String>, trailing_newline: bool) -> String {
    let mut s = lines.join("\n");
    if trailing_newline {
        s.push('\n');
    }
    s
}

fn to_owned_lines(src: &str) -> (Vec<String>, bool) {
    let trailing = src.ends_with('\n');
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    (lines, trailing || src.is_empty())
}

// ─────────────────────────────────────────────────────────────────────────────
// Edits
// ─────────────────────────────────────────────────────────────────────────────

/// Append a new class at the end of the file.
pub fn add_class(src: &str, name: &str, base: Option<&str>) -> (String, usize) {
    let mut out = src.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n\n");
    }
    let header_line = out.lines().count() + 1;
    let base = base.map(str::trim).filter(|b| !b.is_empty());
    match base {
        Some(b) => {
            out.push_str(&format!("class {name}({b}):\n    def __init__(self):\n        super().__init__()\n"))
        }
        None => out.push_str(&format!("class {name}:\n    def __init__(self):\n        pass\n")),
    }
    (out, header_line)
}

/// Add an attribute. When `as_param` is true it also becomes a constructor
/// parameter (`def __init__(self, x: int)` + `self.x = x`).
pub fn add_attribute(
    src: &str,
    class_line: usize,
    name: &str,
    uml_type: Option<&str>,
    as_param: bool,
) -> Option<(String, usize)> {
    let class_idx = class_line.checked_sub(1)?;
    let borrowed: Vec<&str> = src.lines().collect();
    if class_idx >= borrowed.len() {
        return None;
    }
    let uml_type = uml_type.map(str::trim).filter(|t| !t.is_empty());

    let init = match find_method(&borrowed, class_idx, "__init__") {
        Some(i) => i,
        None => {
            // Create a constructor first, then add to it.
            let (with_init, _) = add_method(src, class_line, "__init__", &[], None, true)?;
            return add_attribute(&with_init, class_line, name, uml_type, as_param);
        }
    };

    let (mut lines, trailing) = to_owned_lines(src);
    let body_ind = " ".repeat(body_indent(&borrowed, init));

    let value = if as_param {
        name.to_string()
    } else {
        uml_type.map(default_value).unwrap_or_else(|| "None".into())
    };
    // Keep the type visible in code (and the UML) when the value can't show it.
    let needs_annotation = !as_param && value == "None" && uml_type.is_some();
    let stmt = if needs_annotation {
        format!("{body_ind}self.{name}: {} = None", to_python_type(uml_type.unwrap()))
    } else {
        format!("{body_ind}self.{name} = {value}")
    };

    let pass = lone_pass(&borrowed, init);
    let insert_at = block_end(&borrowed, init);
    let mut target_line;
    if let Some(p) = pass {
        lines[p] = stmt;
        target_line = p + 1;
    } else {
        lines.insert(insert_at, stmt);
        target_line = insert_at + 1;
    }

    if as_param {
        let hdr_end = header_end(&borrowed, init);
        // Insert the parameter just before the closing `)` of the header.
        let line = &lines[hdr_end];
        if let Some(close) = line.rfind(')') {
            let before = line[..close].trim_end();
            let sep = if before.ends_with('(') { "" } else { ", " };
            let param = match uml_type {
                Some(t) => format!("{name}: {}", to_python_type(t)),
                None => name.to_string(),
            };
            lines[hdr_end] = format!("{before}{sep}{param}{}", &line[close..]);
        }
        let _ = &mut target_line;
    }
    Some((join(lines, trailing), target_line))
}

/// Add a method at the end of the class (or a constructor at its start).
pub fn add_method(
    src: &str,
    class_line: usize,
    name: &str,
    params: &[Param],
    uml_ret: Option<&str>,
    assign_params: bool,
) -> Option<(String, usize)> {
    let class_idx = class_line.checked_sub(1)?;
    let borrowed: Vec<&str> = src.lines().collect();
    if class_idx >= borrowed.len() {
        return None;
    }
    let (mut lines, trailing) = to_owned_lines(src);
    let ind = " ".repeat(body_indent(&borrowed, class_idx));
    let ind2 = format!("{ind}    ");

    let ps = python_params(params);
    let sig_params = if ps.is_empty() { "self".to_string() } else { format!("self, {ps}") };
    let ret = uml_ret
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(|r| format!(" -> {}", to_python_type(r)))
        .unwrap_or_default();

    let mut block = vec![format!("{ind}def {name}({sig_params}){ret}:")];
    if assign_params && !params.is_empty() {
        for p in params {
            block.push(format!("{ind2}self.{0} = {0}", p.name));
        }
    } else if let Some(r) = uml_ret.map(str::trim).filter(|r| !r.is_empty() && *r != "None") {
        block.push(format!("{ind2}return {}", default_value(r)));
    } else {
        block.push(format!("{ind2}pass"));
    }

    let is_init = name == "__init__";
    let pass = lone_pass(&borrowed, class_idx);
    let (insert_at, replace) = match (pass, is_init) {
        (Some(p), _) => (p, true),
        (None, true) => {
            // constructor goes first in the class body (after a docstring)
            let mut at = header_end(&borrowed, class_idx) + 1;
            while at < borrowed.len() && is_blank_or_comment(borrowed[at]) {
                at += 1;
            }
            if at < borrowed.len() {
                let t = borrowed[at].trim_start();
                if t.starts_with("\"\"\"") || t.starts_with("'''") {
                    let q = &t[..3];
                    let mut j = at;
                    if t.len() < 6 || !t[3..].contains(q) {
                        j += 1;
                        while j < borrowed.len() && !borrowed[j].contains(q) {
                            j += 1;
                        }
                    }
                    at = j + 1;
                }
            }
            (at, false)
        }
        (None, false) => (block_end(&borrowed, class_idx), false),
    };

    let mut insertion = Vec::new();
    if !replace && !is_init {
        insertion.push(String::new());
    }
    let def_offset = insertion.len();
    insertion.extend(block);
    if !replace && is_init {
        insertion.push(String::new());
    }

    if replace {
        lines.splice(insert_at..insert_at + 1, insertion);
    } else {
        let at = insert_at.min(lines.len());
        lines.splice(at..at, insertion);
    }
    Some((join(lines, trailing), insert_at + def_offset + 1))
}

/// Remove a method (its decorators, signature and body).
pub fn delete_method(src: &str, def_line: usize) -> Option<(String, usize)> {
    let idx = def_line.checked_sub(1)?;
    let borrowed: Vec<&str> = src.lines().collect();
    if idx >= borrowed.len() {
        return None;
    }
    let mut start = idx;
    while start > 0 && borrowed[start - 1].trim_start().starts_with('@') {
        start -= 1;
    }
    let mut end = block_end(&borrowed, idx);
    // swallow one blank line after the method
    if end < borrowed.len() && borrowed[end].trim().is_empty() {
        end += 1;
    } else if start > 0 && borrowed[start - 1].trim().is_empty() {
        start -= 1;
    }
    let (mut lines, trailing) = to_owned_lines(src);
    let class_ind = indent_of(borrowed[idx]);
    lines.drain(start..end.min(lines.len()));
    // A class must not become empty.
    let class_header = (0..start).rev().find(|&i| {
        !is_blank_or_comment(&lines[i]) && indent_of(&lines[i]) < class_ind
    });
    if let Some(h) = class_header {
        let has_body = lines
            .iter()
            .skip(h + 1)
            .take_while(|l| is_blank_or_comment(l) || indent_of(l) >= class_ind)
            .any(|l| !is_blank_or_comment(l));
        if !has_body {
            lines.insert(h + 1, format!("{}pass", " ".repeat(class_ind)));
        }
    }
    Some((join(lines, trailing), start.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uml::parser::parse_project;

    const SRC: &str = "class Point:\n    def __init__(self, x: int):\n        self.x = x\n\n    def show(self):\n        print(self.x)\n\n\np = Point(1)\n";

    fn uml(src: &str) -> (Vec<String>, Vec<String>) {
        let c = &parse_project([("t.py", src)])[0];
        (
            c.attributes.iter().map(|a| a.display()).collect(),
            c.methods.iter().map(|m| m.display()).collect(),
        )
    }

    #[test]
    fn types() {
        assert_eq!(to_python_type("(int,int)"), "tuple[int, int]");
        assert_eq!(to_python_type("list[(int,str)]"), "list[tuple[int, str]]");
        assert_eq!(default_value("(int,float)"), "(0, 0.0)");
    }

    #[test]
    fn add_attr_and_param() {
        let (s, _) = add_attribute(SRC, 1, "y", Some("int"), true).unwrap();
        assert!(s.contains("def __init__(self, x: int, y: int):"));
        let (s, _) = add_attribute(&s, 1, "pos", Some("(int,int)"), false).unwrap();
        let (s, line) = add_attribute(&s, 1, "target", Some("Point"), false).unwrap();
        assert_eq!(s.lines().nth(line - 1).unwrap().trim(), "self.target: Point = None");
        let (a, _) = uml(&s);
        assert_eq!(a, ["x: int", "y: int", "pos: (int,int)", "target: Point"]);
        assert!(s.ends_with("p = Point(1)\n"));
    }

    #[test]
    fn add_methods() {
        let params = parse_param_list("p:(int,int), speed: float");
        let (s, line) = add_method(SRC, 1, "move", &params, Some("bool"), false).unwrap();
        assert!(s.lines().nth(line - 1).unwrap().contains("def move(self, p: tuple[int, int], speed: float) -> bool:"));
        let (_, m) = uml(&s);
        assert_eq!(m, ["__init__(x:int)", "show()", "move(p:(int,int), speed:float): bool"]);
        let (s2, _) = delete_method(&s, line).unwrap();
        assert_eq!(uml(&s2).1, ["__init__(x:int)", "show()"]);
    }

    #[test]
    fn constructor_in_empty_class() {
        let src = "class Empty:\n    pass\n";
        let (s, _) = add_method(src, 1, "__init__", &parse_param_list("a:int, b"), None, true).unwrap();
        assert_eq!(s, "class Empty:\n    def __init__(self, a: int, b):\n        self.a = a\n        self.b = b\n");
        let (s, _) = add_attribute("class E:\n    pass\n", 1, "n", Some("str"), false).unwrap();
        assert_eq!(uml(&s).0, ["n: str"]);
        let (s, l) = add_class("", "Game", None);
        assert_eq!(l, 1);
        let (s, _) = add_class(&s, "Player", Some("Game"));
        assert_eq!(parse_project([("t.py", s.as_str())]).len(), 2);
        let (s, _) = delete_method(&s, 2).unwrap();
        assert!(s.starts_with("class Game:\n    pass\n"));
    }
}
