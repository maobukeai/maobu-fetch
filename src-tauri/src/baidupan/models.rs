use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaiduFileItem {
    pub id: String, // fs_id 字符串
    pub name: String,
    pub kind: String, // "drive#file" or "drive#folder"
    pub size: u64,
    pub path: String,
    pub md5: Option<String>,
    pub category: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaiduShareInfo {
    pub surl: String,
    pub share_id: Option<String>,
    pub uk: Option<String>,
    pub title: String,
    pub files: Vec<BaiduFileItem>,
    pub total_size: u64,
    pub file_count: usize,
    pub folder_count: usize,
    pub pass_code_required: bool,
    pub randsk: Option<String>,
    pub sign: Option<String>,
    pub timestamp: Option<i64>,
    pub seckey: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaiduDirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}
