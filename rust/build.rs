fn main() {
    println!("cargo:rerun-if-env-changed=PYTHONIZER_PYTHON_ZIP");
    if let Ok(zip) = std::env::var("PYTHONIZER_PYTHON_ZIP") {
        println!("cargo:rerun-if-changed={zip}");
    }
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Pythonizer");
        res.set("FileDescription", "Pythonizer");
        let _ = res.compile();
    }
}
