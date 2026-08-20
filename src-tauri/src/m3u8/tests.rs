use super::crypto::*;
use super::parser::*;

#[test]
fn test_parse_master_playlist_and_select_best() {
    let master_content = r#"
#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360
http://example.com/low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1400000,RESOLUTION=842x480
http://example.com/mid.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720,CODECS="avc1.4d401f,mp4a.40.2"
http://example.com/high.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080
http://example.com/fullhd.m3u8
"#;

    let base = "http://example.com/index.m3u8";
    let parsed = parse_m3u8(master_content, base).expect("should parse master playlist");

    match parsed {
        ParsedPlaylist::Master(variants) => {
            assert_eq!(variants.len(), 4);
            assert_eq!(variants[0].bandwidth, Some(800000));
            assert_eq!(variants[0].resolution, Some((640, 360)));
            assert_eq!(variants[2].codecs.as_deref(), Some("avc1.4d401f,mp4a.40.2"));

            let best = select_best_variant(&variants).expect("should select best variant");
            assert_eq!(best, "http://example.com/fullhd.m3u8");
        }
        ParsedPlaylist::Media(_) => panic!("expected Master playlist"),
    }
}

#[test]
fn test_parse_media_playlist_with_aes_and_byterange() {
    let media_content = r#"
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXT-X-MEDIA-SEQUENCE:100
#EXT-X-KEY:METHOD=AES-128,URI="https://priv.example.com/key.bin",IV=0x0123456789abcdef0123456789abcdef
#EXTINF:9.009,
segment100.ts
#EXT-X-BYTERANGE:1000@500
#EXTINF:9.009,
segment101.ts
#EXT-X-ENDLIST
"#;

    let base = "https://example.com/video/playlist.m3u8";
    let parsed = parse_m3u8(media_content, base).expect("should parse media playlist");

    match parsed {
        ParsedPlaylist::Media(media) => {
            assert_eq!(media.target_duration, Some(10.0));
            assert_eq!(media.media_sequence, 100);
            assert!(media.is_endlist);
            assert_eq!(media.segments.len(), 2);

            let s0 = &media.segments[0];
            assert_eq!(s0.index, 0);
            assert_eq!(s0.sequence, 100);
            assert_eq!(s0.duration, 9.009);
            assert_eq!(s0.url, "https://example.com/video/segment100.ts");
            assert!(s0.byte_range.is_none());

            let key0 = s0.key.as_ref().expect("should have key");
            assert_eq!(key0.method, EncryptionMethod::Aes128);
            assert_eq!(key0.uri, "https://priv.example.com/key.bin");
            let expected_iv = [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
            ];
            assert_eq!(key0.iv, Some(expected_iv));

            let s1 = &media.segments[1];
            assert_eq!(s1.index, 1);
            assert_eq!(s1.sequence, 101);
            assert_eq!(s1.byte_range, Some((1000, Some(500))));
        }
        ParsedPlaylist::Master(_) => panic!("expected Media playlist"),
    }
}

#[test]
fn test_parse_hex_iv() {
    let raw = "0x00000000000000000000000000000001";
    let parsed = parse_hex_iv(raw).expect("should parse valid hex iv");
    let mut expected = [0u8; 16];
    expected[15] = 1;
    assert_eq!(parsed, expected);

    // 测试未加 0x 前缀且不带大写
    let raw2 = "aabbccddeeff00112233445566778899";
    let parsed2 = parse_hex_iv(raw2).expect("should parse raw hex");
    assert_eq!(parsed2[0], 0xaa);
    assert_eq!(parsed2[15], 0x99);
}

#[test]
fn test_derive_iv_from_sequence() {
    let seq = 42u64;
    let iv = derive_iv_from_sequence(seq);
    let mut expected = [0u8; 16];
    expected[8..16].copy_from_slice(&42u64.to_be_bytes());
    assert_eq!(iv, expected);
}

#[test]
fn test_aes_128_cbc_round_trip() {
    use aes::Aes128;
    use cbc::Encryptor;
    use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};

    type Aes128CbcEnc = Encryptor<Aes128>;

    let key = [0x42u8; 16];
    let iv = [0x24u8; 16];
    let plaintext = b"Hello, this is a test segment payload for HLS AES-128 decryption verification!";

    // 加密
    let mut enc_buf = vec![0u8; plaintext.len() + 16];
    enc_buf[..plaintext.len()].copy_from_slice(plaintext);
    let encryptor = Aes128CbcEnc::new((&key).into(), (&iv).into());
    let ciphertext = encryptor
        .encrypt_padded_mut::<Pkcs7>(&mut enc_buf, plaintext.len())
        .expect("encryption failed");

    // 原生解密验证
    let decrypted = decrypt_aes_128(&key, &iv, ciphertext).expect("decryption failed");
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_relative_url_resolution() {
    let base = "https://cdn.example.com/hls/master.m3u8";
    assert_eq!(
        resolve_url(base, "sub/media.m3u8").unwrap(),
        "https://cdn.example.com/hls/sub/media.m3u8"
    );
    assert_eq!(
        resolve_url(base, "../other/media.m3u8").unwrap(),
        "https://cdn.example.com/other/media.m3u8"
    );
    assert_eq!(
        resolve_url(base, "/root/media.m3u8").unwrap(),
        "https://cdn.example.com/root/media.m3u8"
    );
    assert_eq!(
        resolve_url(base, "http://absolute.com/stream.m3u8").unwrap(),
        "http://absolute.com/stream.m3u8"
    );
}
