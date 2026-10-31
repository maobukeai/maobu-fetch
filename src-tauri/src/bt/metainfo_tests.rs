use super::*;

fn single(name: &str, length: u64, piece_length: u32, hashes: usize) -> Vec<u8> {
    let mut bytes = format!(
        "d4:infod6:lengthi{length}e4:name{}:{name}12:piece lengthi{piece_length}e6:pieces{}:",
        name.len(),
        hashes * 20
    )
    .into_bytes();
    bytes.extend(vec![0; hashes * 20]);
    bytes.extend_from_slice(b"ee");
    bytes
}

fn multi(entries: &[(&str, u64)]) -> Vec<u8> {
    let mut bytes = b"d4:infod5:filesl".to_vec();
    for (name, length) in entries {
        bytes.extend(format!("d6:lengthi{length}e4:pathl{}:{name}ee", name.len()).bytes());
    }
    bytes.extend_from_slice(b"e4:name4:root12:piece lengthi16384e6:pieces20:");
    bytes.extend([0; 20]);
    bytes.extend_from_slice(b"ee");
    bytes
}

#[test]
fn malformed_sizes_paths_hashes_and_windows_collisions_are_rejected() {
    assert!(inspect_torrent_bytes(&single("valid.bin", 1, 16_384, 1)).is_ok());
    for bytes in [
        single("bad.bin", 1, 0, 1),
        single("bad.bin", 1, 16_384, 0),
        single("../escape", 1, 16_384, 1),
        single("CON.txt", 1, 16_384, 1),
        single("bad.bin", u64::MAX, 16_384, 1),
        multi(&[("one", u64::MAX), ("two", 1)]),
        multi(&[("Same.bin", 1), ("same.bin", 1)]),
        b"d8:announce".to_vec(),
    ] {
        assert!(inspect_torrent_bytes(&bytes).is_err());
    }
    assert!(validate_torrent_bytes(&vec![b'd'; 20 * 1024 * 1024 + 1]).is_err());
}

#[tokio::test]
async fn native_metadata_shape_rejects_zero_divisor_without_panicking() {
    let temp = tempfile::tempdir().unwrap();
    let engine = BtEngine::with_data_dir(temp.path().join("session"));
    let session = engine.get_or_init_session(None).await.unwrap();
    for bad in [
        single("bad.bin", 1, 0, 1),
        multi(&[("one", u64::MAX), ("two", 1)]),
    ] {
        assert!(session
            .add_torrent(
                librqbit::AddTorrent::from_bytes(bad),
                Some(librqbit::AddTorrentOptions {
                    list_only: true,
                    ..Default::default()
                })
            )
            .await
            .is_err());
    }
    engine.shutdown().await.unwrap();
}
