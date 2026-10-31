use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PikPakFileItem {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_content_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PikPakShareInfo {
    pub share_id: String,
    pub title: String,
    pub files: Vec<PikPakFileItem>,
    pub total_size: u64,
    pub file_count: usize,
    pub folder_count: usize,
    pub pass_code_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass_code_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PikPakDirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}
