fn main() {
    println!("cargo:rerun-if-changed=windows/lumaget.rc");
    println!("cargo:rerun-if-changed=windows/app.manifest");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    tauri_build::build();
    #[cfg(windows)]
    embed_resource::compile("windows/lumaget.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile the Maobu Fetch Windows icon and manifest");
}
