use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarkFileItem {
    pub id: String,
    pub name: String,
    pub kind: String, // "drive#file" or "drive#folder"
    pub size: u64,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_fid_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarkShareInfo {
    pub pwd_id: String,
    pub title: String,
    pub files: Vec<QuarkFileItem>,
    pub total_size: u64,
    pub file_count: usize,
    pub folder_count: usize,
    pub pass_code_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stoken: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarkDirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}
