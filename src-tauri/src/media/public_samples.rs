//! Opt-in public-site checks; never part of normal startup or the default test suite.
use super::*;
use url::Url;
mod transfer;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sample {
    id: String,
    url: String,
    #[serde(default)]
    probe_only: bool,
    #[serde(default)]
    allow_unknown_audio: bool,
    #[serde(default)]
    proxy_from_environment: bool,
    #[serde(default)]
    expected_duration: Option<f64>,
    #[serde(default)]
    format_id: Option<String>,
}

pub(crate) async fn run(
    app: &AppHandle,
    manager: &crate::manager::SharedManager,
    output: &Path,
) -> Result<Value, String> {
    if std::env::var("MAOBU_TEST_PUBLIC_SAMPLES").as_deref() != Ok("1") {
        return Err("公网样本需要显式启用，默认测试不联网".into());
    }
    let manifest = std::env::var("MAOBU_TEST_PUBLIC_MANIFEST").map_err(|_| "缺少样本清单")?;
    let raw = tokio::fs::read(manifest)
        .await
        .map_err(|_| "无法读取样本清单")?;
    if raw.len() > 64 * 1024 {
        return Err("样本清单过大".into());
    }
    let samples: Vec<Sample> = serde_json::from_slice(&raw).map_err(|_| "样本清单格式错误")?;
    if samples.is_empty() || samples.len() > 24 {
        return Err("每批需要1至24个公开样本".into());
    }
    let mut results = Vec::new();
    for sample in samples {
        let cancel = CancellationToken::new();
        let task_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        let mut operation = Box::pin(one(app, manager, output, &sample, &task_id, &cancel));
        let outcome = tokio::select! {
            result=operation.as_mut()=>result,
            _=tokio::time::sleep(std::time::Duration::from_secs(240))=>{
                cancel.cancel();
                let _=tokio::time::timeout(std::time::Duration::from_secs(35),operation.as_mut()).await;
                Err("样本超过4分钟，已取消且不声明完整通过".into())
            }
        };
        drop(operation);
        if let Err(error) = manager.settle_public_audit_task(&task_id).await {
            return Err(format!(
                "公网样本停止屏障未完成，停止整批避免跨样本干扰：{}",
                safe_error(&error)
            ));
        }
        let mut result = match outcome {
            Ok(value) => value,
            Err(error) => {
                serde_json::json!({"passed":false,"error":safe_error(&error)})
            }
        };
        result["id"] = sample.id.into();
        result["url"] = public_url(&sample.url).into();
        result["network"] = if sample.proxy_from_environment {
            "explicit-environment-proxy"
        } else {
            "direct"
        }
        .into();
        result["seconds"] = started.elapsed().as_secs_f64().into();
        result["anonymous"] = true.into();
        result["used_personal_cookie"] = false.into();
        eprintln!("PUBLIC_SAMPLE {}", result);
        results.push(result);
    }
    Ok(serde_json::json!({"results":results}))
}

async fn one(
    app: &AppHandle,
    manager: &crate::manager::SharedManager,
    output: &Path,
    sample: &Sample,
    task_id: &str,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let url = Url::parse(&sample.url).map_err(|_| "样本URL无效")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || sample.id.is_empty()
        || !sample
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("样本只允许无账号HTTPS地址及安全标识".into());
    }
    let mut settings = manager.settings().await;
    for (name, target) in [
        ("yt-dlp", &mut settings.yt_dlp_path),
        ("ffmpeg", &mut settings.ffmpeg_path),
        ("ffprobe", &mut settings.ffprobe_path),
    ] {
        *target = execution_fixtures::find_tool(name)
            .ok_or_else(|| format!("需要已有{name}；不自动安装"))?
            .to_string_lossy()
            .into_owned();
    }
    settings.proxy_mode = "none".into();
    settings.default_retry_policy.task_timeout_secs = Some(240);
    settings.default_retry_policy.connection_timeout_secs = 30;
    let mut task = test_support::task();
    task.id = task_id.to_owned();
    task.per_task_speed_limit = 1024 * 1024;
    task.url = sample.url.clone();
    task.destination = output.to_string_lossy().into_owned();
    task.file_name = format!("public-{}.mp4", sample.id);
    task.headers.clear();
    task.status = crate::models::TaskStatus::Paused;
    task.proxy_override = if sample.proxy_from_environment {
        Some(
            ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"]
                .iter()
                .find_map(|key| std::env::var(key).ok().filter(|s| !s.trim().is_empty()))
                .ok_or("未配置环境代理，不假称已通过代理")?,
        )
    } else {
        Some(String::new())
    };
    manager.save_settings(settings.clone()).await?;
    manager
        .store
        .create_task_with_name_intent(&task, true)
        .await?;
    let probe = probe_for_task(app, &settings, &task, cancel, manager)
        .await
        .map_err(|e| format!("元数据阶段：{e}"))?;
    if probe.drm || probe.is_live {
        return Err("样本为DRM或直播，拒绝当作完整点播测试".into());
    }
    let metadata = serde_json::json!({"title":probe.title,"duration":probe.duration,"formats":probe.formats.len(),"layer":"production-probe"});
    eprintln!(
        "PUBLIC_SAMPLE_PROBE {}",
        serde_json::json!({"id":sample.id,"metadata":metadata})
    );
    if sample.probe_only {
        return Ok(
            serde_json::json!({"passed":!probe.formats.is_empty(),"metadata":metadata,"complete_download":false}),
        );
    }
    transfer::run(manager, &settings, task, &probe, sample, cancel, metadata).await
}

fn public_url(raw: &str) -> Option<String> {
    Url::parse(raw).ok().map(|mut parsed| {
        let video = parsed
            .query_pairs()
            .find(|(key, value)| {
                key == "v"
                    && value.len() == 11
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
            .map(|(_, v)| v.into_owned());
        let youtube = matches!(parsed.host_str(), Some("www.youtube.com" | "youtube.com"));
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        parsed.set_query(None);
        parsed.set_fragment(None);
        if youtube {
            if let Some(video) = video {
                parsed.query_pairs_mut().append_pair("v", &video);
            }
        }
        parsed.to_string()
    })
}

fn safe_error(error: &str) -> String {
    let urls = regex::Regex::new(r#"https?://[^\s\"'<>]+"#)
        .expect("static test-only URL redaction expression");
    let safe = urls.replace_all(error, |captures: &regex::Captures<'_>| {
        public_url(&captures[0]).unwrap_or_else(|| "[URL]".into())
    });
    crate::manager::diagnose::redact_sensitive(&safe)
}
