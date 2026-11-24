// Embeds the app icon in the Windows .exe (Explorer, the taskbar, and Steam shortcuts that point
// at it). Regenerate the icon with `python assets/make_icon.py`.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("embedding the icon (needs the Windows SDK's rc.exe)");
    }
}
