use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pan123FileItem {
    pub id: i64,
    pub name: String,
    pub size: u64,
    pub size_formatted: String,
    pub etag: String,
    pub s3_key_flag: String,
    pub kind: String, // "file" | "folder"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pan123ShareInfo {
    pub share_key: String,
    pub share_url: String,
    pub title: String,
    pub is_folder: bool,
    pub requires_password: bool,
    pub files: Vec<Pan123FileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pan123DirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}
