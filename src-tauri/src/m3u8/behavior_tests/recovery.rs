use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn unjournaled_and_corrupted_parts_are_redownloaded() {
    let fixture = Fixture::start(|request| {
        if request.path == "/list.m3u8" {
            Reply::ok(vod(&["blob"]))
        } else {
            Reply::etag(b"CORRECT", "\"same\"")
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    tokio::fs::write(resume::segment_part_path(&path, 0), b"BAD")
        .await
        .unwrap();
    download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    assert_eq!(read_output(&path).await, b"CORRECT");
    tokio::fs::write(resume::segment_part_path(&path, 0), b"DAMAGED")
        .await
        .unwrap();
    download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    assert_eq!(fixture.count("GET", "/blob"), 2);
    assert_eq!(read_output(&path).await, b"CORRECT");
}

#[tokio::test]
async fn restart_reuses_only_local_hash_and_remote_validator_verified_parts() {
    let failing = Arc::new(AtomicBool::new(true));
    let switch = failing.clone();
    let fixture = Fixture::start(move |request| {
        if request.path == "/list.m3u8" {
            return Reply::ok(vod(&["a", "b"]));
        }
        if request.path == "/b" && switch.load(Ordering::Relaxed) {
            let mut reply = Reply::ok(b"FAIL");
            reply.status = 500;
            reply.header_delay = Duration::from_millis(50);
            return reply;
        }
        Reply::etag(if request.path == "/a" { b"A" } else { b"B" }, "\"stable\"")
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    assert!(
        download(&options(&fixture.url, CancellationToken::new()), &path)
            .await
            .is_err()
    );
    assert_eq!(
        read_output(&resume::segment_part_path(&path, 0)).await,
        b"A"
    );
    failing.store(false, Ordering::Relaxed);
    download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    assert_eq!(read_output(&path).await, b"AB");
    assert_eq!(fixture.count("GET", "/a"), 1);
    assert_eq!(fixture.count("HEAD", "/a"), 1);
}

#[tokio::test]
async fn etag_case_change_is_remote_change_and_old_data_is_preserved() {
    let changed = Arc::new(AtomicBool::new(false));
    let switch = changed.clone();
    let fixture = Fixture::start(move |request| {
        if request.path == "/list.m3u8" {
            Reply::ok(vod(&["blob"]))
        } else if switch.load(Ordering::Relaxed) {
            Reply::etag(b"NEW", "\"abc\"")
        } else {
            Reply::etag(b"OLD", "\"ABC\"")
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    changed.store(true, Ordering::Relaxed);
    let outcome = download(&options(&fixture.url, CancellationToken::new()), &path).await;
    assert!(outcome.unwrap_err().starts_with("REMOTE_CHANGED:"));
    assert_eq!(read_output(&path).await, b"OLD");
    assert_eq!(
        read_output(&resume::segment_part_path(&path, 0)).await,
        b"OLD"
    );
}

#[tokio::test]
async fn changed_playlist_preserves_journal_and_parts() {
    let changed = Arc::new(AtomicBool::new(false));
    let switch = changed.clone();
    let fixture = Fixture::start(move |request| {
        if request.path == "/list.m3u8" {
            Reply::ok(vod(if switch.load(Ordering::Relaxed) {
                &["different"]
            } else {
                &["blob"]
            }))
        } else {
            Reply::etag(b"OLD", "\"stable\"")
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    let old_manifest = read_output(&resume::journal_path(&path)).await;
    changed.store(true, Ordering::Relaxed);
    assert!(
        download(&options(&fixture.url, CancellationToken::new()), &path)
            .await
            .unwrap_err()
            .starts_with("REMOTE_CHANGED:")
    );
    assert_eq!(
        read_output(&resume::journal_path(&path)).await,
        old_manifest
    );
    assert_eq!(
        read_output(&resume::segment_part_path(&path, 0)).await,
        b"OLD"
    );
}

#[tokio::test]
async fn last_modified_change_without_etag_prevents_reusing_old_media() {
    let changed = Arc::new(AtomicBool::new(false));
    let switch = changed.clone();
    let fixture = Fixture::start(move |request| {
        if request.path == "/list.m3u8" {
            Reply::ok(vod(&["blob"]))
        } else {
            let mut reply = Reply::ok(b"OLD");
            reply.headers.push((
                "Last-Modified".into(),
                if switch.load(Ordering::Relaxed) {
                    "Wed, 21 Oct 2015 07:29:00 GMT"
                } else {
                    "Wed, 21 Oct 2015 07:28:00 GMT"
                }
                .into(),
            ));
            reply
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    download(&options(&fixture.url, CancellationToken::new()), &path)
        .await
        .unwrap();
    changed.store(true, Ordering::Relaxed);
    assert!(
        download(&options(&fixture.url, CancellationToken::new()), &path)
            .await
            .unwrap_err()
            .starts_with("REMOTE_CHANGED:")
    );
    assert_eq!(read_output(&path).await, b"OLD");
}

#[tokio::test]
async fn merge_rechecks_hash_and_refuses_same_length_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    let part = resume::segment_part_path(&path, 0);
    tokio::fs::write(&part, b"DAMAGED").await.unwrap();
    let entry = resume::SegmentRecord {
        index: 0,
        identity: String::new(),
        sequence: Some(0),
        length: 7,
        sha256: resume::digest(b"CORRECT"),
        etag: None,
        last_modified: None,
    };
    let result =
        super::super::merge::merge_segments(&path, &[entry], &CancellationToken::new()).await;
    assert!(result.unwrap_err().contains("哈希校验失败"));
    assert!(!path.exists());
    assert_eq!(read_output(&part).await, b"DAMAGED");
}
