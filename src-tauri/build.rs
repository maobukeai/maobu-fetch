fn main() {
    println!("cargo:rerun-if-changed=windows/app.manifest");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    // 图标（bundle.icon 的 icons/icon.ico）、版本资源（跟随 tauri.conf.json 的
    // version / productName，不再像 lumaget.rc 那样硬编码）由 tauri_build 统一嵌入。
    //
    // Windows manifest 不用 tauri_build 的 app_manifest() 内联机制：它把 XML 按行
    // 拼进 RC 字符串（每行首尾附加空格），SxS 激活上下文解析该产物会报
    // 14001"并行配置不正确"。这里改为通过 append_rc_content 让 RC 编译器直接
    // include manifest 文件，字节原样嵌入（保留 PerMonitorV2 DPI 感知）。
    // 必须用相对路径：本仓库路径含中文（"下载器"），rc.exe 命令行无法解析
    // 非 ASCII 绝对路径；RC 会以 build script 工作目录（crate 根）解析相对引用。
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest()
        .append_rc_content("1 24 \"windows/app.manifest\"");
    if let Err(error) =
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
    {
        // 构建失败必须让 cargo 感知（AGENTS.md §7：资源编译失败不得吞掉）。
        println!("cargo:error=资源/清单编译失败：{error:#}");
        std::process::exit(1);
    }
}
