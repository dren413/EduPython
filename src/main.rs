#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod booklet;
mod config;
mod console;
mod editor;
mod highlight;
mod icons;
mod project;
mod print;
mod python;
mod runtime;
mod settings;
mod uml;

use std::ffi::OsString;
use std::path::PathBuf;

fn main() -> eframe::Result {
    install_crash_log();
    let icon = eframe::icon_data::from_png_bytes(app::ICON_PNG).ok();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("EduPython")
        // Linux: matches StartupWMClass in the .desktop file (icon in the dock).
        .with_app_id("edupython")
        .with_inner_size([1360.0, 860.0])
        .with_min_inner_size([800.0, 500.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let args = Args::parse(std::env::args_os().skip(1));
    eframe::run_native("EduPython", options, Box::new(move |cc| Ok(Box::new(app::EduPythonApp::new(cc, args.project, args.config)))))
}

/// Release builds have no console on Windows, so a panic would vanish
/// silently. Write it to `%LOCALAPPDATA%\\EduPython\\crash.log` (macOS:
/// `~/Library/Application Support/EduPython/crash.log`) and tell the pupil.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let msg = format!(
            "EduPython {} crashed ({}):\n{info}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::backtrace::Backtrace::force_capture()
        );
        let path = dirs::data_local_dir().map(|d| d.join("EduPython").join("crash.log"));
        let written = path.as_ref().is_some_and(|p| {
            let _ = std::fs::create_dir_all(p.parent().unwrap_or(p));
            std::fs::write(p, &msg).is_ok()
        });
        let where_ = match (&path, written) {
            (Some(p), true) => format!("\n\nDetails were saved to:\n{}", p.display()),
            _ => String::new(),
        };
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("EduPython crashed")
            .set_description(format!("Sorry, EduPython hit an error and has to close.\n\n{info}{where_}"))
            .show();
    }));
}

/// Command line: `EduPython.exe [--config <edupython.ini>] [project folder]`.
/// `--config=<file>` also works. Useful in a Start-menu shortcut, so the exe
/// can be replaced on update while the ini stays where IT put it.
#[derive(Debug, Default, PartialEq)]
struct Args {
    project: Option<PathBuf>,
    config: Option<PathBuf>,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Self {
        let mut out = Self::default();
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            let s = a.to_string_lossy();
            if let Some(v) = ["--config=", "-config=", "/config="].iter().find_map(|p| s.strip_prefix(p)) {
                out.config = Some(PathBuf::from(v));
            } else if matches!(s.as_ref(), "--config" | "-config" | "/config" | "-c") {
                out.config = it.next().map(PathBuf::from);
            } else if out.project.is_none() {
                out.project = Some(PathBuf::from(a));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Args {
        Args::parse(v.iter().map(OsString::from))
    }

    #[test]
    fn command_line() {
        assert_eq!(parse(&[]), Args::default());
        assert_eq!(parse(&["proj"]), Args { project: Some("proj".into()), config: None });
        assert_eq!(parse(&["--config", "S:\\a.ini", "proj"]), Args { project: Some("proj".into()), config: Some("S:\\a.ini".into()) });
        assert_eq!(parse(&["proj", "--config=x.ini"]), Args { project: Some("proj".into()), config: Some("x.ini".into()) });
        assert_eq!(parse(&["/config", "x.ini"]).config, Some("x.ini".into()));
    }
}
