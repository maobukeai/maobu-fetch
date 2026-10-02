//! 猫步下载器 · 局域网互联2 (LanDisk) 客户端任务树解构服务。
//!
//! 核心设计：
//! 1. 针对《局域网互联2》无法断点续传的 `POST /api/download/batch` 动态 Deflate 压缩流，
//!    在客户端主动调用 `/api/files?path=...` 递归探测远程文件树；
//! 2. 将批量文件与多级目录解构为独立的单文件 HTTP Range 206 下载任务；
//! 3. 保留完整的相对目录结构在本地落盘，每个单文件享受 16 线程 Range 并发加速与随时断点续传；
//! 4. 支持可选在本地毫秒级将已下载的目录结构封包为标准 ZIP（0 远程服务端开销）。

use reqwest::header::ACCEPT_ENCODING;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use url::Url;
use zip::write::{SimpleFileOptions, ZipWriter};

/// 局域网互联2 文件项结构（由 `GET /api/files?path=...` 返回）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanDiskFileItem {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(rename = "isDirectory", default)]
    pub is_directory: bool,
    #[serde(default)]
    pub mtime: Option<serde_json::Value>,
}

/// 解构后的单个下载文件描述
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LanDiskDeconstructedFile {
    pub name: String,
    pub remote_path: String,
    /// 相对目录（例如 "" 或 "Photos/2026"）
    pub relative_dir: String,
    pub size: u64,
    pub download_url: String,
}

/// 局域网互联2 文件树解构结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanDiskInspectionResult {
    pub root_name: String,
    pub total_size: u64,
    pub file_count: usize,
    pub folder_count: usize,
    pub files: Vec<LanDiskDeconstructedFile>,
}

/// URL 解析参数提取结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanDiskUrlParams {
    pub base_url: String,
    pub target_path: Option<String>,
    pub folder_name: Option<String>,
    pub pin: Option<String>,
    pub files: Vec<String>,
    pub is_batch: bool,
}

/// 提取文件名中的有效名字（处理 Windows/Unix 路径分隔符）
pub fn extract_path_leaf_name(path_str: &str) -> String {
    let clean = path_str.trim().trim_end_matches(['/', '\\']);
    if let Some(pos) = clean.rfind(['/', '\\']) {
        let leaf = &clean[pos + 1..];
        if !leaf.is_empty() {
            return leaf.to_string();
        }
    }
    if !clean.is_empty() {
        clean.to_string()
    } else {
        "folder_download".to_string()
    }
}

/// 判断 URL 是否为局域网互联 (LanDisk) 相关链接
pub fn is_landisk_url(url_str: &str) -> bool {
    let parsed = match Url::parse(url_str.trim()) {
        Ok(u) => u,
        Err(_) => return false,
    };

    let path = parsed.path().to_ascii_lowercase();
    let query = parsed.query().unwrap_or("").to_ascii_lowercase();
    let fragment = parsed.fragment().unwrap_or("").to_ascii_lowercase();

    // 1. 明确的 API 路径
    if path.contains("/api/download/batch")
        || path.contains("/api/download")
        || path.contains("/api/files")
        || path.contains("/api/drives")
    {
        return true;
    }

    // 2. 网页前端 SPA 带有 ?path= 或 #/files 且运行在局域网 / Tailscale 主机
    let is_private_host = crate::proxy::is_private_or_tailscale_url(url_str)
        || parsed
            .host_str()
            .map(|h| {
                h.eq_ignore_ascii_case("localhost")
                    || h == "127.0.0.1"
                    || h.ends_with(".local")
                    || h.ends_with(".lan")
                    || h.ends_with(".ts.net")
                    || h.ends_with(".tailscale.net")
            })
            .unwrap_or(false);

    if is_private_host {
        if query.contains("path=") || query.contains("files=") || fragment.contains("path=") {
            return true;
        }
        if path.is_empty() || path == "/" {
            // 带有 pin 或 token 鉴权参数的根路径
            if query.contains("pin=") || query.contains("token=") {
                return true;
            }
        }
    }

    false
}

/// 解析局域网互联 URL 并提取查询参数
pub fn parse_landisk_url(url_str: &str) -> Option<LanDiskUrlParams> {
    let parsed = Url::parse(url_str.trim()).ok()?;
    let path = parsed.path();
    let is_batch = path.contains("/api/download/batch");

    let mut target_path: Option<String> = None;
    let mut folder_name: Option<String> = None;
    let mut pin: Option<String> = None;
    let mut files: Vec<String> = Vec::new();

    // 提取 query 参数
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "path" => {
                if target_path.is_none() && !v.is_empty() {
                    target_path = Some(v.into_owned());
                }
            }
            "folderName" | "name" => {
                if folder_name.is_none() && !v.is_empty() {
                    folder_name = Some(v.into_owned());
                }
            }
            "pin" | "token" => {
                if pin.is_none() && !v.is_empty() {
                    pin = Some(v.into_owned());
                }
            }
            "files" | "files[]" => {
                if !v.is_empty() {
                    files.push(v.into_owned());
                }
            }
            _ => {}
        }
    }

    // 若 query 中未提取到 path，尝试从 hash fragment 提取（如 #/files?path=...）
    if target_path.is_none() {
        if let Some(frag) = parsed.fragment() {
            if let Some(idx) = frag.find('?') {
                let frag_query = &frag[idx + 1..];
                for pair in frag_query.split('&') {
                    let mut parts = pair.splitn(2, '=');
                    if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                        let decoded_v = urlencoding_decode(v);
                        if k == "path" && !decoded_v.is_empty() {
                            target_path = Some(decoded_v);
                        } else if (k == "pin" || k == "token") && pin.is_none() && !decoded_v.is_empty() {
                            pin = Some(decoded_v);
                        }
                    }
                }
            }
        }
    }

    if target_path.is_none() && files.is_empty() {
        return None;
    }

    let mut base_url = parsed.clone();
    base_url.set_path("");
    base_url.set_query(None);
    base_url.set_fragment(None);

    Some(LanDiskUrlParams {
        base_url: base_url.to_string().trim_end_matches('/').to_string(),
        target_path,
        folder_name,
        pin,
        files,
        is_batch,
    })
}

fn urlencoding_decode(s: &str) -> String {
    percent_encoding_decode(s)
}

fn percent_encoding_decode(s: &str) -> String {
    let mut bytes = Vec::new();
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                if let Ok(hex_byte) = u8::from_str_radix(&format!("{}{}", c1 as char, c2 as char), 16) {
                    bytes.push(hex_byte);
                    continue;
                }
            }
        } else if b == b'+' {
            bytes.push(b' ');
            continue;
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).to_string()
}

/// 构造单个文件的下载直链
pub fn build_landisk_download_url(base_url_str: &str, remote_path: &str, pin: Option<&str>) -> String {
    let base = base_url_str.trim().trim_end_matches('/');
    let mut url = match Url::parse(&format!("{base}/api/download")) {
        Ok(u) => u,
        Err(_) => {
            let encoded: String = url::form_urlencoded::byte_serialize(remote_path.as_bytes()).collect();
            return format!("{base}/api/download?path={encoded}");
        }
    };
    url.query_pairs_mut().append_pair("path", remote_path);
    if let Some(p) = pin.filter(|s| !s.is_empty()) {
        url.query_pairs_mut().append_pair("pin", p);
    }
    url.to_string()
}

/// 对单个路径（单目录或单文件）进行客户端文件树递归探测与解构
pub async fn deconstruct_landisk_path(
    client: &reqwest::Client,
    base_url: &Url,
    target_path: &str,
    pin: Option<&str>,
) -> Result<LanDiskInspectionResult, String> {
    let base_str = base_url.as_str().trim_end_matches('/');
    let root_name = extract_path_leaf_name(target_path);

    // 首先探测该路径是否为目录
    let mut files_url = Url::parse(&format!("{base_str}/api/files"))
        .map_err(|e| format!("构造探测 URL 失败: {e}"))?;
    files_url.query_pairs_mut().append_pair("path", target_path);
    if let Some(p) = pin.filter(|s| !s.is_empty()) {
        files_url.query_pairs_mut().append_pair("pin", p);
    }

    let mut req = client.get(files_url).header(ACCEPT_ENCODING, "identity");
    if let Some(p) = pin.filter(|s| !s.is_empty()) {
        req = req.header("x-pin", p);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("无法连接局域网互联服务器: {e}"))?;

    // 如果状态码不是 200，检查是否为单文件（LanDisk 对文件请求 /api/files 会返回 400 "Not a directory"）
    if resp.status() == reqwest::StatusCode::BAD_REQUEST {
        // 单文件场景：直接构建单个下载任务
        let download_url = build_landisk_download_url(base_str, target_path, pin);
        return Ok(LanDiskInspectionResult {
            root_name: root_name.clone(),
            total_size: 0,
            file_count: 1,
            folder_count: 0,
            files: vec![LanDiskDeconstructedFile {
                name: root_name,
                remote_path: target_path.to_string(),
                relative_dir: String::new(),
                size: 0,
                download_url,
            }],
        });
    }

    if !resp.status().is_success() {
        return Err(format!("局域网互联返回错误 HTTP {}", resp.status()));
    }

    let items: Vec<LanDiskFileItem> = resp
        .json()
        .await
        .map_err(|e| format!("解析文件列表 JSON 失败: {e}"))?;

    // BFS 递归探测整棵树
    let mut queue: VecDeque<(String, String)> = VecDeque::new(); // (remote_dir_path, relative_prefix)
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(target_path.to_string());

    let mut deconstructed_files: Vec<LanDiskDeconstructedFile> = Vec::new();
    let mut total_size: u64 = 0;
    let mut folder_count: usize = 0;

    // 处理根目录的第一层
    for item in items {
        if item.is_directory {
            folder_count += 1;
            if visited.insert(item.path.clone()) {
                queue.push_back((item.path, item.name));
            }
        } else {
            total_size += item.size;
            let download_url = build_landisk_download_url(base_str, &item.path, pin);
            deconstructed_files.push(LanDiskDeconstructedFile {
                name: item.name,
                remote_path: item.path,
                relative_dir: String::new(),
                size: item.size,
                download_url,
            });
        }
    }

    // 限制最大探测文件与深度，防止恶意符号链接或超大目录耗尽内存
    const MAX_FILES: usize = 10000;
    const MAX_FOLDERS: usize = 1000;

    while let Some((dir_path, rel_prefix)) = queue.pop_front() {
        if folder_count >= MAX_FOLDERS || deconstructed_files.len() >= MAX_FILES {
            tracing::warn!("局域网互联目录探测已达安全上限，截断后续层级");
            break;
        }

        let mut sub_files_url = Url::parse(&format!("{base_str}/api/files"))
            .map_err(|e| format!("构造子目录探测 URL 失败: {e}"))?;
        sub_files_url.query_pairs_mut().append_pair("path", &dir_path);
        if let Some(p) = pin.filter(|s| !s.is_empty()) {
            sub_files_url.query_pairs_mut().append_pair("pin", p);
        }

        let mut sub_req = client.get(sub_files_url).header(ACCEPT_ENCODING, "identity");
        if let Some(p) = pin.filter(|s| !s.is_empty()) {
            sub_req = sub_req.header("x-pin", p);
        }

        if let Ok(sub_resp) = sub_req.send().await {
            if sub_resp.status().is_success() {
                if let Ok(sub_items) = sub_resp.json::<Vec<LanDiskFileItem>>().await {
                    for item in sub_items {
                        if item.is_directory {
                            folder_count += 1;
                            if visited.insert(item.path.clone()) {
                                let next_rel = if rel_prefix.is_empty() {
                                    item.name
                                } else {
                                    format!("{rel_prefix}/{}", item.name)
                                };
                                queue.push_back((item.path, next_rel));
                            }
                        } else {
                            total_size += item.size;
                            let download_url = build_landisk_download_url(base_str, &item.path, pin);
                            deconstructed_files.push(LanDiskDeconstructedFile {
                                name: item.name,
                                remote_path: item.path,
                                relative_dir: rel_prefix.clone(),
                                size: item.size,
                                download_url,
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(LanDiskInspectionResult {
        root_name,
        total_size,
        file_count: deconstructed_files.len(),
        folder_count,
        files: deconstructed_files,
    })
}

/// 对多选批量路径进行解构（支持文件与目录混合选择）
pub async fn deconstruct_landisk_batch(
    client: &reqwest::Client,
    base_url: &Url,
    batch_name: &str,
    paths: &[String],
    pin: Option<&str>,
) -> Result<LanDiskInspectionResult, String> {
    if paths.is_empty() {
        return Err("所选下载列表为空".into());
    }

    let clean_batch_name = if batch_name.trim().is_empty() || batch_name == "batch_download" {
        if paths.len() == 1 {
            extract_path_leaf_name(&paths[0])
        } else {
            format!("batch_download_{}_items", paths.len())
        }
    } else {
        batch_name.trim().to_string()
    };

    let mut merged_files: Vec<LanDiskDeconstructedFile> = Vec::new();
    let mut total_size: u64 = 0;
    let mut folder_count: usize = 0;

    for target in paths {
        let single_res = deconstruct_landisk_path(client, base_url, target, pin).await?;
        folder_count += single_res.folder_count;

        // 判断 target 是否为目录（有子目录，或包含多个文件，或包含单个文件但其 remote_path 与 target 不同）
        let is_dir = single_res.folder_count > 0
            || single_res.files.len() > 1
            || single_res.files.first().map(|f| &f.remote_path != target).unwrap_or(false);

        for mut file in single_res.files {
            total_size += file.size;
            // 只有当批量选择中包含目录时，才将该目录项归拢到以其目录名称命名的子目录下；单文件直接平铺在根目录
            if paths.len() > 1 && is_dir {
                let leaf = extract_path_leaf_name(target);
                file.relative_dir = if file.relative_dir.is_empty() {
                    leaf
                } else {
                    format!("{leaf}/{}", file.relative_dir)
                };
            }
            merged_files.push(file);
        }
    }

    Ok(LanDiskInspectionResult {
        root_name: clean_batch_name,
        total_size,
        file_count: merged_files.len(),
        folder_count,
        files: merged_files,
    })
}

/// 本地目录毫秒级快速打包为 ZIP 文件（支持全客户端秒级打包，0 服务端开销）
pub fn pack_directory_to_zip(source_dir: &Path, output_zip_path: &Path) -> Result<u64, String> {
    if !source_dir.exists() || !source_dir.is_dir() {
        return Err(format!("源目录不存在或不是目录: {:?}", source_dir));
    }

    // 确保目标父目录存在
    if let Some(parent) = output_zip_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败: {e}"))?;
    }

    // 临时文件原子写入
    let temp_zip_path = output_zip_path.with_extension("maobu_zip_tmp");

    let res = (|| -> Result<u64, String> {
        let file = File::create(&temp_zip_path)
            .map_err(|e| format!("创建 ZIP 临时文件失败: {e}"))?;

        let mut zip_writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // 收集所有文件并写入 zip
        let mut buffer = vec![0u8; 64 * 1024];
        let mut walk_dirs = vec![source_dir.to_path_buf()];

        while let Some(current_dir) = walk_dirs.pop() {
            let entries = std::fs::read_dir(&current_dir)
                .map_err(|e| format!("读取目录失败: {e}"))?;

            for entry in entries {
                let entry = entry.map_err(|e| format!("读取条目失败: {e}"))?;
                let path = entry.path();

                if path.is_dir() {
                    walk_dirs.push(path);
                } else if path.is_file() {
                    // 跳过未完成的临时分片文件、临时 zip 文件与目标输出 zip 自身
                    if path == temp_zip_path || path == output_zip_path {
                        continue;
                    }
                    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                        if ext.eq_ignore_ascii_case("lumaget")
                            || ext.eq_ignore_ascii_case("maobu_zip_tmp")
                        {
                            continue;
                        }
                    }

                    // 计算相对路径并规范化为 ZIP 标准的 '/'
                    let rel = path
                        .strip_prefix(source_dir)
                        .map_err(|e| format!("计算相对路径失败: {e}"))?;
                    let rel_str = rel.to_string_lossy().replace('\\', "/");

                    zip_writer
                        .start_file(&rel_str, options)
                        .map_err(|e| format!("向 ZIP 写入文件头失败: {e}"))?;

                    let mut in_file = File::open(&path)
                        .map_err(|e| format!("打开待压缩文件失败: {e}"))?;

                    loop {
                        let n = in_file
                            .read(&mut buffer)
                            .map_err(|e| format!("读取文件内容失败: {e}"))?;
                        if n == 0 {
                            break;
                        }
                        zip_writer
                            .write_all(&buffer[..n])
                            .map_err(|e| format!("写入 ZIP 内容失败: {e}"))?;
                    }
                }
            }
        }

        zip_writer
            .finish()
            .map_err(|e| format!("完成 ZIP 打包失败: {e}"))?;

        // 原子重命名
        if output_zip_path.exists() {
            let _ = std::fs::remove_file(output_zip_path);
        }
        std::fs::rename(&temp_zip_path, output_zip_path)
            .map_err(|e| format!("重命名完成 ZIP 失败: {e}"))?;

        let meta = std::fs::metadata(output_zip_path)
            .map_err(|e| format!("获取 ZIP 大小失败: {e}"))?;

        Ok(meta.len())
    })();

    if res.is_err() && temp_zip_path.exists() {
        let _ = std::fs::remove_file(&temp_zip_path);
    }

    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_path_leaf_name() {
        assert_eq!(extract_path_leaf_name("D:\\Photos\\Vacation"), "Vacation");
        assert_eq!(extract_path_leaf_name("D:/Photos/Vacation/"), "Vacation");
        assert_eq!(extract_path_leaf_name("/home/user/music/song.mp3"), "song.mp3");
        assert_eq!(extract_path_leaf_name("single_file.zip"), "single_file.zip");
        assert_eq!(extract_path_leaf_name(""), "folder_download");
    }

    #[test]
    fn test_is_landisk_url_recognition() {
        assert!(is_landisk_url("http://100.100.1.5:8080/api/download/batch"));
        assert!(is_landisk_url("http://192.168.1.100:3000/api/download?path=D:\\Photos"));
        assert!(is_landisk_url("http://node.tailscale.net:8080/api/files?path=/root"));
        assert!(is_landisk_url("http://100.64.0.1:8080/#/files?path=D:\\Doc"));
        assert!(is_landisk_url("http://localhost:3000/?pin=123456"));

        // 公网非 LanDisk 路由返回 false
        assert!(!is_landisk_url("https://example.com/file.zip"));
        assert!(!is_landisk_url("https://github.com/release/v1.zip"));
    }

    #[test]
    fn test_parse_landisk_url() {
        let url = "http://100.100.1.5:8080/api/download/batch?folderName=MyPhotos&files=D%3A%5CPhotos%5CA.jpg&files=D%3A%5CPhotos%5CB.jpg&pin=8888";
        let params = parse_landisk_url(url).expect("应该成功解析");
        assert_eq!(params.base_url, "http://100.100.1.5:8080");
        assert_eq!(params.folder_name.as_deref(), Some("MyPhotos"));
        assert_eq!(params.pin.as_deref(), Some("8888"));
        assert_eq!(params.files.len(), 2);
        assert_eq!(params.files[0], "D:\\Photos\\A.jpg");
        assert_eq!(params.files[1], "D:\\Photos\\B.jpg");
        assert!(params.is_batch);

        // 单文件/单目录 URL 解析
        let single_url = "http://192.168.1.50:3000/api/download?path=D%3A%5CMovies%5CAvatar&pin=1234";
        let single_params = parse_landisk_url(single_url).expect("应该成功解析单路径");
        assert_eq!(single_params.target_path.as_deref(), Some("D:\\Movies\\Avatar"));
        assert_eq!(single_params.pin.as_deref(), Some("1234"));
    }

    #[test]
    fn test_build_landisk_download_url() {
        let dl = build_landisk_download_url("http://100.100.1.5:8080", "D:\\Photos\\IMG_001.JPG", Some("666"));
        assert!(dl.starts_with("http://100.100.1.5:8080/api/download?"));
        assert!(dl.contains("path=D%3A%5CPhotos%5CIMG_001.JPG") || dl.contains("path=D%3A%2FPhotos%2FIMG_001.JPG"));
        assert!(dl.contains("pin=666"));
    }

    #[test]
    fn test_pack_directory_to_zip() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("my_folder");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("a.txt"), b"Hello A").unwrap();

        let sub_dir = src_dir.join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::write(sub_dir.join("b.txt"), b"Hello B in sub").unwrap();

        let zip_out = tmp.path().join("output.zip");
        let zip_size = pack_directory_to_zip(&src_dir, &zip_out).expect("打包应该成功");
        assert!(zip_size > 0);
        assert!(zip_out.exists());

        // 验证 ZipArchive 能解包读取
        let file = File::open(&zip_out).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.len(), 2);

        {
            let mut file_a = archive.by_name("a.txt").unwrap();
            let mut content_a = String::new();
            file_a.read_to_string(&mut content_a).unwrap();
            assert_eq!(content_a, "Hello A");
        }

        {
            let mut file_b = archive.by_name("sub/b.txt").unwrap();
            let mut content_b = String::new();
            file_b.read_to_string(&mut content_b).unwrap();
            assert_eq!(content_b, "Hello B in sub");
        }
    }

    #[test]
    fn test_pack_directory_to_zip_filters_lumaget_and_tmp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("download_dir");
        std::fs::create_dir_all(&src_dir).unwrap();

        // 正常已完成文件
        std::fs::write(src_dir.join("photo.jpg"), b"JPEG DATA").unwrap();
        // 未完成的分片临时文件与中间文件（应被自动忽略）
        std::fs::write(src_dir.join("video.mp4.lumaget"), b"INCOMPLETE PART").unwrap();
        std::fs::write(src_dir.join("old_archive.maobu_zip_tmp"), b"OLD TMP").unwrap();

        let zip_out = tmp.path().join("bundle.zip");
        let zip_size = pack_directory_to_zip(&src_dir, &zip_out).expect("打包应该成功");
        assert!(zip_size > 0);

        let file = File::open(&zip_out).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        // 应该仅包含 1 个有效文件 photo.jpg，排除 .lumaget 和 .maobu_zip_tmp
        assert_eq!(archive.len(), 1);
        assert!(archive.by_name("photo.jpg").is_ok());
        assert!(archive.by_name("video.mp4.lumaget").is_err());
        assert!(archive.by_name("old_archive.maobu_zip_tmp").is_err());
    }

    #[test]
    fn test_pack_directory_to_zip_nonexistent_dir_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let non_dir = tmp.path().join("does_not_exist");
        let zip_out = tmp.path().join("fail.zip");
        let res = pack_directory_to_zip(&non_dir, &zip_out);
        assert!(res.is_err());
        assert!(!zip_out.exists());
    }
}
