fn main() {
    println!("cargo:rerun-if-env-changed=EDUPYTHON_PYTHON_ZIP");
    if let Ok(zip) = std::env::var("EDUPYTHON_PYTHON_ZIP") {
        println!("cargo:rerun-if-changed={zip}");
    }
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "EduPython");
        res.set("FileDescription", "EduPython");
        let _ = res.compile();
    }
}
