//! Opt-in public file audit: ordinary HTTP worker, no GUI, no execution of downloaded content.
use super::super::*;
use super::harness::Harness;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    url: String,
    headers: HashMap<String, String>,
    public_share_url: String,
    expected_sha256: String,
}

pub async fn run(app: &AppHandle) {
    assert_eq!(
        std::env::var("MAOBU_TEST_CLOUD_FILE").as_deref(),
        Ok("1"),
        "external file audit requires explicit opt-in"
    );
    let manifest =
        std::env::var("MAOBU_TEST_CLOUD_FILE_MANIFEST").expect("private TEMP manifest required");
    let bytes = fs::read(manifest).await.unwrap();
    assert!(bytes.len() < 64 * 1024);
    let input: Input = serde_json::from_slice(&bytes).unwrap();
    let direct = Url::parse(&input.url).unwrap();
    assert!(
        matches!(direct.scheme(), "http" | "https")
            && direct.host_str().is_some()
            && direct.username().is_empty()
            && direct.password().is_none()
    );
    assert!(
        input.expected_sha256.len() == 64
            && input.expected_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    );
    let source = Url::parse(&input.public_share_url).unwrap();
    assert!(
        source.scheme() == "https"
            && source.query().is_none()
            && source.username().is_empty()
            && source.password().is_none()
    );
    let harness = Harness::new(app, Some(240)).await;
    let mut settings = harness.manager.settings().await;
    settings.default_retry_policy.connection_timeout_secs = 30;
    settings.proxy_mode = "none".into();
    harness.manager.save_settings(settings).await.unwrap();
    let mut task = super::super::tests::test_task(
        &harness.output,
        "cloud-public-file.bin",
        CollisionPolicy::Rename,
    );
    task.id = Uuid::new_v4().to_string();
    task.url = input.url;
    task.headers = input.headers;
    task.connection_count = 1;
    task.max_retries = 0;
    task.per_task_speed_limit = 1024 * 1024;
    task.expected_checksum = Some(input.expected_sha256.clone());
    task.proxy_override = Some(
        ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|s| !s.trim().is_empty()))
            .unwrap_or_default(),
    );
    harness
        .manager
        .store
        .create_task_with_name_intent(&task, true)
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let mut operation = Box::pin(harness.manager.run_public_audit_task(task.clone(), &cancel));
    let outcome = tokio::select! {
        result=operation.as_mut()=>result,
        _=tokio::time::sleep(Duration::from_secs(240))=>{
            cancel.cancel();
            let _=tokio::time::timeout(Duration::from_secs(35),operation.as_mut()).await;
            Err("公网文件测试超过4分钟，已停止；不声明完整下载通过".into())
        }
    };
    drop(operation);
    harness
        .manager
        .settle_public_audit_task(&task.id)
        .await
        .unwrap();
    let verified=match outcome {
        Ok(row)=>fingerprint(&harness.output.join(&row.file_name)).await.map(|(length,sha)|{
            assert!(length>0 && length<=256*1024*1024);
            assert_eq!(sha,input.expected_sha256.to_ascii_lowercase(),"must match the publisher's actual official checksum");
            serde_json::json!({"passed":true,"complete_download":true,"bytes":length,"sha256":sha,"official_sha256_match":true,"executed":false})
        }),
        Err(error)=>Err(error),
    };
    harness.manager.shutdown_bt().await.unwrap();
    let mut result = match &verified {
        Ok(value) => value.clone(),
        Err(error) => {
            serde_json::json!({"passed":false,"complete_download":false,"error":redact_sensitive(error),"executed":false})
        }
    };
    result["public_share_url"] = input.public_share_url.into();
    result["direct_host"] = direct.host_str().unwrap().into();
    result["network"] = "explicit-existing-environment-proxy".into();
    eprintln!("PUBLIC_CLOUD_FILE {}", result);
    verified.expect(
        "public file audit must actually complete and match the official SHA; failure is retained",
    );
}

async fn fingerprint(path: &Path) -> Result<(u64, String), String> {
    let mut file = fs::File::open(path).await.map_err(|_| "最终文件不存在")?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|_| "读取最终文件失败")?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        hash.update(&buffer[..count]);
    }
    Ok((bytes, hex::encode(hash.finalize())))
}
