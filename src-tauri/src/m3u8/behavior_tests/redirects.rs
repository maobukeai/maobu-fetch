use super::*;

fn redirect(location: &str) -> Reply {
    let mut reply = Reply::ok([]);
    reply.status = 302;
    reply.headers.push(("Location".into(), location.into()));
    reply
}

#[tokio::test]
async fn redirects_retain_same_origin_credentials_and_strip_cross_origin_credentials() {
    let media = Fixture::start(|_| Reply::etag(b"MEDIA", "\"same\"")).await;
    let remote = media.url.replace("list.m3u8", "media");
    let source = Fixture::start(move |request| match request.path.as_str() {
        "/list.m3u8" => redirect("nested/index.m3u8"),
        "/nested/index.m3u8" => Reply::ok(vod(&["../redirect-media"])),
        "/redirect-media" => redirect(&remote),
        _ => panic!("unexpected synthetic path"),
    })
    .await;
    let mut opts = options(&source.url, CancellationToken::new());
    for (name, value) in [
        ("Cookie", "synthetic=source"),
        ("Authorization", "Bearer synthetic"),
        ("X-API-Key", "synthetic-private"),
        ("Referer", "https://example.invalid/watch?private=synthetic"),
    ] {
        opts.http.headers.insert(name.into(), value.into());
    }
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    download(&opts, &path).await.unwrap();
    assert_eq!(read_output(&path).await, b"MEDIA");
    assert!(source.requests.lock().unwrap().iter().all(|request| request
        .headers
        .get("cookie")
        .is_some_and(|v| v == "synthetic=source")
        && request.headers.contains_key("authorization")
        && request.headers.contains_key("x-api-key")));
    assert!(media.requests.lock().unwrap().iter().all(|request| !request
        .headers
        .contains_key("cookie")
        && !request.headers.contains_key("authorization")
        && !request.headers.contains_key("x-api-key")
        && request
            .headers
            .get("referer")
            .is_some_and(|v| v == "https://example.invalid/")));
    // Reopen exercises the same redirect protection on cached HEAD validation.
    download(&opts, &path).await.unwrap();
    assert_eq!(media.count("HEAD", "/media"), 1);
}

#[tokio::test]
async fn redirect_loops_unsupported_schemes_and_long_chains_are_bounded() {
    for kind in 0..3 {
        let source = Fixture::start(move |request| match kind {
            0 => redirect("/list.m3u8"),
            1 => redirect("file:///private"),
            _ => {
                let next = request
                    .path
                    .trim_start_matches('/')
                    .parse::<usize>()
                    .unwrap_or(0)
                    + 1;
                redirect(&format!("/{next}"))
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let result = download(
            &options(&source.url, CancellationToken::new()),
            &output(&dir),
        )
        .await;
        assert!(result.unwrap_err().starts_with("M3U8 校验失败：重定向"));
        assert!(source.requests.lock().unwrap().len() <= 11);
    }
}

#[tokio::test]
async fn cookie_jar_is_reformatted_for_each_redirect_path_without_sending_the_raw_jar() {
    let media = Fixture::start(|_| Reply::etag(b"JAR-MEDIA", "\"jar\"")).await;
    let remote = media.url.replace("list.m3u8", "media");
    let source = Fixture::start(move |request| match request.path.as_str() {
        "/list.m3u8" => Reply::ok(vod(&["jump"])),
        "/jump" => redirect(&remote),
        _ => panic!("unexpected synthetic path"),
    })
    .await;
    let jar = "# Netscape HTTP Cookie File\n127.0.0.1\tFALSE\t/list.m3u8\tFALSE\t0\tplaylist\tsynthetic-list\n127.0.0.1\tFALSE\t/media\tFALSE\t0\tmedia\tsynthetic-media\n127.0.0.1\tFALSE\t/\tFALSE\t0\troot\tsynthetic-root\n127.0.0.1\tFALSE\t/\tFALSE\t1\texpired\tnever\nforeign.invalid\tFALSE\t/\tFALSE\t0\tforeign\tnever";
    let mut opts = options(&source.url, CancellationToken::new());
    opts.http.headers.insert("Cookie".into(), jar.into());
    opts.http
        .headers
        .insert("Authorization".into(), "Bearer synthetic".into());
    let directory = tempfile::tempdir().unwrap();
    let target = output(&directory);
    download(&opts, &target).await.unwrap();
    assert_eq!(read_output(&target).await, b"JAR-MEDIA");
    for request in source.requests.lock().unwrap().iter() {
        let expected = if request.path == "/list.m3u8" {
            "playlist=synthetic-list; root=synthetic-root"
        } else {
            "root=synthetic-root"
        };
        assert_eq!(request.headers.get("cookie").unwrap(), expected);
    }
    for request in media.requests.lock().unwrap().iter() {
        assert_eq!(
            request.headers.get("cookie").unwrap(),
            "media=synthetic-media; root=synthetic-root"
        );
        assert!(!request.headers.contains_key("authorization"));
    }
}
