use super::probe_runtime::ProbeSession;
use super::*;

pub(super) async fn douyin(
    session: &ProbeSession<'_>,
    effective_url: &str,
) -> Result<Value, String> {
    let client = session.client;
    let native_target = if is_douyin_live(effective_url) {
        "https://live.douyin.com/"
    } else {
        "https://www.douyin.com/"
    };
    let scoped_cookie = session.cookie.map(|cookie| {
        crate::media_cookies::header_for_url(cookie, session.source_url, native_target)
    });
    let cookie = scoped_cookie.as_deref().filter(|value| !value.is_empty());
    let referer = session.referer;
    let user_agent = session.user_agent;
    Ok({
        // 优先检查抖音直播 URL
        if is_douyin_live(&effective_url) {
            if let Some(room_id) = extract_douyin_live_room_id(&effective_url) {
                match crate::media_platforms::fetch_douyin_live_detail_with_client(
                    &room_id, cookie, referer, user_agent, client,
                )
                .await
                {
                    Ok(detail) => {
                        let mut json = convert_douyin_live_to_yt_dlp_json(&detail);
                        json["webpage_url"] = Value::String(effective_url.to_owned());
                        tracing::info!(
                            room_id = %room_id,
                            "抖音直播直连解析成功，跳过 yt-dlp 调用"
                        );
                        json
                    }
                    Err(_error) => {
                        tracing::warn!(
                            room_id = %room_id,
                            "抖音直播直连解析失败，回退到 yt-dlp 流程"
                        );
                        session.yt_dlp(&effective_url).await?
                    }
                }
            } else {
                session.yt_dlp(&effective_url).await?
            }
        } else if let Some(id) = extract_douyin_aweme_id(&effective_url) {
            match crate::media_platforms::fetch_douyin_aweme_detail_with_client(
                &id, cookie, referer, user_agent, client,
            )
            .await
            {
                Ok(detail) => {
                    let mut json = convert_douyin_aweme_to_yt_dlp_json(&detail);
                    // 保留原始 URL 给后续命名模板与下载流程使用
                    json["webpage_url"] = Value::String(effective_url.to_owned());
                    tracing::info!(
                        aweme_id = %id,
                        "抖音直连解析成功，跳过 yt-dlp 调用"
                    );
                    json
                }
                Err(_error) => {
                    tracing::warn!(
                        aweme_id = %id,
                        "抖音直连解析失败，回退到 yt-dlp 流程"
                    );
                    let canonical_url = format!("https://www.douyin.com/video/{id}");
                    session.yt_dlp(&canonical_url).await?
                }
            }
        } else {
            let host = crate::media_platforms::extract_host(&effective_url);
            if host == "douyin.com" || host == "www.douyin.com" {
                return Err("未识别到具体的抖音视频 ID，请打开具体的视频页面后再下载".into());
            }
            session.yt_dlp(&effective_url).await?
        }
    })
}

pub(super) async fn twitter(
    session: &ProbeSession<'_>,
    effective_url: &str,
) -> Result<Value, String> {
    let client = session.client;
    let cookie = session.cookie;
    let referer = session.referer;
    let user_agent = session.user_agent;
    Ok({
        if let Some(tweet_id) = crate::media_platforms::extract_twitter_status_id(&effective_url) {
            match crate::media_platforms::fetch_twitter_tweet_detail_with_client(
                &tweet_id, cookie, referer, user_agent, client,
            )
            .await
            {
                Ok(detail) => {
                    let mut json =
                        crate::media_platforms::convert_twitter_tweet_to_yt_dlp_json(&detail);
                    json["webpage_url"] = Value::String(effective_url.to_owned());
                    // Native-empty results may use valid unknown tool candidates; unknown
                    // native tracks only yield to confirmed tool audio, never empty metadata.
                    let tool = if native_probe_fallback::needs_tool(&json)?
                        && resolve_yt_dlp(session.app, session.settings).is_some()
                    {
                        Some(session.yt_dlp(effective_url).await)
                    } else {
                        None
                    };
                    json = native_probe_fallback::select(json, tool)?;
                    tracing::info!(
                        tweet_id = %tweet_id,
                        "Twitter 媒体元数据解析完成"
                    );
                    json
                }
                Err(_error) => {
                    tracing::warn!(
                        tweet_id = %tweet_id,
                        "Twitter 直连解析失败，回退到 yt-dlp 流程"
                    );
                    session.yt_dlp(&effective_url).await?
                }
            }
        } else {
            session.yt_dlp(&effective_url).await?
        }
    })
}

pub(super) async fn pikpak(
    session: &ProbeSession<'_>,
    effective_url: &str,
    ffmpeg_available: bool,
) -> Result<MediaProbeResult, String> {
    let client = session.client;
    let cookie = session.cookie;
    let metadata_task = session.metadata_task;
    let platform = MediaPlatform::PikPak;
    let url = session.source_url;
    let parsed = url::Url::parse(effective_url).map_err(|_| "PikPak 地址无效")?;

    if let Some(parsed) = crate::pikpak::parse_pikpak_url(&effective_url) {
        let context = session.cloud_context()?;
        let device_id = hex::encode(rand::random::<[u8; 16]>());
        let share = crate::pikpak::inspect_pikpak_share_with_context(
            &effective_url,
            parsed.pass_code.clone(),
            &device_id,
            &context,
        )
        .await
        .map_err(|_| "PikPak 原文件列表读取失败，请在网盘文件选择器中检查权限")?;
        let files: Vec<_> = share
            .files
            .iter()
            .filter(|file| file.kind == "drive#file")
            .collect();
        if files.len() != 1 {
            return Err(
                "该 PikPak 分享包含多个文件或目录，请使用网盘文件选择器明确选择原文件".into(),
            );
        }
        let file = files[0];
        let direct = match crate::pikpak::resolve_pikpak_file_with_context(
            &parsed.share_id,
            &file.id,
            share.pass_code_token.as_deref(),
            &device_id,
            &context,
        )
        .await
        {
            Ok(resolved) => resolved.url,
            Err(_) => file
                .web_content_link
                .clone()
                .ok_or("未取得 PikPak 原文件地址，请在网盘文件选择器中重试")?,
        };
        let result = original_file::result(file, &direct, ffmpeg_available)?;
        if let Some(task) = metadata_task {
            output::record_probe_metadata(
                task,
                &serde_json::json!({"title":result.title,"id":file.id}),
                platform,
            )
            .await?;
        } else {
            let metadata = probe_cache::minimal(
                &serde_json::json!({"title":result.title,"id":file.id}),
                platform,
            );
            probe_cache::save(session.data_dir, url, cookie, &metadata).await?;
        }
        return Ok(result);
    }
    let response = client
        .head(effective_url)
        .send()
        .await
        .map_err(|_| "PikPak 原文件信息读取失败，请使用网盘文件选择器")?;
    if !response.status().is_success() {
        return Err("PikPak 原文件地址不可访问，请检查登录权限或分享有效期".into());
    }
    let name = parsed
        .path_segments()
        .and_then(|segments| segments.last())
        .filter(|name| !name.is_empty())
        .unwrap_or("PikPak 原文件")
        .to_owned();
    let file = crate::pikpak::PikPakFileItem {
        id: String::new(),
        name,
        kind: "drive#file".into(),
        size: response.content_length().unwrap_or(0),
        path: String::new(),
        mime_type: response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        file_extension: None,
        thumbnail_url: None,
        web_content_link: None,
    };
    return original_file::result(&file, &effective_url, ffmpeg_available);
}

pub(super) async fn xiaohongshu(
    session: &ProbeSession<'_>,
    effective_url: &str,
) -> Result<Value, String> {
    let client = session.client;
    let cookie = session.cookie;
    let referer = session.referer;
    let user_agent = session.user_agent;
    match crate::media_platforms::fetch_xiaohongshu_note_detail_with_client(
        effective_url,
        cookie,
        referer,
        user_agent,
        client,
    )
    .await
    {
        Ok(mut json) => {
            json["webpage_url"] = Value::String(effective_url.to_owned());
            tracing::info!(url = %effective_url, "小红书直连解析成功，跳过 yt-dlp 调用");
            Ok(json)
        }
        Err(err) => {
            tracing::warn!(url = %effective_url, error = %err, "小红书直连解析未命中或失败，回退到 yt-dlp 流程");
            session.yt_dlp(effective_url).await
        }
    }
}
