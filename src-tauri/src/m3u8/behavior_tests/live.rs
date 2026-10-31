use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn live(sequence: u64, paths: &[&str], ended: bool, init: bool) -> String {
    format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:0.1\n#EXT-X-MEDIA-SEQUENCE:{sequence}\n{}{}{}",
        if init {
            "#EXT-X-MAP:URI=\"init\"\n"
        } else {
            ""
        },
        paths
            .iter()
            .map(|path| format!("#EXTINF:0.1,\n{path}\n"))
            .collect::<String>(),
        if ended { "#EXT-X-ENDLIST\n" } else { "" }
    )
}

#[tokio::test]
async fn live_refreshes_deduplicates_and_finishes_only_at_endlist() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let fixture = Fixture::start(move |request| {
        if request.path == "/list.m3u8" {
            let poll = counter.fetch_add(1, Ordering::Relaxed);
            return Reply::ok(match poll {
                0 => live(100, &["a"], false, false),
                1 => live(100, &["a", "b"], false, false),
                _ => live(101, &["b", "c"], true, false),
            });
        }
        Reply::ok(match request.path.as_str() {
            "/a" => b"A",
            "/b" => b"B",
            "/c" => b"C",
            _ => b"?",
        })
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    let outcome = download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    assert!(!outcome.stopped_live);
    assert_eq!(outcome.bytes, 3);
    assert_eq!(read_output(&path).await, b"ABC");
    assert!(polls.load(Ordering::Relaxed) >= 3);
    for part in ["/a", "/b", "/c"] {
        assert_eq!(fixture.count("GET", part), 1);
    }
}

#[tokio::test]
async fn explicit_live_stop_saves_verified_contiguous_recording() {
    let fixture = Fixture::start(|request| {
        if request.path == "/list.m3u8" {
            Reply::ok(live(100, &["a", "b"], false, false))
        } else {
            Reply::ok(if request.path == "/a" { b"A" } else { b"B" })
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    let cancel = CancellationToken::new();
    let opts = options(&fixture.url, cancel.clone());
    let finalize = opts.finalize_live.clone();
    let progress = opts.http.progress.clone();
    let target = path.clone();
    let handle = tokio::spawn(async move { download(&opts, &target).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while progress.saved_bytes.load(Ordering::Relaxed) < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(!handle.is_finished(), "无 ENDLIST 的直播不能首窗口即完成");
    assert!(progress.is_live.load(Ordering::Acquire),"only a parsed media playlist without ENDLIST identifies recording");
    finalize.store(true, Ordering::Release);
    cancel.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(outcome.stopped_live);
    assert_eq!(read_output(&path).await, b"AB");
    assert_eq!(progress.active_connections.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn live_cancel_exit_and_timeout_do_not_finalize_without_explicit_permission() {
    let fixture = Fixture::start(|request| {
        if request.path == "/list.m3u8" {
            Reply::ok(live(100, &["a"], false, false))
        } else {
            Reply::ok(b"A")
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    let cancel = CancellationToken::new();
    let opts = options(&fixture.url, cancel.clone());
    let progress = opts.http.progress.clone();
    let target = path.clone();
    let handle = tokio::spawn(async move { download(&opts, &target).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while progress.saved_bytes.load(Ordering::Relaxed) < 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(!path.exists());
    assert_eq!(
        read_output(&resume::segment_part_path(&path, 0)).await,
        b"A"
    );
}

#[tokio::test]
async fn live_inactivity_is_bounded_and_does_not_report_completion() {
    let fixture = Fixture::start(|request| {
        if request.path == "/list.m3u8" {
            Reply::ok(live(100, &["a"], false, false))
        } else {
            Reply::ok(b"A")
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    let mut opts = options(&fixture.url, CancellationToken::new());
    opts.live_idle_timeout = Duration::from_millis(100);
    let outcome = tokio::time::timeout(Duration::from_secs(2), download(&opts, &path))
        .await
        .unwrap();
    assert!(outcome.unwrap_err().contains("长时间未更新"));
    assert!(!path.exists());
    assert_eq!(
        read_output(&resume::segment_part_path(&path, 0)).await,
        b"A"
    );
}

#[tokio::test]
async fn rolling_fmp4_map_is_prepended_once_and_changes_are_preserved() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let fixture = Fixture::start(move |request| {
        if request.path == "/list.m3u8" {
            return Reply::ok(if counter.fetch_add(1, Ordering::Relaxed) == 0 {
                live(100, &["a"], false, true)
            } else {
                live(101, &["b"], true, true)
            });
        }
        Reply::ok(match request.path.as_str() {
            "/init" => b"INIT".as_slice(),
            "/a" => b"A",
            _ => b"B",
        })
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    assert_eq!(read_output(&path).await, b"INITAB");
    assert_eq!(fixture.count("GET", "/init"), 1);
}

#[tokio::test]
async fn reused_live_sequence_with_changed_url_is_rejected() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let fixture = Fixture::start(move |request| {
        if request.path == "/list.m3u8" {
            Reply::ok(live(
                100,
                if counter.fetch_add(1, Ordering::Relaxed) == 0 {
                    &["a"]
                } else {
                    &["different"]
                },
                false,
                false,
            ))
        } else {
            Reply::ok(b"A")
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    let outcome = download(&options(&fixture.url, CancellationToken::new()), &path).await;
    assert!(outcome.unwrap_err().starts_with("REMOTE_CHANGED:"));
    assert!(!path.exists());
}

#[tokio::test]
async fn fmp4_map_changes_are_kept_in_correct_media_order() {
    let fixture = Fixture::start(|request| match request.path.as_str() {
        "/list.m3u8" => Reply::ok("#EXTM3U\n#EXT-X-MAP:URI=\"init-a\"\n#EXTINF:1,\na\n#EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI=\"init-b\"\n#EXTINF:1,\nb\n#EXT-X-ENDLIST\n"),
        "/init-a" => Reply::ok(b"INIT_A"), "/init-b" => Reply::ok(b"INIT_B"),
        "/a" => Reply::ok(b"A"), _ => Reply::ok(b"B"),
    }).await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    assert_eq!(read_output(&path).await, b"INIT_AAINIT_BB");
}
