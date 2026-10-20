// Embeds the burst logo as the exe icon, plus version information, on Windows.
fn main() {
    println!("cargo:rerun-if-changed=assets/logo/bestofburst.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/logo/bestofburst.ico")
            .set("ProductName", "BestOfBurst")
            .set("FileDescription", "BestOfBurst photo culler")
            .set("CompanyName", "Ihan Hansaja")
            .set("LegalCopyright", "Copyright (c) 2026 Ihan Hansaja. MIT License.");
        if let Err(e) = res.compile() {
            // A missing resource compiler only costs the icon; the build still succeeds.
            println!("cargo:warning=could not embed the exe icon: {e}");
        }
    }
}
