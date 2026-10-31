//! Real browser -> unchanged Rust handlers, with an isolated Manager and an ephemeral port.
use super::*;
use std::path::PathBuf;

pub(crate) async fn run(
    app: &AppHandle,
    manager: &SharedManager,
    output: &std::path::Path,
    source: &str,
    slow_source: &str,
    expected_sha: &str,
) -> Result<serde_json::Value, String> {
    let root = output.join("bridge-browser");
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    for browser in ["Edge", "Chrome"] {
        results.push(
            browser_case(
                app,
                manager,
                &root,
                browser,
                source,
                slow_source,
                expected_sha,
            )
            .await?,
        );
    }
    Ok(serde_json::Value::Array(results))
}

async fn browser_case(
    app: &AppHandle,
    manager: &SharedManager,
    root: &std::path::Path,
    browser: &str,
    source: &str,
    slow_source: &str,
    expected_sha: &str,
) -> Result<serde_json::Value, String> {
    let directory = root.join(browser);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| error.to_string())?;
    let pairing = PairingService::new(manager.clone());
    let code = pairing.info().await?.code;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    if address.port() == 17433 {
        return Err("隔离桥不得使用正式端口".into());
    }
    let app_router = router(manager.clone(), pairing, app.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app_router)
            .await
            .map_err(|error| error.to_string())
    });
    let guard = Server(server);
    let config = directory.join("fixture.json");
    let phase = directory.join("phase.json");
    let revoked = directory.join("revoked");
    let offline_ready = directory.join("offline-ready");
    let offline = directory.join("offline");
    let value = serde_json::json!({"browser":browser,"base_url":format!("http://{address}"),"code":code,"source_url":source,"slow_url":slow_source,"destination":directory.join("downloads"),"expected_sha256":expected_sha,"phase_file":phase,"revoked_file":revoked,"offline_ready_file":offline_ready,"offline_file":offline,"root":directory});
    tokio::fs::write(&config, value.to_string())
        .await
        .map_err(|error| error.to_string())?;
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/audit/bridge-browser-e2e.mjs");
    let mut command = crate::media_tools::create_hidden_tokio_command("node");
    command.arg(script).arg(&config);
    let mut child = Box::pin(crate::media::execution_fixtures::checked_with_timeout(
        &mut command,
        Duration::from_secs(120),
    ));
    let phase_value = tokio::select! {
        result=child.as_mut()=>{let _=result?;return Err("浏览器在提交桥接验证阶段之前退出".into())},
        result=wait_json(&phase)=>result?,
    };
    verify_tasks(manager, &phase_value, expected_sha).await?;
    manager.store.clear_pairing().await?;
    tokio::fs::write(&revoked, b"revoked")
        .await
        .map_err(|error| error.to_string())?;
    tokio::select! { result=child.as_mut()=>{let _=result?;return Err("浏览器在验证离线回退之前退出".into())},result=wait_file(&offline_ready)=>result? }
    drop(guard);
    tokio::fs::write(&offline, b"offline")
        .await
        .map_err(|error| error.to_string())?;
    let output = child.await?;
    serde_json::from_slice(&output).map_err(|_| "浏览器桥接验证结果不是有效JSON".into())
}

async fn verify_tasks(
    manager: &SharedManager,
    value: &serde_json::Value,
    expected_sha: &str,
) -> Result<(), String> {
    let cancelled = value
        .get("cancelled_task")
        .and_then(serde_json::Value::as_str)
        .ok_or("缺少取消任务验证")?;
    let cancelled = manager
        .store
        .get_task(cancelled)
        .await?
        .ok_or("桥接取消任务不存在")?;
    if cancelled.status != crate::models::TaskStatus::Cancelled || cancelled.active_connections != 0
    {
        return Err("真实桥接取消未停止任务".into());
    }
    if PathBuf::from(&cancelled.destination)
        .join(&cancelled.file_name)
        .exists()
    {
        return Err("已取消任务暴露了完成文件".into());
    }
    let completed = value
        .get("completed_task")
        .and_then(serde_json::Value::as_str)
        .ok_or("缺少完成任务验证")?;
    let completed = manager
        .store
        .get_task(completed)
        .await?
        .ok_or("桥接完成任务不存在")?;
    if completed.status != crate::models::TaskStatus::Completed
        || completed.source != "browser"
        || crate::manager::has_auth_headers(&completed.headers)
    {
        return Err("真实桥接完成状态或凭证清理不正确".into());
    }
    let bytes = tokio::fs::read(PathBuf::from(completed.destination).join(completed.file_name))
        .await
        .map_err(|error| error.to_string())?;
    if hex::encode(Sha256::digest(&bytes)) != expected_sha {
        return Err("真实浏览器桥接下载SHA不一致".into());
    }
    Ok(())
}

async fn wait_file(path: &std::path::Path) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            if path.is_file() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| "等待隔离浏览器验证超时")?
}
async fn wait_json(path: &std::path::Path) -> Result<serde_json::Value, String> {
    wait_file(path).await?;
    serde_json::from_slice(
        &tokio::fs::read(path)
            .await
            .map_err(|error| error.to_string())?,
    )
    .map_err(|_| "浏览器阶段结果无效".into())
}
struct Server(tokio::task::JoinHandle<Result<(), String>>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
