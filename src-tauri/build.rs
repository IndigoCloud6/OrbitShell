fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Tauri embeds resources only into application binaries. The lib test
        // harness also links rfd's TaskDialogIndirect and needs Common Controls 6.
        // Let the linker supply this one manifest to all targets, without also
        // embedding Tauri's equivalent manifest into the application resources.
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()))
            .expect("Unable to prepare Tauri Windows resources");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    } else {
        tauri_build::build();
    }
}
