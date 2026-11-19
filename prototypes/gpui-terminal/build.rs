use std::{env, fs, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let icon = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/orbitshell.ico");
    let resource = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR"))
        .join("orbitshell-icon.rc");
    println!("cargo:rerun-if-changed={}", icon.display());

    // GPUI loads resource 1 as the icon for its Windows window class.
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    fs::write(
        &resource,
        format!("#pragma code_page(65001)\n1 ICON \"{icon_path}\"\n"),
    )
    .expect("failed to write OrbitShell's Windows icon resource");
    embed_resource::compile_for(resource, ["orbitshell-gpui-terminal"], embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed OrbitShell's Windows icon");
}
