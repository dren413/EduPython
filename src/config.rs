//! School configuration: an optional `edupython.ini` installed by IT.
//!
//! EduPython looks for it (first match wins) in:
//! 0. the file given on the command line: `EduPython.exe --config <file>`
//!    (handy in a Start-menu shortcut, so the ini can live anywhere)
//! 1. the file named by the environment variable `EDUPYTHON_CONFIG`
//! 2. the folder of the program (`EduPython.exe`)
//! 3. macOS: the folder that contains `EduPython.app`, then `Contents/Resources`
//! 4. a system folder: `%ProgramData%\EduPython` (Windows) or
//!    `/Library/Application Support/EduPython` (macOS)
//! 5. the user's own ini in the app-data folder (`%LOCALAPPDATA%\EduPython`,
//!    `~/Library/Application Support/EduPython`, `~/.local/share/EduPython`).
//!    EduPython creates it on first start with open defaults (pupils may pick
//!    Python and install packages), so at home no ini has to be set up, and
//!    any school ini above overrides it.
//!
//! Relative paths inside the ini are relative to the ini's own folder, and
//! `%VAR%` / `$VAR` / `~` are expanded. See `packaging/edupython.example.ini`.
//!
//! Without an ini, a `booklet.pdf` in one of these folders is still shown.

use std::path::{Path, PathBuf};

pub const INI_NAME: &str = "edupython.ini";
pub const DEFAULT_BOOKLET: &str = "booklet.pdf";
/// Where pupils can download the booklet when none is installed.
pub const BOOKLET_URL: &str = "https://edupython.lu/Python%20reference.pdf";
/// Tried in this order when EduPython downloads the booklet itself.
pub const BOOKLET_URLS: [&str; 2] = [BOOKLET_URL, "https://www.gashi.lu/edupython/Python%20reference.pdf"];

#[derive(Clone, Debug, PartialEq)]
pub struct SchoolConfig {
    /// The ini file that was loaded (`None` = no ini, built-in defaults).
    pub source: Option<PathBuf>,
    /// Interpreters to try first, in order.
    pub python_paths: Vec<String>,
    /// Search the computer for Python when none of `python_paths` work.
    pub fallback_search: bool,
    /// Pupils may pick another interpreter in the app.
    pub allow_change: bool,
    /// Pupils may install packages with pip from the app.
    pub allow_install: bool,
    /// Extra folders added to `PYTHONPATH`.
    pub packages_path: Vec<PathBuf>,
    /// Packages that should be importable (checked and shown in the app).
    pub packages: Vec<String>,
    /// The booklet PDF, if one was found.
    pub booklet: Option<PathBuf>,
    /// Name shown for the booklet.
    pub booklet_title: String,
    /// Show file paths (interpreter, package folders, the ini) to pupils.
    pub show_paths: bool,
    /// Folder offered for new projects and in "Open Project" (e.g. `H:\Documents`).
    pub projects_folder: Option<PathBuf>,
}

impl Default for SchoolConfig {
    fn default() -> Self {
        Self {
            source: None,
            python_paths: Vec::new(),
            fallback_search: true,
            allow_change: true,
            allow_install: true,
            packages_path: Vec::new(),
            packages: vec!["pygame".into()],
            booklet: None,
            booklet_title: "Booklet".into(),
            show_paths: true,
            projects_folder: None,
        }
    }
}

impl SchoolConfig {
    /// Find and load the configuration for this computer. `explicit` is the
    /// ini given with `--config` on the command line (tried first).
    pub fn load(explicit: Option<&Path>) -> Self {
        ensure_user_ini();
        let dirs = search_dirs();
        let explicit = explicit.map(|p| PathBuf::from(expand_vars(&p.to_string_lossy()))).map(|p| if p.is_dir() { p.join(INI_NAME) } else { p });
        let env_ini = std::env::var_os("EDUPYTHON_CONFIG").map(PathBuf::from);
        let ini = explicit
            .into_iter()
            .chain(env_ini)
            .find(|p| p.is_file())
            .or_else(|| dirs.iter().map(|d| d.join(INI_NAME)).find(|p| p.is_file()));
        let mut cfg = match &ini {
            Some(path) => match std::fs::read_to_string(path) {
                Ok(text) => Self::parse(&text, path),
                Err(_) => Self::default(),
            },
            None => Self::default(),
        };
        if cfg.booklet.is_none() {
            cfg.booklet = dirs.iter().map(|d| d.join(DEFAULT_BOOKLET)).find(|p| p.is_file());
        }
        cfg
    }

    /// Parse ini text. `path` is the ini file (relative paths are resolved
    /// against its folder). Unknown keys are ignored.
    pub fn parse(text: &str, path: &Path) -> Self {
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut cfg = Self {
            source: Some(path.to_path_buf()),
            // Managed computers are locked down unless the ini says otherwise.
            allow_change: false,
            allow_install: false,
            ..Self::default()
        };
        let mut packages_set = false;
        let mut section = String::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_ascii_lowercase();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim().to_ascii_lowercase();
            let value = strip_quotes(value.trim());
            match (section.as_str(), key.as_str()) {
                ("python", k) if k == "path" || k.starts_with("path") && k[4..].chars().all(|c| c.is_ascii_digit()) => {
                    if !value.is_empty() {
                        cfg.python_paths.push(resolve_command(value, &base));
                    }
                }
                ("python", "fallback_search") => cfg.fallback_search = parse_bool(value, cfg.fallback_search),
                ("python", "allow_change") => cfg.allow_change = parse_bool(value, cfg.allow_change),
                ("python", "allow_install") => cfg.allow_install = parse_bool(value, cfg.allow_install),
                ("python", "packages_path") => {
                    cfg.packages_path.extend(split_list(value, ';').map(|p| resolve_path(p, &base)));
                }
                ("python", "packages") => {
                    if !packages_set {
                        cfg.packages.clear();
                        packages_set = true;
                    }
                    for p in split_list(value, ',') {
                        if !cfg.packages.iter().any(|x| x == p) {
                            cfg.packages.push(p.to_string());
                        }
                    }
                }
                ("booklet", "file") if !value.is_empty() => {
                    let p = resolve_path(value, &base);
                    cfg.booklet = p.is_file().then_some(p);
                }
                ("booklet", "title") if !value.is_empty() => cfg.booklet_title = value.to_string(),
                ("display", "show_paths") => cfg.show_paths = parse_bool(value, cfg.show_paths),
                ("projects", "folder") if !value.is_empty() => cfg.projects_folder = Some(resolve_path(value, &base)),
                _ => {}
            }
        }
        cfg
    }
}

fn strip_quotes(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' && b[b.len() - 1] == b'"' || b[0] == b'\'' && b[b.len() - 1] == b'\'') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn split_list(s: &str, sep: char) -> impl Iterator<Item = &str> {
    s.split(sep).map(|x| strip_quotes(x.trim())).filter(|x| !x.is_empty())
}

fn parse_bool(s: &str, default: bool) -> bool {
    match s.to_ascii_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => true,
        "0" | "no" | "false" | "off" => false,
        _ => default,
    }
}

/// Expand `%VAR%`, `$VAR`, `${VAR}` and a leading `~`.
pub fn expand_vars(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '%' {
            if let Some(end) = chars[i + 1..].iter().position(|&x| x == '%') {
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                if !name.is_empty() && name.chars().all(|x| x.is_alphanumeric() || x == '_' || x == '(' || x == ')') {
                    if let Ok(v) = std::env::var(&name) {
                        out.push_str(&v);
                        i += end + 2;
                        continue;
                    }
                }
            }
        } else if c == '$' {
            let (start, braced) = if chars.get(i + 1) == Some(&'{') { (i + 2, true) } else { (i + 1, false) };
            let len = chars[start..].iter().take_while(|x| x.is_alphanumeric() || **x == '_').count();
            if len > 0 && (!braced || chars.get(start + len) == Some(&'}')) {
                let name: String = chars[start..start + len].iter().collect();
                if let Ok(v) = std::env::var(&name) {
                    out.push_str(&v);
                    i = start + len + usize::from(braced);
                    continue;
                }
            }
        } else if c == '~' && i == 0 && matches!(chars.get(1), None | Some('/') | Some('\\')) {
            if let Some(home) = dirs::home_dir() {
                out.push_str(&home.to_string_lossy());
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn resolve_path(value: &str, base: &Path) -> PathBuf {
    let p = PathBuf::from(expand_vars(value));
    if p.is_absolute() || p.starts_with("\\\\") { p } else { base.join(p) }
}

/// Python entries may be a path (`C:\Python313\python.exe`, `../python/python.exe`)
/// or a bare command name (`python3`, `py`) that is looked up on PATH.
fn resolve_command(value: &str, base: &Path) -> String {
    let expanded = expand_vars(value);
    if expanded.contains('/') || expanded.contains('\\') {
        resolve_path(&expanded, base).to_string_lossy().into_owned()
    } else {
        expanded
    }
}

/// Installed from the Microsoft Store (MSIX): the program folder is then
/// read-only, so IT uses `%ProgramData%\EduPython` instead.
pub fn is_store_app() -> bool {
    cfg!(windows)
        && std::env::current_exe()
            .map(|e| e.to_string_lossy().to_ascii_lowercase().contains("\\windowsapps\\"))
            .unwrap_or(false)
}

/// The folder to suggest for `booklet.pdf`: next to `EduPython.exe`
/// (Windows, Linux) or next to `EduPython.app` (macOS); for the Store
/// version `%ProgramData%\EduPython`, which is searched as well.
pub fn program_folder() -> Option<PathBuf> {
    if is_store_app() {
        return std::env::var_os("ProgramData").map(|pd| PathBuf::from(pd).join("EduPython"));
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    if cfg!(target_os = "macos") && dir.ends_with("Contents/MacOS") {
        return dir.parent()?.parent()?.parent().map(Path::to_path_buf);
    }
    Some(dir)
}

/// The user's own ini (lowest priority), in the local app-data folder.
pub fn user_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("EduPython"))
}

/// Settings in the user's own ini, written on first start. Every value is the
/// open default, so it behaves exactly like having no ini at all.
const USER_INI: &str = r#"; EduPython settings for this user (created automatically).
;
; This file is used only when there is no other edupython.ini: one next to
; EduPython, in the system folder, or given with  --config <file>  wins.
; Edit it to change the defaults at home. Lines starting with ; are comments.
; All options: https://github.com/dren413/EduPython#school-setup-it-booklet-and-fixed-python

[python]
; Python interpreters to try first (one "path =" per line), e.g.
; path = C:\Python313\python.exe
fallback_search = yes
allow_change = yes
allow_install = yes
packages = pygame

[booklet]
; Leave empty to use the booklet built into EduPython.
; file = C:\Users\me\Documents\Python reference.pdf
; title = Python Reference Guide

[display]
show_paths = yes

[projects]
; folder = ~/Documents
"#;

/// Create the user's ini with the open defaults if it does not exist yet.
fn ensure_user_ini() {
    let Some(dir) = user_dir() else { return };
    let path = dir.join(INI_NAME);
    if !path.exists() && std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(path, USER_INI);
    }
}

/// Folders searched for the ini and the default booklet, most specific first.
fn search_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        // macOS: …/EduPython.app/Contents/MacOS/edupython
        if cfg!(target_os = "macos") && dir.ends_with("Contents/MacOS") {
            if let Some(app) = dir.parent().and_then(Path::parent) {
                if let Some(outside) = app.parent() {
                    v.push(outside.to_path_buf());
                }
            }
            v.push(dir.join("../Resources"));
        }
        v.insert(0, dir);
    }
    if cfg!(windows) {
        if let Some(pd) = std::env::var_os("ProgramData") {
            v.push(PathBuf::from(pd).join("EduPython"));
        }
    } else if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/Library/Application Support/EduPython"));
    } else {
        v.push(PathBuf::from("/etc/edupython"));
    }
    v.extend(user_dir());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ini() {
        let dir = std::env::temp_dir().join(format!("edupython-ini-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ref.pdf"), b"%PDF").unwrap();
        let text = r#"
; comment
[Python]
path = python/python.exe
path2 = "py"
fallback_search = no
packages = pygame, numpy
packages = pygame
packages_path = libs ; "shared libs"
[booklet]
file = ref.pdf
title = Python Reference 2.5.4
[display]
show_paths = no
[projects]
folder = "%EDUPYTHON_TEST_HOME%\Documents"
"#;
        // SAFETY: only this test reads this variable.
        unsafe { std::env::set_var("EDUPYTHON_TEST_HOME", "H:") };
        let cfg = SchoolConfig::parse(text, &dir.join(INI_NAME));
        assert!(cfg.source.is_some());
        assert_eq!(cfg.python_paths, [dir.join("python/python.exe").to_string_lossy().into_owned(), "py".to_string()]);
        assert!(!cfg.fallback_search);
        assert!(!cfg.allow_change, "managed defaults to locked");
        assert!(!cfg.allow_install);
        assert_eq!(cfg.packages, ["pygame", "numpy"]);
        assert_eq!(cfg.packages_path, [dir.join("libs"), dir.join("shared libs")]);
        assert_eq!(cfg.booklet, Some(dir.join("ref.pdf")));
        assert_eq!(cfg.booklet_title, "Python Reference 2.5.4");
        assert!(!cfg.show_paths);
        // `H:\Documents` is absolute on Windows only; elsewhere it is joined to the ini folder.
        let expected = if cfg!(windows) { PathBuf::from("H:\\Documents") } else { dir.join("H:\\Documents") };
        assert_eq!(cfg.projects_folder, Some(expected));

        let cfg = SchoolConfig::parse("[python]\nallow_change = yes\n[booklet]\nfile = missing.pdf\n", &dir.join(INI_NAME));
        assert!(cfg.allow_change);
        assert_eq!(cfg.packages, ["pygame"]);
        assert_eq!(cfg.booklet, None);
        assert!(cfg.show_paths);
        assert_eq!(cfg.projects_folder, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_ini_is_open() {
        let cfg = SchoolConfig::parse(USER_INI, Path::new("/x/edupython.ini"));
        assert_eq!(SchoolConfig { source: None, ..cfg }, SchoolConfig::default(), "the user ini must behave like no ini");
    }

    #[test]
    fn expands_variables() {
        // SAFETY: tests in this module do not read this variable concurrently.
        unsafe { std::env::set_var("EDUPYTHON_TEST_DIR", "/x/y") };
        assert_eq!(expand_vars("%EDUPYTHON_TEST_DIR%\\python.exe"), "/x/y\\python.exe");
        assert_eq!(expand_vars("$EDUPYTHON_TEST_DIR/bin"), "/x/y/bin");
        assert_eq!(expand_vars("${EDUPYTHON_TEST_DIR}/bin"), "/x/y/bin");
        assert_eq!(expand_vars("100%"), "100%");
        assert_eq!(expand_vars("%NOT_SET_EDUPYTHON%"), "%NOT_SET_EDUPYTHON%");
    }
}
