use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct Tools {
    pub yt_dlp: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

pub(crate) fn find_tool(name: &str) -> Option<PathBuf> {
    let executable = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(&executable))
            .find(|path| path.is_file())
    })
}

pub(crate) struct Fixtures {
    pub tools: Tools,
    pub root: PathBuf,
    pub base_url: String,
    pub cookie_hits: Arc<AtomicUsize>,
    pub leaked_proxy_header: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixtures {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub(crate) async fn create(directory: &Path) -> Result<Option<Fixtures>, String> {
    let (Some(yt_dlp), Some(ffmpeg), Some(ffprobe)) = (
        find_tool("yt-dlp"),
        find_tool("ffmpeg"),
        find_tool("ffprobe"),
    ) else {
        return Ok(None);
    };
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| error.to_string())?;
    let root = tokio::fs::canonicalize(directory)
        .await
        .map_err(|error| error.to_string())?;
    let tools = Tools {
        yt_dlp,
        ffmpeg,
        ffprobe,
    };
    generate(&tools, &root).await?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let files = root.clone();
    let cookie_hits = Arc::new(AtomicUsize::new(0));
    let hits = cookie_hits.clone();
    let leaked_proxy_header = Arc::new(AtomicBool::new(false));
    let leaked = leaked_proxy_header.clone();
    let router =
        axum::Router::new().fallback(move |request: axum::http::Request<axum::body::Body>| {
            let files = files.clone();
            let hits = hits.clone();
            let leaked = leaked.clone();
            async move {
                if request.headers().contains_key("proxy-authorization") {
                    leaked.store(true, Ordering::Relaxed);
                }
                if request.headers().get("cookie").is_some_and(|value| {
                    value
                        .to_str()
                        .is_ok_and(|value| value.contains("session=synthetic"))
                }) {
                    hits.fetch_add(1, Ordering::Relaxed);
                }
                let name = request.uri().path().trim_start_matches('/');
                if name.is_empty()
                    || name.contains("..")
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
                    })
                {
                    return axum::http::Response::builder()
                        .status(400)
                        .body(axum::body::Body::empty())
                        .unwrap();
                }
                let path = files.join(name);
                let Ok(bytes) = tokio::fs::read(&path).await else {
                    return axum::http::Response::builder()
                        .status(404)
                        .body(axum::body::Body::empty())
                        .unwrap();
                };
                if bytes.is_empty() {
                    return axum::http::Response::builder()
                        .status(404)
                        .body(axum::body::Body::empty())
                        .unwrap();
                }
                let content_type = if name.ends_with(".mpd") {
                    "application/dash+xml"
                } else if name.ends_with(".m3u8") {
                    "application/vnd.apple.mpegurl"
                } else if name.ends_with(".ts") {
                    "video/mp2t"
                } else {
                    "video/mp4"
                };
                let range = request
                    .headers()
                    .get("range")
                    .and_then(|value| value.to_str().ok());
                let (status, start, end) = match range
                    .and_then(|value| value.strip_prefix("bytes="))
                    .and_then(|value| value.split_once('-'))
                    .and_then(|(start, end)| {
                        let start = start.parse::<usize>().ok()?;
                        let end = if end.is_empty() {
                            bytes.len().checked_sub(1)?
                        } else {
                            end.parse::<usize>().ok()?
                        };
                        (start <= end && end < bytes.len()).then_some((start, end))
                    }) {
                    Some((start, end)) => (206, start, end),
                    None => (200, 0, bytes.len().saturating_sub(1)),
                };
                let mut response = axum::http::Response::builder()
                    .status(status)
                    .header("Content-Type", content_type)
                    .header("Accept-Ranges", "bytes")
                    .header("Content-Length", end - start + 1);
                if status == 206 {
                    response = response.header(
                        "Content-Range",
                        format!("bytes {start}-{end}/{}", bytes.len()),
                    );
                }
                response
                    .body(axum::body::Body::from(bytes[start..=end].to_vec()))
                    .unwrap()
            }
        });
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Ok(Some(Fixtures {
        tools,
        root,
        base_url: format!("http://{address}"),
        cookie_hits,
        leaked_proxy_header,
        server,
    }))
}

async fn generate(tools: &Tools, root: &Path) -> Result<(), String> {
    let source = root.join("fixture.mp4");
    let mut command = create_hidden_tokio_command(&tools.ffmpeg);
    command
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=12",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-threads",
            "1",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
            "-n",
        ])
        .arg(&source);
    checked(&mut command).await?;
    for (kind, name, extra) in [
        (
            "dash",
            "fixture.mpd",
            vec![
                "-seg_duration",
                "1",
                "-use_template",
                "1",
                "-use_timeline",
                "1",
            ],
        ),
        (
            "dash",
            "builtin.mpd",
            vec![
                "-seg_duration",
                "1",
                "-format_options",
                "use_editlist=0",
                "-init_seg_name",
                "builtin-init-$RepresentationID$.m4s",
                "-media_seg_name",
                "builtin-chunk-$RepresentationID$-$Number%05d$.m4s",
            ],
        ),
        ("mpegts", "fixture.ts", vec![]),
        (
            "hls",
            "fixture.m3u8",
            vec![
                "-hls_time",
                "1",
                "-hls_playlist_type",
                "vod",
                "-hls_segment_filename",
                "part-%02d.ts",
            ],
        ),
    ] {
        let mut command = create_hidden_tokio_command(&tools.ffmpeg);
        command.current_dir(root);
        command
            .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
            .arg(&source)
            .args(["-map", "0", "-c", "copy"])
            .args(extra)
            .args(["-f", kind, "-n"])
            .arg(root.join(name));
        checked(&mut command).await?;
    }
    Ok(())
}

pub(crate) async fn checked(command: &mut tokio::process::Command) -> Result<Vec<u8>, String> {
    checked_with_timeout(command, Duration::from_secs(30)).await
}

pub(crate) async fn checked_with_timeout(
    command: &mut tokio::process::Command,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let output =
        process::capture(command, &CancellationToken::new(), timeout, 2 * 1024 * 1024).await?;
    if !output.status.success() {
        return Err(format!(
            "本地媒体测试工具执行失败：{}",
            crate::manager::redact_sensitive(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    Ok(output.stdout)
}
