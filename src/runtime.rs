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

/// File name of the built-in booklet (as pupils see it in the app-data folder).
pub const BUNDLED_BOOKLET_NAME: &str = "Python reference.pdf";
/// Title shown for the built-in booklet.
pub const BUNDLED_BOOKLET_TITLE: &str = "Python Reference Guide";

/// The booklet that ships inside EduPython, so there is always one that works.
/// It is copied once (and again after an update that changes it) to
/// `%LOCALAPPDATA%\EduPython\booklet\` (macOS: `~/Library/Application Support/EduPython/booklet/`,
/// Linux: `~/.local/share/EduPython/booklet/`). Returns its path.
pub fn bundled_booklet() -> Option<PathBuf> {
    static PDF: &[u8] = include_bytes!("../assets/Python reference.pdf");

    let dir = dirs::data_local_dir()?.join("EduPython").join("booklet");
    let path = dir.join(BUNDLED_BOOKLET_NAME);
    let up_to_date = std::fs::read(&path).is_ok_and(|b| b == PDF);
    if !up_to_date {
        std::fs::create_dir_all(&dir).ok()?;
        // Write to a temporary file first, so a half-written PDF is never shown.
        let tmp = dir.join(format!(".booklet-{}.tmp", std::process::id()));
        std::fs::write(&tmp, PDF).ok()?;
        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
            // Another EduPython may hold the file open (Windows); use it if it is complete.
            if !std::fs::read(&path).is_ok_and(|b| b == PDF) {
                return None;
            }
        }
    }
    Some(path)
}
