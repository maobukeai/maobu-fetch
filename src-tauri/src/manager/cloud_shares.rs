//! Raw share pages require explicit file selection before a download task starts.
use super::*;

pub(super) fn is_raw_share_task(task: &DownloadTask) -> bool {
    if task
        .method
        .as_deref()
        .is_some_and(|method| !method.eq_ignore_ascii_case("GET"))
    {
        return false;
    }
    is_raw_share_url(&task.url)
}

pub(super) fn is_raw_share_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw.trim()) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let pieces: Vec<_> = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    let identifier = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    };
    let share_path = pieces.first() == Some(&"s")
        && (2..=3).contains(&pieces.len())
        && pieces[1..].iter().all(|part| identifier(part));
    if matches!(host.as_str(), "pan.baidu.com" | "yun.baidu.com") {
        return share_path
            || (url.path() == "/share/init"
                && url
                    .query_pairs()
                    .any(|(key, value)| key == "surl" && !value.is_empty()));
    }
    if matches!(
        host.as_str(),
        "pan.quark.cn"
            | "drive.quark.cn"
            | "mypikpak.com"
            | "www.mypikpak.com"
            | "mypikpak.net"
            | "www.mypikpak.net"
    ) {
        return share_path;
    }
    let canonical = host.strip_prefix("www.").unwrap_or(&host);
    if matches!(
        canonical,
        "123pan.com" | "123pan.cn" | "123684.com" | "123952.com"
    ) {
        return share_path;
    }
    if host.ends_with(".share.123pan.cn") {
        return pieces.len() == 2 && pieces[0] == "123pan" && identifier(pieces[1]);
    }
    let labels: Vec<_> = host.split('.').collect();
    if labels.len() >= 2
        && matches!(
            *labels.last().unwrap_or(&""),
            "com" | "net" | "cn" | "org" | "xyz" | "cc" | "icu" | "vip"
        )
    {
        let root = labels[labels.len() - 2];
        let family = root
            .strip_prefix("lanzou")
            .or_else(|| root.strip_prefix("lanzo"));
        if family.is_some_and(|suffix| {
            suffix.len() <= 1 && suffix.bytes().all(|byte| byte.is_ascii_alphabetic())
        }) {
            return (pieces.len() == 1 && identifier(pieces[0]))
                || (pieces.len() == 2
                    && matches!(pieces[0], "u" | "tp" | "b")
                    && identifier(pieces[1]));
        }
    }
    false
}

impl DownloadManager {
    pub(super) async fn resolve_cloud_share_task(
        self: &SharedManager,
        task: &mut DownloadTask,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        if cancel.is_cancelled() {
            return Err("任务已暂停".into());
        }
        if is_raw_share_task(task) {
            return Err("MEDIA_PROBE_ERROR:这是网盘分享页面，请先在新建任务中解析分享并勾选需要的文件，再开始下载；未自动创建其他任务".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_share_urls_require_selection_and_download_endpoints_remain_http() {
        for url in [
            "https://pan.baidu.com/s/1synthetic",
            "https://yun.baidu.com/share/init?surl=synthetic",
            "https://pan.quark.cn/s/synthetic",
            "https://drive.quark.cn/s/synthetic/child",
            "https://mypikpak.com/s/synthetic",
            "https://mypikpak.net/s/synthetic/child",
            "https://www.123pan.com/s/synthetic",
            "https://123pan.cn/s/synthetic",
            "https://123684.com/s/synthetic",
            "https://123952.com/s/synthetic",
            "https://test.share.123pan.cn/123pan/synthetic",
            "https://wwabc.lanzouq.com/synthetic",
            "https://lanzoux.com/b/synthetic",
            "https://lanzoui.com/u/synthetic",
        ] {
            assert!(is_raw_share_url(url), "missed synthetic share shape");
        }
        for url in [
            "https://cdn.example/file?next=https://pan.baidu.com/s/synthetic",
            "https://pan.baidu.com.evil.invalid/s/synthetic",
            "https://evil.invalid/pan.quark.cn/s/synthetic",
            "https://d.pcs.baidu.com/rest/2.0/pcs/file?method=download",
            "https://cdn.baidupcs.com/file.bin",
            "https://drive.quark.cn/1/clouddrive/file/download",
            "https://api-drive.mypikpak.com/drive/v1/files/actual",
            "https://download.123pan.com/file.bin",
            "https://123pan.com/api/share/download/info",
            "https://wwabc.lanzouq.com/file/signed-download",
            "https://cdn.example/lanzou-file.bin",
            "https://fake-lanzou.evil.invalid/synthetic",
            "https://pan.baidu.com/s/file.mp4",
            "https://123pan.com/s/file.zip",
        ] {
            assert!(
                !is_raw_share_url(url),
                "real direct request must not be treated as a share page"
            );
        }
        let directory = tempfile::tempdir().unwrap();
        let mut task =
            super::super::tests::test_task(directory.path(), "file.bin", CollisionPolicy::Rename);
        task.url = "https://pan.baidu.com/s/synthetic".into();
        task.method = Some("POST".into());
        assert!(!is_raw_share_task(&task));
    }
}
