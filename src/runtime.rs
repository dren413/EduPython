//! Optional built-in Python runtime.
//!
//! When built with `--features bundled-python` and the environment variable
//! `EDUPYTHON_PYTHON_ZIP` pointing to a zipped Python (with pygame), the zip
//! is embedded into the executable. On first start it is unpacked once into
//! the user's local data folder, so pupils need to install nothing.

use std::path::PathBuf;

/// Path of the built-in interpreter, unpacking it on first use.
/// Returns `None` in builds without an embedded runtime.
#[cfg(feature = "bundled-python")]
pub fn embedded_python() -> Option<PathBuf> {
    use std::io::Cursor;

    static ZIP: &[u8] = include_bytes!(env!("EDUPYTHON_PYTHON_ZIP"));

    let base = dirs::data_local_dir()?.join("EduPython");
    let dir = base.join(format!("python-{}-{}", env!("CARGO_PKG_VERSION"), ZIP.len()));
    let exe = if cfg!(windows) { dir.join("python.exe") } else { dir.join("bin").join("python3") };
    let marker = dir.join(".complete");
    if marker.exists() && exe.exists() {
        return Some(exe);
    }

    // Remove older unpacked runtimes and half-finished attempts.
    if let Ok(entries) = std::fs::read_dir(&base) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("python-") {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let tmp = base.join(format!("unpacking-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).ok()?;
    let mut archive = zip::ZipArchive::new(Cursor::new(ZIP)).ok()?;
    archive.extract(&tmp).ok()?;
    std::fs::rename(&tmp, &dir).ok()?;
    std::fs::write(&marker, b"ok").ok()?;
    exe.exists().then_some(exe)
}

#[cfg(not(feature = "bundled-python"))]
pub fn embedded_python() -> Option<PathBuf> {
    None
}

/// File name of the booklet that EduPython downloads (as pupils see it in the app-data folder).
pub const BUNDLED_BOOKLET_NAME: &str = "Python reference.pdf";
/// Title shown for the downloaded booklet.
pub const BUNDLED_BOOKLET_TITLE: &str = "Python Reference Guide";

/// Where the downloaded booklet is kept: `%LOCALAPPDATA%\EduPython\booklet\`
/// (macOS: `~/Library/Application Support/EduPython/booklet/`,
/// Linux: `~/.local/share/EduPython/booklet/`).
fn booklet_cache() -> Option<PathBuf> {
    Some(booklet_dir()?.join(BUNDLED_BOOKLET_NAME))
}

/// Folder that holds the downloaded booklet and PDFs the pupil added.
pub fn booklet_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("EduPython").join("booklet"))
}

/// Copy a PDF the pupil chose (or dropped on the booklet panel) into the
/// app-data folder, so it keeps working when the original is moved or deleted.
pub fn adopt_booklet(src: &std::path::Path) -> Result<PathBuf, String> {
    if !is_pdf(src) {
        return Err("This file is not a PDF.".into());
    }
    let dir = booklet_dir().ok_or("No app-data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = src.file_name().ok_or("No file name")?;
    let dest = dir.join(name);
    if src != dest {
        std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
    }
    Ok(dest)
}

fn is_pdf(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 5];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head == b"%PDF-"
}

/// The booklet downloaded earlier, if there is one.
///
/// The booklet is not built into the program (it is not EduPython's own
/// work, and the signed exe may only contain open-source code), so EduPython
/// downloads it once from edupython.lu and keeps that copy, which then also
/// works offline.
pub fn cached_booklet() -> Option<PathBuf> {
    booklet_cache().filter(|p| is_pdf(p))
}

/// Download the booklet in the background (with the `curl` that comes with
/// Windows 10+, macOS and Linux) into the app-data folder. No personal data
/// is sent. The result arrives on the channel; the UI is repainted then.
pub fn download_booklet(ctx: eframe::egui::Context) -> std::sync::mpsc::Receiver<Result<PathBuf, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let path = booklet_cache().ok_or("No app-data folder")?;
            let dir = path.parent().ok_or("No app-data folder")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let tmp = dir.join(format!(".download-{}.tmp", std::process::id()));
            // The main site first, then the mirror.
            for url in crate::config::BOOKLET_URLS {
                let mut cmd = std::process::Command::new("curl");
                cmd.args(["-fsSL", "--retry", "1", "--connect-timeout", "15", "--max-time", "120", "-o"]).arg(&tmp).arg(url);
                crate::python::hide_window(&mut cmd);
                let Ok(out) = cmd.output() else { break };
                if out.status.success() && is_pdf(&tmp) {
                    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
                    return Ok(path);
                }
                let _ = std::fs::remove_file(&tmp);
            }
            // curl's own messages mean little to pupils.
            Err("Could not download it. Is the computer online?".to_string())
        })();
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}
