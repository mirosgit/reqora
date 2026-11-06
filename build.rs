//! Build script: on Windows, embeds the app icon and version info into `reqora.exe`
//! so Explorer, the taskbar and the Start menu show them.

fn main() {
    println!("cargo:rerun-if-changed=assets/reqora.ico");
    #[cfg(windows)]
    embed_windows_resources();
}

#[cfg(windows)]
fn embed_windows_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/reqora.ico");
    // Task Manager shows FileDescription as the process name.
    res.set("FileDescription", "Reqora");
    res.set("ProductName", "Reqora");
    if let Err(e) = res.compile() {
        // A missing resource compiler only costs the icon; the app still builds.
        println!("cargo:warning=could not embed the Windows icon: {e}");
    }
}
