fn main() {
    println!("cargo:rerun-if-env-changed=EDUPYTHON_PYTHON_ZIP");
    if let Ok(zip) = std::env::var("EDUPYTHON_PYTHON_ZIP") {
        println!("cargo:rerun-if-changed={zip}");
    }
    // Check the *target*, not the machine we build on, so a Windows exe built
    // on a Mac (x86_64-pc-windows-gnu) also gets its icon.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        // Absolute path: windres runs in OUT_DIR and would not find a relative one.
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon.ico");
        res.set_icon(&icon.to_string_lossy());
        res.set("ProductName", "EduPython");
        res.set("FileDescription", "EduPython");
        // Cross-compiling from macOS/Linux: use the MinGW tools (brew install mingw-w64).
        if !cfg!(windows) {
            res.set_windres_path("x86_64-w64-mingw32-windres");
            res.set_ar_path("x86_64-w64-mingw32-ar");
        }
        if let Err(e) = res.compile() {
            println!("cargo:warning=Windows icon not embedded: {e}");
        }
    }
}
