fn main() {
    // lib 单测实际构造 Wry 时同样需要 Common Controls v6。
    // 仅主程序嵌资源不会覆盖 test harness，缺失会在测试入口前报 0xc0000139。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:windows/app.manifest");
    }
    println!("cargo:rerun-if-changed=windows/app.manifest");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    // 图标和版本资源仍由 tauri_build 编译；Manifest 统一由链接器嵌入。
    // RC 内联 XML 会改变字节引起 SxS 14001；仅对 bin 的资源指令又覆盖不到
    // lib --test harness。通用 link-arg 使用相对输入路径，避免中文绝对路径问题。
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    if let Err(error) =
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
    {
        // 构建失败必须让 cargo 感知（AGENTS.md §7：资源编译失败不得吞掉）。
        println!("cargo:error=资源/清单编译失败：{error:#}");
        std::process::exit(1);
    }
}
