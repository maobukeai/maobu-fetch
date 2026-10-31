//! A declared HTML response cannot complete a selected Lanzou binary file.
use super::DownloadTask;
use reqwest::{header::CONTENT_TYPE, Response};
use std::path::Path;

const HTML_FILE_ERROR: &str = "MEDIA_PROBE_ERROR:蓝奏云将响应标记为 HTML 网页，与所选文件类型不符，可能是中间页或验证页。已停止下载，未发布完成文件；已有文件和分片已保留。请在浏览器打开分享，再通过猫步下载器扩展接管真正的文件下载。";

pub(super) fn requires_binary(task: &DownloadTask) -> bool {
    if !task.source.eq_ignore_ascii_case("lanzou") {
        return false;
    }
    let extension = Path::new(&task.file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "exe"
            | "msi"
            | "msix"
            | "msixbundle"
            | "appx"
            | "appxbundle"
            | "zip"
            | "7z"
            | "rar"
            | "tar"
            | "gz"
            | "tgz"
            | "bz2"
            | "xz"
            | "zst"
            | "iso"
            | "img"
            | "bin"
            | "apk"
            | "dmg"
            | "pkg"
            | "mp4"
            | "m4v"
            | "mkv"
            | "webm"
            | "mov"
            | "avi"
            | "flv"
            | "wmv"
            | "ts"
            | "m2ts"
            | "mpeg"
            | "mpg"
            | "mp3"
            | "m4a"
            | "aac"
            | "flac"
            | "wav"
            | "ogg"
            | "opus"
            | "pdf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "tif"
            | "tiff"
            | "ico"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
    )
}

pub(super) fn validate_response(task: &DownloadTask, response: &Response) -> Result<(), String> {
    validate_binary_response(requires_binary(task), response)
}

/// Range sessions retain only this boolean, not another copy of task credentials.
pub(super) fn validate_binary_response(
    expected_binary: bool,
    response: &Response,
) -> Result<(), String> {
    if !expected_binary || !response.status().is_success() {
        return Ok(());
    }
    let mime = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if mime.is_some_and(|value| {
        value.eq_ignore_ascii_case("text/html")
            || value.eq_ignore_ascii_case("application/xhtml+xml")
    }) {
        return Err(HTML_FILE_ERROR.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(source: &str, name: &str) -> DownloadTask {
        let mut task = super::super::tests::test_task(
            Path::new("unused"),
            name,
            super::super::CollisionPolicy::Rename,
        );
        task.source = source.into();
        task
    }
    fn response(status: u16, mime: Option<&str>) -> Response {
        let mut builder = axum::http::Response::builder().status(status);
        if let Some(mime) = mime {
            builder = builder.header(CONTENT_TYPE, mime);
        }
        builder
            .body(reqwest::Body::from("<html>intermediate page</html>"))
            .unwrap()
            .into()
    }

    #[test]
    fn selected_lanzou_binary_types_reject_html_including_case_and_mime_parameters() {
        for name in [
            "archive.7z.exe",
            "archive.tar.gz",
            "MOVIE.MP4",
            "music.flac",
            "image.png",
            "report.pdf",
        ] {
            for mime in [
                "text/html",
                "Text/Html; charset=UTF-8",
                " application/xhtml+xml ; charset=utf-8",
            ] {
                let error = validate_response(&task("lanzou", name), &response(200, Some(mime)))
                    .unwrap_err();
                assert!(error.starts_with("MEDIA_PROBE_ERROR:"));
                assert!(error.contains("浏览器") && error.contains("分片已保留"));
            }
        }
    }

    #[test]
    fn intentional_html_and_undeclared_file_types_are_not_globally_blocked() {
        for name in [
            "page.html",
            "page.HTM",
            "source.xhtml",
            "notes.txt",
            "data.json",
            "download",
            "unknown.custom",
        ] {
            assert!(
                validate_response(&task("lanzou", name), &response(200, Some("text/html"))).is_ok()
            );
        }
        for source in ["desktop", "extension", "quark", "not-lanzou"] {
            assert!(validate_response(
                &task(source, "saved.exe"),
                &response(200, Some("text/html"))
            )
            .is_ok());
        }
    }

    #[test]
    fn unknown_or_non_html_mime_and_failed_http_status_keep_existing_handling() {
        for mime in [
            None,
            Some("application/octet-stream"),
            Some("video/mp4"),
            Some("text/plain"),
            Some("application/json"),
            Some("text/html-not-a-mime"),
        ] {
            assert!(
                validate_response(&task("lanzou", "archive.exe"), &response(200, mime)).is_ok()
            );
        }
        for status in [302, 403, 416, 503] {
            assert!(validate_response(
                &task("lanzou", "archive.exe"),
                &response(status, Some("text/html"))
            )
            .is_ok());
        }
    }

    #[test]
    fn range_responses_use_the_same_guard_without_carrying_secrets() {
        assert!(
            validate_binary_response(true, &response(206, Some("application/xhtml+xml"))).is_err()
        );
        assert!(validate_binary_response(false, &response(206, Some("text/html"))).is_ok());
        let mut task = task("lanzou", "archive.exe");
        task.url = "https://private.invalid/?token=synthetic-private".into();
        task.headers
            .insert("Cookie".into(), "synthetic-cookie".into());
        let error = validate_response(&task, &response(200, Some("text/html"))).unwrap_err();
        assert!(!error.contains("synthetic") && !error.contains("private.invalid"));
    }
}
