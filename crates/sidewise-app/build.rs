fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/app.ico");
        res.set("ProductName", "Sidewise");
        res.set("FileDescription", "Sidewise Touchpad Edge Gestures");
        res.set("LegalCopyright", "Copyright (C) 2026 Sidewise");
        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to compile windows resource: {}", e);
        }
    }
}
