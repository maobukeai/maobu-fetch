use super::*;

pub async fn prepare_native_output_name(
    task: &mut DownloadTask,
    templates: &[PlatformNamingTemplate],
    store: &crate::store::Store,
) -> Result<(), String> {
    let intent = store.get_task_name_intent(&task.id).await?;
    if intent == Some(true) {
        return Ok(());
    }
    let mut metadata = output::read_metadata(task);
    if metadata.is_none() {
        let cookie = task
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
            .map(|(_, value)| value.as_str());
        metadata = probe_cache::load(store.data_dir(), &task.url, cookie).await?;
        if let Some(value) = &metadata {
            let workspace = output::Workspace::open(task, "").await?;
            workspace.record_metadata(value).await?;
        }
    }
    if intent == Some(false)
        && metadata.is_none()
        && find_template_for_platform(templates, detect_platform(&task.url).as_str()).is_some_and(
            |template| {
                ["{author}", "{date}", "{channel}", "{id}", "{bvid}"]
                    .iter()
                    .any(|field| template.template.contains(field))
            },
        )
    {
        return Err("平台命名模板需要作者或日期等媒体信息，但当前探测缓存已过期或身份不匹配。请重新分析媒体后创建任务，原有文件不会改动".into());
    }
    apply(task, metadata.as_ref(), templates, store).await
}

/// The manager may do one cancellable task-budgeted probe before replacing a page URL.
/// Old task records have unknown naming intent and retain their existing name without probing.
pub async fn need_native_naming_probe(
    task: &DownloadTask,
    templates: &[PlatformNamingTemplate],
    store: &crate::store::Store,
) -> Result<bool, String> {
    if store.get_task_name_intent(&task.id).await? != Some(false) {
        return Ok(false);
    }
    let platform = detect_platform(&task.url);
    let Some(template) = find_template_for_platform(templates, platform.as_str()) else {
        return Ok(false);
    };
    let required = ["{author}", "{date}", "{channel}", "{id}", "{bvid}"];
    if !required
        .iter()
        .any(|field| template.template.contains(field))
    {
        return Ok(false);
    }
    let mut metadata = output::read_metadata(task);
    if metadata.is_none() {
        let cookie = task
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("cookie"))
            .map(|(_, value)| value.as_str());
        metadata = probe_cache::load(store.data_dir(), &task.url, cookie).await?;
    }
    let Some(metadata) = metadata else {
        return Ok(true);
    };
    let vars = build_naming_vars(&metadata, platform);
    Ok([
        ("{author}", vars.author),
        ("{date}", vars.date),
        ("{channel}", vars.channel),
        ("{id}", vars.id),
        ("{bvid}", vars.bvid),
    ]
    .iter()
    .any(|(field, value)| {
        template.template.contains(field)
            && value
                .as_deref()
                .is_none_or(|value| value.is_empty() || value == "NA")
    }))
}

/// Naming uses only metadata already obtained by the downloader; completed media never waits on another website request.
pub(super) async fn apply(
    task: &mut DownloadTask,
    metadata: Option<&Value>,
    templates: &[PlatformNamingTemplate],
    store: &crate::store::Store,
) -> Result<(), String> {
    let intent = store.get_task_name_intent(&task.id).await?;
    if intent == Some(true) {
        return Ok(());
    }
    let Some(metadata) = metadata else {
        return Ok(());
    };
    let platform = match metadata.get("maobu_platform").and_then(Value::as_str) {
        Some("bilibili") => MediaPlatform::Bilibili,
        Some("youtube") => MediaPlatform::YouTube,
        Some("douyin") => MediaPlatform::Douyin,
        Some("twitter") => MediaPlatform::Twitter,
        Some("tiktok") => MediaPlatform::TikTok,
        Some("weibo") => MediaPlatform::Weibo,
        Some("pikpak") => MediaPlatform::PikPak,
        _ => detect_platform(&task.url),
    };
    let Some(template) = find_template_for_platform(templates, platform.as_str()) else {
        return Ok(());
    };
    let title = metadata.get("title").and_then(Value::as_str).unwrap_or("");
    let stem = Path::new(&task.file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let derived_title = crate::manager::safe_name(title);
    let source_name = url::Url::parse(&task.url)
        .ok()
        .and_then(|url| url.path_segments()?.next_back().map(str::to_owned))
        .unwrap_or_default();
    // Preserve an explicitly chosen name. Existing task JSON has no persisted user-edited flag,
    // so only automatically derived title/URL/identifier names are eligible for a global template.
    if intent.is_none()
        && stem != derived_title
        && task.file_name != source_name
        && !stem
            .chars()
            .all(|char| char.is_ascii_digit() || char == '.' || char == '_')
    {
        return Ok(());
    }
    let vars = build_naming_vars(metadata, platform);
    for (field, value) in [
        ("{author}", &vars.author),
        ("{date}", &vars.date),
        ("{channel}", &vars.channel),
        ("{id}", &vars.id),
        ("{bvid}", &vars.bvid),
    ] {
        if template.template.contains(field)
            && value
                .as_deref()
                .is_none_or(|value| value.trim().is_empty() || value == "NA")
        {
            return Err("平台没有返回命名模板所需的作者、日期或编号，请检查模板或使用不依赖未知属性的文件名".into());
        }
    }
    let mut next = apply_naming_template(&template.template, &vars);
    let rules = store.filename_cleanup_rule_list().await?;
    next = crate::manager::apply_filename_cleanup(&next, &rules);
    next = crate::manager::safe_name(&next);
    if next.trim().is_empty() {
        return Err("平台命名模板没有产生有效文件名，请检查模板设置".into());
    }
    let extension = Path::new(&task.file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("mp4");
    task.file_name = format!("{next}.{extension}");
    Ok(())
}
