//! Finding a Python interpreter and running student programs.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::config::SchoolConfig;

/// On Windows, never flash a console window for the child process.
#[cfg(windows)]
fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
fn hide_window(_cmd: &mut Command) {}

#[derive(Clone, Debug, PartialEq)]
pub struct PythonExec {
    pub command: String,
    pub pre_args: Vec<String>,
    /// Extra package folders (school config), put in front of `PYTHONPATH`.
    pub extra_path: Vec<PathBuf>,
}

impl PythonExec {
    fn new(command: impl Into<String>) -> Self {
        Self { command: command.into(), pre_args: Vec::new(), extra_path: Vec::new() }
    }

    fn with_extra_path(mut self, extra: &[PathBuf]) -> Self {
        self.extra_path = extra.to_vec();
        self
    }

    pub fn display(&self) -> String {
        if self.pre_args.is_empty() {
            self.command.clone()
        } else {
            format!("{} {}", self.command, self.pre_args.join(" "))
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.pre_args);
        cmd.env("PYGAME_HIDE_SUPPORT_PROMPT", "1");
        if !self.extra_path.is_empty() {
            let mut paths = self.extra_path.clone();
            if let Some(old) = std::env::var_os("PYTHONPATH") {
                paths.extend(std::env::split_paths(&old));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PYTHONPATH", joined);
            }
        }
        hide_window(&mut cmd);
        cmd
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// From the school's edupython.ini
    School,
    Bundled,
    Custom,
    Auto,
    NotFound,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::School => "set by your school",
            Source::Bundled => "bundled",
            Source::Custom => "chosen by you",
            Source::Auto => "found automatically",
            Source::NotFound => "not found",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PythonInfo {
    pub exec: PythonExec,
    pub source: Source,
    pub version: String,
    /// Packages to check (school config, default: pygame) and whether they are installed.
    pub packages: Vec<(String, bool)>,
}

impl PythonInfo {
    pub fn missing(&self) -> Vec<&str> {
        self.packages.iter().filter(|(_, ok)| !ok).map(|(p, _)| p.as_str()).collect()
    }

    /// One line per package, e.g. "pygame: installed".
    pub fn packages_summary(&self) -> String {
        self.packages
            .iter()
            .map(|(p, ok)| format!("{p}: {}", if *ok { "installed" } else { "not installed" }))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Returns the Python version (e.g. "Python 3.12.1") if `exec` is Python 3.
fn probe(exec: &PythonExec) -> Option<String> {
    let out = exec
        .command()
        .args(["-c", "import sys; print('ok', sys.version.split()[0], sys.version_info[0])"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut parts = s.split_whitespace();
    if parts.next()? != "ok" {
        return None;
    }
    let version = parts.next()?.to_string();
    (parts.next()? == "3").then(|| format!("Python {version}"))
}

/// Which of `packages` can be imported (one Python call for all of them).
pub fn check_packages(exec: &PythonExec, packages: &[String]) -> Vec<(String, bool)> {
    if packages.is_empty() {
        return Vec::new();
    }
    let script = "import sys, importlib.util\nfor n in sys.argv[1:]:\n    try:\n        ok = importlib.util.find_spec(n) is not None\n    except Exception:\n        ok = False\n    print(n, 'yes' if ok else 'no')\n";
    let out = exec.command().arg("-c").arg(script).args(packages).stdin(Stdio::null()).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    packages
        .iter()
        .map(|p| {
            let ok = text.lines().any(|l| l.trim() == format!("{p} yes"));
            (p.clone(), ok)
        })
        .collect()
}

fn bundled_candidates() -> Vec<PythonExec> {
    let embedded: Vec<PythonExec> =
        crate::runtime::embedded_python().map(|p| PythonExec::new(p.to_string_lossy())).into_iter().collect();
    if !embedded.is_empty() {
        return embedded;
    }
    let Ok(exe) = std::env::current_exe() else { return vec![] };
    let Some(dir) = exe.parent() else { return vec![] };
    // A `python-runtime` folder next to the program also works (portable folder).
    let mut roots: Vec<PathBuf> = vec![dir.join("python-runtime")];
    // macOS .app bundle: Contents/MacOS/edupython → Contents/Resources
    roots.push(dir.join("../Resources/python-runtime"));
    roots
        .into_iter()
        .map(|r| {
            if cfg!(windows) {
                r.join("python.exe")
            } else {
                r.join("bin").join("python3")
            }
        })
        .filter(|p| p.exists())
        .map(|p| PythonExec::new(p.to_string_lossy()))
        .collect()
}

fn system_candidates() -> Vec<PythonExec> {
    if cfg!(windows) {
        let mut v = vec![
            PythonExec { command: "py".into(), pre_args: vec!["-3".into()], extra_path: Vec::new() },
            PythonExec::new("python"),
            PythonExec::new("python3"),
        ];
        if let Some(local) = dirs::data_local_dir() {
            for ver in ["314", "313", "312", "311", "310"] {
                v.push(PythonExec::new(
                    local.join(format!("Programs/Python/Python{ver}/python.exe")).to_string_lossy(),
                ));
            }
        }
        v
    } else {
        [
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
            "/Library/Frameworks/Python.framework/Versions/Current/bin/python3",
            "python3",
            "/usr/bin/python3",
            "python",
        ]
        .into_iter()
        .map(PythonExec::new)
        .collect()
    }
}

/// Find an interpreter. Order:
/// 1. the school's edupython.ini paths (always first)
/// 2. if the ini allows it (or there is none): bundled → user choice → automatic search
pub fn detect(cfg: &SchoolConfig, custom: Option<&str>) -> PythonInfo {
    let extra = &cfg.packages_path;
    let mut found: Option<(PythonExec, Source, String)> = None;
    for p in &cfg.python_paths {
        let c = PythonExec::new(p).with_extra_path(extra);
        if let Some(v) = probe(&c) {
            found = Some((c, Source::School, v));
            break;
        }
    }
    let search = cfg.python_paths.is_empty() || cfg.fallback_search;
    if found.is_none() && search {
        for c in bundled_candidates() {
            let c = c.with_extra_path(extra);
            if let Some(v) = probe(&c) {
                found = Some((c, Source::Bundled, v));
                break;
            }
        }
    }
    if found.is_none() && search && cfg.allow_change {
        if let Some(cmd) = custom.filter(|c| !c.trim().is_empty()) {
            let c = PythonExec::new(cmd.trim()).with_extra_path(extra);
            if let Some(v) = probe(&c) {
                found = Some((c, Source::Custom, v));
            }
        }
    }
    if found.is_none() && search {
        for c in system_candidates() {
            let c = c.with_extra_path(extra);
            if let Some(v) = probe(&c) {
                found = Some((c, Source::Auto, v));
                break;
            }
        }
    }
    match found {
        Some((exec, source, version)) => {
            let packages = check_packages(&exec, &cfg.packages);
            PythonInfo { exec, source, version, packages }
        }
        None => PythonInfo {
            exec: PythonExec::new(if cfg!(windows) { "python" } else { "python3" }),
            source: Source::NotFound,
            version: "—".into(),
            packages: cfg.packages.iter().map(|p| (p.clone(), false)).collect(),
        },
    }
}

/// Check a user-entered interpreter path.
pub fn validate(command: &str) -> Result<String, String> {
    probe(&PythonExec::new(command.trim()))
        .ok_or_else(|| format!("'{}' is not a working Python 3 interpreter.", command.trim()))
}

/// Detect in the background so the window opens instantly.
pub fn detect_async(cfg: SchoolConfig, custom: Option<String>, ctx: eframe::egui::Context) -> Receiver<PythonInfo> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let info = detect(&cfg, custom.as_deref());
        let _ = tx.send(info);
        ctx.request_repaint();
    });
    rx
}

// ─────────────────────────────────────────────────────────────────────────────
// Running programs
// ─────────────────────────────────────────────────────────────────────────────

pub enum RunEvent {
    Stdout(String),
    Stderr(String),
    Exit(Option<i32>),
}

pub struct Runner {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Option<ChildStdin>,
    rx: Receiver<RunEvent>,
    finished: bool,
}

fn spawn_reader(mut src: impl Read + Send + 'static, tx: Sender<RunEvent>, ctx: eframe::egui::Context, err: bool) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match src.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    // Only emit complete UTF-8 sequences; keep a split char for later.
                    let valid = match std::str::from_utf8(&pending) {
                        Ok(_) => pending.len(),
                        Err(e) if e.error_len().is_none() => e.valid_up_to(),
                        Err(_) => pending.len(),
                    };
                    let chunk: Vec<u8> = pending.drain(..valid).collect();
                    let text = String::from_utf8_lossy(&chunk).into_owned();
                    let ev = if err { RunEvent::Stderr(text) } else { RunEvent::Stdout(text) };
                    if tx.send(ev).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            }
        }
        if !pending.is_empty() {
            let text = String::from_utf8_lossy(&pending).into_owned();
            let _ = tx.send(if err { RunEvent::Stderr(text) } else { RunEvent::Stdout(text) });
        }
    })
}

impl Runner {
    /// Run `python -u <args>` in `dir`.
    pub fn start(exec: &PythonExec, dir: &Path, args: &[&str], ctx: &eframe::egui::Context) -> Result<Runner, String> {
        let mut cmd = exec.command();
        cmd.arg("-u")
            .args(args)
            .current_dir(dir)
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("Could not start Python ({}): {e}", exec.display()))?;

        let (tx, rx) = channel();
        let stdin = child.stdin.take();
        let out = spawn_reader(child.stdout.take().unwrap(), tx.clone(), ctx.clone(), false);
        let err = spawn_reader(child.stderr.take().unwrap(), tx.clone(), ctx.clone(), true);

        let child = Arc::new(Mutex::new(Some(child)));
        let watch = Arc::clone(&child);
        let ctx = ctx.clone();
        thread::spawn(move || {
            let code = loop {
                thread::sleep(Duration::from_millis(40));
                let mut lock = watch.lock().unwrap();
                match lock.as_mut().map(|c| c.try_wait()) {
                    Some(Ok(Some(status))) => break status.code(),
                    Some(Ok(None)) => continue,
                    _ => break None,
                }
            };
            // Let the readers flush everything before reporting the exit.
            let _ = out.join();
            let _ = err.join();
            let _ = tx.send(RunEvent::Exit(code));
            ctx.request_repaint();
        });

        Ok(Runner { child, stdin, rx, finished: false })
    }

    pub fn poll(&mut self) -> Vec<RunEvent> {
        let events: Vec<RunEvent> = self.rx.try_iter().collect();
        if events.iter().any(|e| matches!(e, RunEvent::Exit(_))) {
            self.finished = true;
        }
        events
    }

    pub fn is_running(&self) -> bool {
        !self.finished
    }

    pub fn send_line(&mut self, line: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.write_all(line.as_bytes());
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        }
    }

    pub fn kill(&mut self) {
        self.stdin = None;
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        if !self.finished {
            self.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_with_input_and_error() {
        let info = detect(&SchoolConfig::default(), None);
        if info.source == Source::NotFound {
            return; // no Python on this machine
        }
        let dir = std::env::temp_dir().join(format!("edupython-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.py"), "n = input('Name? ')\nprint('Hi', n, 'é')\nraise ValueError('boom')\n").unwrap();
        let ctx = eframe::egui::Context::default();
        let mut r = Runner::start(&info.exec, &dir, &["main.py"], &ctx).unwrap();
        r.send_line("Bob");
        let (mut out, mut err, mut code) = (String::new(), String::new(), None);
        for _ in 0..250 {
            for ev in r.poll() {
                match ev {
                    RunEvent::Stdout(s) => out.push_str(&s),
                    RunEvent::Stderr(s) => err.push_str(&s),
                    RunEvent::Exit(c) => code = Some(c),
                }
            }
            if code.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(out, "Name? Hi Bob é\n");
        assert!(err.contains("ValueError: boom"), "{err}");
        assert!(err.contains("line 3"));
        assert_eq!(code, Some(Some(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_stops_program() {
        let info = detect(&SchoolConfig::default(), None);
        if info.source == Source::NotFound {
            return;
        }
        let dir = std::env::temp_dir();
        let ctx = eframe::egui::Context::default();
        let mut r = Runner::start(&info.exec, &dir, &["-c", "while True: pass"], &ctx).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(r.is_running());
        r.kill();
        for _ in 0..100 {
            r.poll();
            if !r.is_running() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("program was not stopped");
    }

    #[test]
    fn school_paths_come_first_and_packages_are_checked() {
        let auto = detect(&SchoolConfig::default(), None);
        if auto.source == Source::NotFound {
            return;
        }
        let cfg = SchoolConfig {
            source: Some(PathBuf::from("edupython.ini")),
            python_paths: vec!["/no/such/python".into(), auto.exec.command.clone()],
            fallback_search: false,
            packages: vec!["json".into(), "no_such_module_xyz".into()],
            ..SchoolConfig::default()
        };
        let info = detect(&cfg, Some("/also/not/python"));
        assert_eq!(info.source, Source::School);
        assert_eq!(info.packages, [("json".to_string(), true), ("no_such_module_xyz".to_string(), false)]);
        assert_eq!(info.missing(), ["no_such_module_xyz"]);

        // Nothing from the ini works and searching is off → not found.
        let cfg = SchoolConfig { python_paths: vec!["/no/such/python".into()], ..cfg };
        assert_eq!(detect(&cfg, None).source, Source::NotFound);
    }
}
