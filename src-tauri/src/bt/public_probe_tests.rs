//! 显式公网探针只向此纯元数据测试传入临时文件；不创建 Session 或桌面客户端。
#[test]
#[ignore = "仅由获授权的Ubuntu公网探针传入受限临时元数据文件"]
fn inspect_downloaded_official_torrent_metadata() {
    use sha2::{Digest, Sha256};
    let path = std::path::PathBuf::from(
        std::env::var_os("MAOBU_PUBLIC_TORRENT_PATH").expect("explicit probe path"),
    );
    let path = path.canonicalize().unwrap();
    assert!(path.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    assert!(path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("maobu-public-bt-"));
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("ubuntu-"));
    assert!(std::fs::metadata(&path).unwrap().len() <= 1024 * 1024);
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&bytes)),
        std::env::var("MAOBU_PUBLIC_TORRENT_SHA256").unwrap()
    );
    let result = super::inspect_torrent_bytes(&bytes).unwrap();
    eprintln!(
        "UBUNTU_PRODUCTION_METADATA {}",
        serde_json::to_string(&result).unwrap()
    );
    assert!(!result.files.is_empty());
}
