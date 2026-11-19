//! Optional built-in Python runtime.
//!
//! When built with `--features bundled-python` and the environment variable
//! `PYTHONIZER_PYTHON_ZIP` pointing to a zipped Python (with pygame), the zip
//! is embedded into the executable. On first start it is unpacked once into
//! the user's local data folder, so pupils need to install nothing.

use std::path::PathBuf;

/// Path of the built-in interpreter, unpacking it on first use.
/// Returns `None` in builds without an embedded runtime.
#[cfg(feature = "bundled-python")]
pub fn embedded_python() -> Option<PathBuf> {
    use std::io::Cursor;

    static ZIP: &[u8] = include_bytes!(env!("PYTHONIZER_PYTHON_ZIP"));

    let base = dirs::data_local_dir()?.join("Pythonizer");
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
