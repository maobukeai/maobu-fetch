use super::{parse_size_to_bytes, LanzouFileItem};
use crate::{cloud_pagination::Limits, cloud_request::CloudRequestContext};
use serde_json::Value;
use std::collections::HashMap;
pub(super) struct FolderListing {
    pub files: Vec<LanzouFileItem>,
    pub requires_password: bool,
}
struct FolderRequest<'a> {
    api_url: &'a str,
    params: &'a HashMap<String, String>,
    referer: &'a str,
    cookie: Option<&'a str>,
    host: &'a str,
    requires_password: bool,
    has_password: bool,
}
fn incomplete(reason: &str) -> String {
    format!("蓝奏云文件列表未完整：{reason}；请重试或选择较小目录，未返回部分结果")
}
pub(super) async fn collect_folder(
    network: &CloudRequestContext,
    api_url: &str,
    base_params: &HashMap<String, String>,
    referer: &str,
    cookie: Option<&str>,
    host: &str,
    requires_password: bool,
    has_password: bool,
) -> Result<FolderListing, String> {
    let request = FolderRequest {
        api_url,
        params: base_params,
        referer,
        cookie,
        host,
        requires_password,
        has_password,
    };
    network
        .run("蓝奏云", collect(network, &request, Limits::default()))
        .await
}
async fn page(
    network: &CloudRequestContext,
    source: &FolderRequest<'_>,
    page: usize,
) -> Result<Value, String> {
    let mut params = source.params.clone();
    params.insert("pg".into(), page.to_string());
    let mut request = network
        .post(source.api_url)
        .timeout(std::time::Duration::from_secs(20))
        .form(&params)
        .header("Referer", source.referer)
        .header("Accept", "application/json, text/javascript, */*");
    if let Some(cookie) = source.cookie {
        request = request.header("Cookie", cookie);
    }
    let response = request
        .send()
        .await
        .map_err(|error| incomplete(&format!("目录请求失败（{}）", error)))?;
    let text = network
        .text(response)
        .await
        .map_err(|error| incomplete(&error))?;
    serde_json::from_str(&text).map_err(|_| incomplete("服务器返回无效 JSON"))
}
async fn collect(
    network: &CloudRequestContext,
    source: &FolderRequest<'_>,
    limits: Limits,
) -> Result<FolderListing, String> {
    let mut files = Vec::new();
    let mut unique: HashMap<String, (String, u64)> = HashMap::new();
    let mut bytes = 0usize;
    let mut total_size = 0u64;
    for number in 1..=limits.pages {
        if network.cancel().is_cancelled() {
            return Err(incomplete("操作已取消"));
        }
        let value = page(network, source, number).await?;
        let status = value
            .get("zt")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| incomplete("目录状态字段缺失"))?;
        if status == 2 && terminal(&value) {
            return Ok(FolderListing {
                files,
                requires_password: false,
            });
        }
        if status != 1 {
            if number == 1
                && source.requires_password
                && !source.has_password
                && matches!(status, 3 | 4)
            {
                return Ok(FolderListing {
                    files: Vec::new(),
                    requires_password: true,
                });
            }
            let reason = if source.requires_password {
                "提取码错误或目录访问被拒绝"
            } else {
                "目录读取被拒绝，请检查分享状态"
            };
            return Err(incomplete(&format!(
                "{reason}（第{number}页，状态{status}）"
            )));
        }
        let list = value
            .get("text")
            .and_then(|v| v.as_array())
            .ok_or_else(|| incomplete("目录响应缺少文件数组"))?;
        if list.is_empty() {
            return Ok(FolderListing {
                files,
                requires_password: false,
            });
        }
        let before = files.len();
        for raw in list {
            let entry = parse_item(raw, source.host)?;
            let identity = (entry.name.clone(), entry.size);
            if let Some(old) = unique.get(&entry.id) {
                if old != &identity {
                    return Err(incomplete("同一文件的元信息在分页期间变化"));
                }
                continue;
            }
            unique.insert(entry.id.clone(), identity);
            if unique.len() > limits.entries {
                return Err(incomplete("目录条目超过安全上限"));
            }
            bytes = bytes
                .checked_add(raw.to_string().len())
                .filter(|bytes| *bytes <= limits.metadata_bytes)
                .ok_or_else(|| incomplete("目录元数据超过安全上限"))?;
            total_size = total_size
                .checked_add(entry.size)
                .ok_or_else(|| incomplete("文件总大小溢出"))?;
            files.push(entry);
        }
        if files.len() == before {
            return Err(incomplete("分页没有进展，服务器重复返回同一页"));
        }
    }
    Err(incomplete("目录分页达到保护上限"))
}
fn parse_item(item: &Value, host: &str) -> Result<LanzouFileItem, String> {
    let id = item
        .get("id")
        .and_then(crate::cloud_pagination::text)
        .filter(|id| !id.is_empty() && id != "-1")
        .ok_or_else(|| incomplete("文件标识无效"))?;
    let name = item
        .get("name_all")
        .and_then(|v| v.as_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| incomplete("文件名称缺失"))?
        .replace("<span class=\"s_ad\">推广</span>", "");
    let size_text = item
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_owned();
    let size = parse_size_to_bytes(&size_text);
    let url = if id.starts_with("http://") || id.starts_with("https://") {
        id.clone()
    } else {
        format!("https://{host}/{id}")
    };
    Ok(LanzouFileItem {
        id,
        name,
        size,
        size_formatted: size_text,
        time: item
            .get("time")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .into(),
        kind: "file".into(),
        url,
    })
}
fn terminal(value: &Value) -> bool {
    if value
        .get("text")
        .and_then(|v| v.as_array())
        .is_some_and(Vec::is_empty)
    {
        return true;
    }
    value
        .get("text")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("info").and_then(|v| v.as_str()))
        .is_some_and(|text| {
            matches!(
                text.trim(),
                "" | "没有了"
                    | "没有更多文件"
                    | "没有文件"
                    | "已显示全部文件"
                    | "no more files"
                    | "no file"
            )
        })
}
#[cfg(test)]
#[path = "pagination_tests.rs"]
mod tests;
