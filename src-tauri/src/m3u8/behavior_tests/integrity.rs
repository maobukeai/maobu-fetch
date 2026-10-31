use super::*;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

#[tokio::test]
async fn range_requires_exact_206_and_merges_verified_bytes() {
    let playlist = "#EXTM3U\n#EXT-X-BYTERANGE:4@0\n#EXTINF:1,\nblob\n#EXT-X-BYTERANGE:4@4\n#EXTINF:1,\nblob\n#EXT-X-ENDLIST\n";
    for mode in ["valid", "ignored", "wrong", "missing", "416"] {
        let fixture = Fixture::start(move |request| {
            if request.path == "/list.m3u8" {
                return Reply::ok(playlist);
            }
            let range = request.headers.get("range").unwrap();
            let start = if range.starts_with("bytes=0-") { 0 } else { 4 };
            let mut reply = Reply::ok(&b"AAAABBBB"[start..start + 4]);
            reply.status = if mode == "ignored" {
                200
            } else if mode == "416" {
                416
            } else {
                206
            };
            if mode != "missing" {
                let start = if mode == "wrong" { start + 1 } else { start };
                reply.headers.push((
                    "Content-Range".into(),
                    format!("bytes {start}-{}/8", start + 3),
                ));
            }
            reply
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let path = output(&dir);
        let result = download(&options(&fixture.url, CancellationToken::new()), &path).await;
        if mode == "valid" {
            assert_eq!(result.unwrap().bytes, 8);
            assert_eq!(read_output(&path).await, b"AAAABBBB");
            assert_eq!(
                resume::hash_file(&path).await.unwrap(),
                resume::digest(b"AAAABBBB")
            );
        } else {
            assert!(result.unwrap_err().starts_with("M3U8 校验失败"), "{mode}");
            assert!(!path.exists());
        }
    }
}

#[tokio::test]
async fn rejects_drm_and_malformed_crypto_before_requesting_media() {
    for declaration in [
        "METHOD=SAMPLE-AES,URI=\"key\"",
        "METHOD=FAKE,URI=\"key\"",
        "METHOD=AES-128,KEYFORMAT=\"com.apple.streamingkeydelivery\",URI=\"key\"",
        "METHOD=AES-128",
        "METHOD=AES-128,URI=\"key\",IV=nothex",
    ] {
        let playlist =
            format!("#EXTM3U\n#EXT-X-KEY:{declaration}\n#EXTINF:1,\nblob\n#EXT-X-ENDLIST\n");
        let fixture = Fixture::start(move |_| Reply::ok(&playlist)).await;
        let dir = tempfile::tempdir().unwrap();
        assert!(download(
            &options(&fixture.url, CancellationToken::new()),
            &output(&dir)
        )
        .await
        .is_err());
        assert_eq!(fixture.count("GET", "/blob"), 0);
    }
}

#[tokio::test]
async fn merge_failure_preserves_old_target_and_verified_parts() {
    let fixture = Fixture::start(|request| {
        if request.path == "/list.m3u8" {
            Reply::ok(vod(&["blob"]))
        } else {
            Reply::ok(b"CORRECT")
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    tokio::fs::write(&path, b"STALE").await.unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    let result = download(&options(&fixture.url, CancellationToken::new()), &path).await;
    assert!(result.is_err());
    assert_eq!(read_output(&path).await, b"STALE");
    assert_eq!(
        read_output(&resume::segment_part_path(&path, 0)).await,
        b"CORRECT"
    );
    assert!(resume::journal_path(&path).exists());
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&path, permissions).unwrap();
}

#[tokio::test]
async fn unknown_length_stream_is_complete_and_partial_body_never_completes() {
    for truncated in [false, true] {
        let fixture = Fixture::start(move |request| {
            if request.path == "/list.m3u8" {
                return Reply::ok(vod(&["blob"]));
            }
            let mut reply = Reply::ok(b"DATA");
            reply.include_length = false;
            if truncated {
                reply.headers.push(("Content-Length".into(), "8".into()));
            }
            reply
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let path = output(&dir);
        let result = download(&options(&fixture.url, CancellationToken::new()), &path).await;
        if truncated {
            assert!(result.is_err());
            assert!(!path.exists());
        } else {
            assert_eq!(result.unwrap().bytes, 4);
            assert_eq!(read_output(&path).await, b"DATA");
        }
    }
}

#[tokio::test]
async fn user_cancel_interrupts_silent_headers_and_body() {
    for silent_headers in [false, true] {
        let fixture = Fixture::start(move |request| {
            if request.path == "/list.m3u8" {
                return Reply::ok(vod(&["blob"]));
            }
            let mut reply = Reply::ok(b"DATA");
            if silent_headers {
                reply.header_delay = Duration::from_secs(60);
            } else {
                reply.body_delay = Duration::from_secs(60);
            }
            reply
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let path = output(&dir);
        let cancel = CancellationToken::new();
        let opts = options(&fixture.url, cancel.clone());
        let result = tokio::spawn(async move {
            let result = download(&opts, &path).await;
            (result, opts)
        });
        while fixture.count("GET", "/blob") == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let started = Instant::now();
        cancel.cancel();
        let (outcome, opts) = tokio::time::timeout(Duration::from_millis(500), result)
            .await
            .unwrap()
            .unwrap();
        assert!(outcome.is_err());
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(
            opts.http
                .progress
                .active_connections
                .load(Ordering::Relaxed),
            0
        );
    }
}

#[tokio::test]
async fn failed_worker_immediately_stops_other_silent_workers_without_cancelling_user_token() {
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    let fixture = Fixture::start(move |request| {
        if request.path == "/list.m3u8" {
            return Reply::ok(vod(&["silent", "fail"]));
        }
        counter.fetch_add(1, Ordering::Relaxed);
        let mut reply = Reply::ok(b"DATA");
        if request.path == "/silent" {
            reply.body_delay = Duration::from_secs(60);
        } else {
            reply.status = 403;
        }
        reply
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        download(&options(&fixture.url, cancel.clone()), &output(&dir)),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(!cancel.is_cancelled());
    assert!(seen.load(Ordering::Relaxed) >= 1);
}
