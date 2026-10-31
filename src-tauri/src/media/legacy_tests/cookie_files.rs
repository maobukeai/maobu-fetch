use super::*;
/// 验证 yt-dlp 子进程命令行不包含 Cookie 原文。
///
/// 这是 SubTask 4.6 的核心安全测试：通过启动一个真实子进程
/// （Windows 用 cmd.exe /c echo，Unix 用 /bin/echo）打印其命令行参数，
/// 然后断言：
/// - Cookie 原文（如 `secret_value_abc123`）不出现在命令行
/// - `--cookies` 标志出现在命令行（指向临时文件）
/// - 不存在 `--add-header Cookie:` 模式
#[tokio::test]
async fn attach_auth_args_does_not_leak_cookie_to_command_line() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base_dir = temp_dir.path().to_path_buf();

    // 选择平台对应的 echo 可执行文件
    #[cfg(windows)]
    let mut command = Command::new("cmd.exe");
    #[cfg(windows)]
    {
        command.args(["/c", "echo"]);
    }
    #[cfg(not(windows))]
    let mut command = Command::new("/bin/echo");

    let secret_cookie = "session=secret_value_abc123; token=sensitive_xyz";
    let guard = attach_auth_args_in_dir(
        &mut command,
        &base_dir,
        "https://www.example.com/video",
        Some(secret_cookie),
        Some("https://www.example.com/page"),
        Some("TestUA/2.0"),
    )
    .await
    .expect("attach_auth_args_in_dir 应成功");

    // 验证临时 Cookie 文件已创建
    let cookie_path = guard.path().expect("应返回 cookie 文件路径").to_path_buf();
    assert!(
        cookie_path.exists(),
        "Cookie 临时文件应存在：{}",
        cookie_path.display()
    );

    // 启动子进程并捕获其 stdout（即命令行参数回显）
    let output = command.output().await.expect("echo 子进程应可启动");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // 断言 1：Cookie 原文不得出现在命令行
    assert!(
        !stdout.contains("secret_value_abc123"),
        "Cookie 原文泄露到命令行：{stdout}"
    );
    assert!(
        !stdout.contains("sensitive_xyz"),
        "Cookie token 值泄露到命令行：{stdout}"
    );
    assert!(
        !stdout.contains("session=secret_value_abc123"),
        "完整 Cookie 字符串泄露到命令行：{stdout}"
    );

    // 断言 2：`--cookies` 标志应出现（指向临时文件）
    assert!(
        stdout.contains("--cookies"),
        "命令行应包含 --cookies 标志：{stdout}"
    );

    // 断言 3：不得出现 `--add-header Cookie:` 危险模式
    let stdout_lower = stdout.to_lowercase();
    assert!(
        !(stdout_lower.contains("--add-header") && stdout_lower.contains("cookie")),
        "命令行不得通过 --add-header 传递 Cookie：{stdout}"
    );

    // Referer 可能含凭证，和 UA 一并进入受限临时配置而非 argv。
    assert!(!stdout.contains("https://www.example.com/page"));
    assert!(!stdout.contains("TestUA/2.0"));
    assert!(stdout.contains("--config-locations"));

    // 显式 consume，删除临时文件
    guard.consume().await;
    assert!(
        !cookie_path.exists(),
        "Cookie 临时文件应在 consume 后被删除"
    );
}

/// 验证无认证参数时不创建临时文件也不添加 `--cookies` 参数。
#[tokio::test]
async fn attach_auth_args_without_cookie_is_noop() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base_dir = temp_dir.path().to_path_buf();

    #[cfg(windows)]
    let mut command = Command::new("cmd.exe");
    #[cfg(windows)]
    {
        command.args(["/c", "echo"]);
    }
    #[cfg(not(windows))]
    let mut command = Command::new("/bin/echo");

    let guard = attach_auth_args_in_dir(
        &mut command,
        &base_dir,
        "https://example.com/video",
        None,
        None,
        None,
    )
    .await
    .expect("无认证参数应成功");

    assert!(guard.path().is_none(), "无 cookie 时不应创建文件");

    let output = command.output().await.expect("echo 子进程应可启动");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("--cookies"),
        "无 cookie 时不应添加 --cookies 标志：{stdout}"
    );
    assert!(
        !stdout.contains("--referer"),
        "无 referer 时不应添加 --referer 标志：{stdout}"
    );

    // consume 无文件情况应正常
    guard.consume().await;
}

/// 验证空 Cookie 字符串被当作无 Cookie 处理（不创建文件）。
#[tokio::test]
async fn attach_auth_args_with_empty_cookie_is_noop() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base_dir = temp_dir.path().to_path_buf();

    #[cfg(windows)]
    let mut command = Command::new("cmd.exe");
    #[cfg(windows)]
    {
        command.args(["/c", "echo"]);
    }
    #[cfg(not(windows))]
    let mut command = Command::new("/bin/echo");

    let guard = attach_auth_args_in_dir(
        &mut command,
        &base_dir,
        "https://example.com/video",
        Some("   "),
        None,
        None,
    )
    .await
    .expect("空 cookie 应成功");

    assert!(guard.path().is_none(), "空 cookie 时不应创建文件");
    guard.consume().await;
}

/// 验证临时 Cookie 文件使用 Netscape 格式且包含正确的域名。
#[tokio::test]
async fn cookie_file_uses_netscape_format_with_correct_domain() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base_dir = temp_dir.path().to_path_buf();

    let mut command = Command::new("echo");
    command.arg("noop");

    let guard = attach_auth_args_in_dir(
        &mut command,
        &base_dir,
        "https://www.example.com/video",
        Some("session=abc; theme=dark"),
        None,
        None,
    )
    .await
    .expect("应成功创建 cookie 文件");

    let cookie_path = guard.path().unwrap().to_path_buf();
    let content = tokio::fs::read_to_string(&cookie_path)
        .await
        .expect("应能读取 cookie 文件");

    // Netscape 格式首行
    assert!(
        content.starts_with("# Netscape HTTP Cookie File"),
        "Cookie 文件应以 Netscape 头行开始：{content}"
    );
    // Netscape 格式：domain\tFALSE\t/\tTRUE\t0\tname\tvalue
    // 域名按 exact host 保留 www.example.com，secure 匹配 https
    assert!(
        content.contains("www.example.com\tFALSE\t/\tTRUE\t0\tsession\tabc"),
        "Cookie 文件应包含 www.example.com 域名和 session 条目：{content}"
    );
    assert!(
        content.contains("www.example.com\tFALSE\t/\tTRUE\t0\ttheme\tdark"),
        "Cookie 文件应包含 theme 条目：{content}"
    );

    guard.consume().await;
    assert!(!cookie_path.exists(), "文件应在 consume 后删除");
}
