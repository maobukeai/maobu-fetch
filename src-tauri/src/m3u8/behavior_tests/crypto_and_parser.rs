use super::super::{crypto::decrypt_aes_128, parser::parse_m3u8};
use super::*;
use std::sync::atomic::Ordering;

fn encrypted(plaintext: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
    use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    let mut buffer = vec![0; plaintext.len() + 16];
    buffer[..plaintext.len()].copy_from_slice(plaintext);
    cbc::Encryptor::<aes::Aes128>::new(key.into(), iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
        .unwrap()
        .to_vec()
}

#[tokio::test]
async fn aes128_download_decrypts_and_bad_padding_never_creates_output() {
    for bad_iv in [false, true] {
        let key = [0x42u8; 16];
        let iv = [0u8; 16];
        let ciphertext = encrypted(b"SECRET7", &key, &iv);
        let fixture = Fixture::start(move |request| match request.path.as_str() {
            "/list.m3u8" => Reply::ok(format!("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\",IV=0x{}\n#EXTINF:1,\nblob\n#EXT-X-ENDLIST\n", if bad_iv { "1" } else { "0" })),
            "/key" => Reply::ok(key), _ => Reply::ok(&ciphertext),
        }).await;
        let dir = tempfile::tempdir().unwrap();
        let path = output(&dir);
        let result = download(&options(&fixture.url, CancellationToken::new()), &path).await;
        if bad_iv {
            assert!(result.is_err());
            assert!(!path.exists());
        } else {
            assert_eq!(result.unwrap().bytes, 7);
            assert_eq!(read_output(&path).await, b"SECRET7");
        }
    }
}

#[test]
fn crypto_rejects_invalid_padding_instead_of_using_unchecked_no_padding() {
    let key = [0x42u8; 16];
    let iv = [0u8; 16];
    let ciphertext = encrypted(b"SECRET7", &key, &iv);
    let mut wrong_iv = iv;
    wrong_iv[15] = 1;
    assert!(decrypt_aes_128(&key, &wrong_iv, &ciphertext).is_err());
}

#[tokio::test]
async fn unknown_length_encrypted_workers_share_memory_without_key_deadlock() {
    let key = [0x42u8; 16];
    let iv = [0u8; 16];
    let ciphertext = encrypted(b"PLAIN", &key, &iv);
    let fixture = Fixture::start(move |request| match request.path.as_str() {
        "/list.m3u8" => Reply::ok("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\",IV=0x0\n#EXTINF:1,\na\n#EXTINF:1,\nb\n#EXT-X-ENDLIST\n"),
        "/key" => Reply::ok(key),
        _ => { let mut reply = Reply::ok(&ciphertext); reply.include_length = false; reply },
    }).await;
    let dir = tempfile::tempdir().unwrap();
    let path = output(&dir);
    tokio::time::timeout(
        Duration::from_secs(2),
        download(&options(&fixture.url, CancellationToken::new()), &path),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(read_output(&path).await, b"PLAINPLAIN");
}

#[test]
fn parser_rejects_ambiguous_ranges_keys_and_untrusted_protocols() {
    for playlist in [
        "#EXTM3UX\n#EXTINF:1,\na\n",
        "#EXTM3U\n#EXT-X-BYTERANGE:0@0\n#EXTINF:1,\na\n",
        "#EXTM3U\n#EXT-X-BYTERANGE:4\n#EXTINF:1,\na\n",
        "#EXTM3U\n#EXT-X-BYTERANGE:4@0\n#EXTINF:1,\na\n#EXT-X-BYTERANGE:4\n#EXTINF:1,\nb\n",
        "#EXTM3U\n#EXT-X-KEY:METHOD=NONE,URI=\"key\"\n#EXTINF:1,\na\n",
        "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\",METHOD=NONE\n#EXTINF:1,\na\n",
        "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"unterminated\n#EXTINF:1,\na\n",
        "#EXTM3U\n#EXT-X-SESSION-KEY:METHOD=SAMPLE-AES,URI=\"key\"\n#EXT-X-STREAM-INF:BANDWIDTH=10\na\n",
        "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXT-X-MAP:URI=\"init\"\n#EXTINF:1,\na\n",
        "#EXTM3U\n#EXT-X-MAP:BYTERANGE=\"4@0\"\n#EXTINF:1,\na\n",
        "#EXTM3U\n#EXTINF:1,\nfile:///private\n",
        "#EXTM3U\n#EXTINF:NaN,\na\n",
        "#EXTM3U\n#EXTINF:1,\n",
    ] { assert!(parse_m3u8(playlist, "https://example.invalid/list.m3u8").is_err(), "{playlist}"); }
}

#[test]
fn master_rejects_external_or_missing_audio_groups_but_keeps_muxed_audio() {
    let external="#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Audio\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=10,AUDIO=\"a\"\nvideo.m3u8\n";
    assert!(parse_m3u8(external, "https://example.invalid/master.m3u8")
        .err()
        .unwrap()
        .contains("外部独立音轨"));
    let dangling = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10,AUDIO=\"missing\"\nvideo.m3u8\n";
    assert!(parse_m3u8(dangling, "https://example.invalid/master.m3u8").is_err());
    let muxed="#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"inband\",NAME=\"Muxed\"\n#EXT-X-STREAM-INF:BANDWIDTH=10,AUDIO=\"inband\"\nmuxed.m3u8\n";
    assert!(parse_m3u8(muxed, "https://example.invalid/master.m3u8").is_ok());
}

#[tokio::test]
async fn external_master_audio_never_downloads_a_video_only_file_as_success() {
    let fixture=Fixture::start(|request|match request.path.as_str(){
        "/list.m3u8"=>Reply::ok("#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Audio\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=10,AUDIO=\"a\"\nvideo.m3u8\n"),
        "/video.m3u8"=>Reply::ok(vod(&["silent-video"])),
        "/audio.m3u8"=>Reply::ok(vod(&["audio-data"])),
        _=>Reply::ok(b"VIDEO-ONLY"),
    }).await;
    let directory = tempfile::tempdir().unwrap();
    let target = output(&directory);
    let opts = options(&fixture.url, CancellationToken::new());
    let error = download(&opts, &target).await.unwrap_err();
    assert!(error.contains("外部独立音轨"));
    assert!(!target.exists());
    assert_eq!(
        fixture.requests.lock().unwrap().len(),
        1,
        "unsupported external audio must fail before fetching either child playlist or media bytes"
    );
    assert!(
        !opts.http.progress.is_live.load(Ordering::Acquire),
        "a Master URL alone cannot classify a task as live"
    );
}

#[tokio::test]
async fn hls_does_not_forward_source_cookie_or_authorization_to_other_origins() {
    let media = Fixture::start(|_| Reply::ok(b"MEDIA")).await;
    let media_url = media.url.replace("list.m3u8", "blob");
    let playlist = Fixture::start(move |_| Reply::ok(vod(&[&media_url]))).await;
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options(&playlist.url, CancellationToken::new());
    opts.http
        .headers
        .insert("Cookie".into(), "synthetic_session=source".into());
    opts.http
        .headers
        .insert("Authorization".into(), "Bearer synthetic".into());
    opts.http
        .headers
        .insert("X-API-Key".into(), "synthetic-private-key".into());
    opts.http.headers.insert(
        "Referer".into(),
        "https://example.invalid/watch?token=synthetic".into(),
    );
    download(&opts, &output(&dir)).await.unwrap();
    assert!(playlist.requests.lock().unwrap()[0]
        .headers
        .contains_key("cookie"));
    assert!(media.requests.lock().unwrap().iter().all(|request| !request
        .headers
        .contains_key("cookie")
        && !request.headers.contains_key("authorization")
        && !request.headers.contains_key("x-api-key")
        && request
            .headers
            .get("referer")
            .is_some_and(|value| value == "https://example.invalid/")));
}
