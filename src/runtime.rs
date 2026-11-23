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
    Some(dirs::data_local_dir()?.join("EduPython").join("booklet").join(BUNDLED_BOOKLET_NAME))
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
            let mut cmd = std::process::Command::new("curl");
            cmd.args(["-fsSL", "--retry", "2", "--max-time", "120", "-o"]).arg(&tmp).arg(crate::config::BOOKLET_URL);
            crate::python::hide_window(&mut cmd);
            let out = cmd.output().map_err(|e| format!("Could not start the download: {e}"))?;
            if !out.status.success() || !is_pdf(&tmp) {
                let _ = std::fs::remove_file(&tmp);
                let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(if msg.is_empty() { "The download failed. Is the computer online?".to_string() } else { msg });
            }
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            Ok(path)
        })();
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}
