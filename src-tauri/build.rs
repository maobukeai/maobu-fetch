fn main() {
    tauri_build::build();
    #[cfg(windows)]
    let _ = embed_resource::compile("windows/lumaget.rc", embed_resource::NONE);
}
