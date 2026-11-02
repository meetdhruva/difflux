//! Embeds the app icon and version information into Windows executables,
//! so Explorer, the taskbar and "Properties → Details" show difflux properly.

#![allow(clippy::print_stdout)] // build scripts talk to cargo through stdout

fn main() {
    println!("cargo::rerun-if-changed=../../assets/icon/difflux.ico");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=DIFFLUX_REQUIRE_WINDOWS_RESOURCES");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon/difflux.ico")
            .set("ProductName", "difflux")
            .set("FileDescription", "difflux - file and folder compare")
            .set("CompanyName", "difflux contributors")
            .set("LegalCopyright", "MIT OR Apache-2.0")
            .set("OriginalFilename", "difflux.exe");
        if let Err(e) = res.compile() {
            // A missing resource compiler must not break source builds, but release builds
            // set DIFFLUX_REQUIRE_WINDOWS_RESOURCES so they never ship without an icon.
            assert!(std::env::var_os("DIFFLUX_REQUIRE_WINDOWS_RESOURCES").is_none(), "could not embed Windows icon and version info: {e}");
            println!("cargo::warning=could not embed Windows icon and version info: {e}");
        }
    }
}
