//! Shared bounded, cancellable and fail-closed cloud directory enumeration.
#[cfg(test)]
#[path = "cloud_pagination/fixture.rs"]
pub(crate) mod fixture;
use futures_util::StreamExt;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub struct Limits {
    pub entries: usize,
    pub folders: usize,
    pub pages: usize,
    pub depth: usize,
    pub duration: Duration,
    pub metadata_bytes: usize,
}
pub async fn cancellable<T>(
    cancel: &CancellationToken,
    platform: &str,
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {biased;_=cancel.cancelled()=>Err(format!("{platform}文件列表未完整：操作已取消")),result=tokio::time::timeout(Duration::from_secs(120),future)=>result.map_err(|_|format!("{platform}文件列表未完整：分享解析超过总时限，请选择较小目录重试"))?}
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: 20_000,
            folders: 2_000,
            pages: 1_000,
            depth: 128,
            duration: Duration::from_secs(120),
            metadata_bytes: 32 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PageKey {
    pub number: u32,
    pub token: Option<String>,
}
impl Default for PageKey {
    fn default() -> Self {
        Self {
            number: 1,
            token: None,
        }
    }
}
pub struct Entry {
    pub id: String,
    pub name: String,
    pub folder: bool,
    pub size: u64,
    pub path: String,
    pub raw: Value,
}
pub struct Page {
    pub entries: Vec<Entry>,
    pub next: Option<PageKey>,
    pub total: Option<u64>,
}
pub trait Source: Send {
    fn page<'a>(
        &'a mut self,
        folder: &'a str,
        key: PageKey,
    ) -> impl Future<Output = Result<Page, String>> + Send + 'a;
    fn context(&self) -> &Context;
}
pub struct Context {
    pub client: reqwest::Client,
    pub base: String,
    pub cancel: CancellationToken,
    pub limits: Limits,
    started: Instant,
    pub platform: &'static str,
    pub user_agent: Option<String>,
    network: Option<crate::cloud_request::CloudRequestContext>,
}
impl Context {
    pub fn new(
        client: reqwest::Client,
        base: &str,
        cancel: CancellationToken,
        platform: &'static str,
    ) -> Self {
        Self {
            client,
            base: base.trim_end_matches('/').into(),
            cancel,
            limits: Limits::default(),
            started: Instant::now(),
            platform,
            user_agent: None,
            network: None,
        }
    }
    pub fn with_user_agent(mut self, user_agent: Option<&str>) -> Self {
        self.user_agent = user_agent.map(str::to_owned);
        self
    }
    pub fn with_network(mut self, network: crate::cloud_request::CloudRequestContext) -> Self {
        self.network = Some(network);
        self
    }
    pub fn get(&self, url: impl AsRef<str>) -> crate::cloud_request::CloudRequest {
        let network = self.network.clone().unwrap_or_else(|| {
            crate::cloud_request::CloudRequestContext::new(
                self.client.clone(),
                self.client.clone(),
                self.cancel.clone(),
            )
        });
        let mut request = network.get(url);
        if let Some(agent) = &self.user_agent {
            request = request.header(reqwest::header::USER_AGENT, agent);
        }
        request
    }
    pub fn check(&self) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            return Err(self.incomplete("操作已取消"));
        }
        if self.started.elapsed() >= self.limits.duration {
            return Err(self.incomplete("目录读取达到总时间上限，请选择较小目录重试"));
        }
        Ok(())
    }
    pub fn incomplete(&self, reason: &str) -> String {
        format!(
            "{}文件列表未完整：{}；未返回部分结果，请重试或缩小分享目录",
            self.platform, reason
        )
    }
    pub async fn json(&self, request: crate::cloud_request::CloudRequest) -> Result<Value, String> {
        self.check()?;
        let remaining = self
            .limits
            .duration
            .saturating_sub(self.started.elapsed())
            .min(Duration::from_secs(20));
        let work = async {
            let response = request
                .send()
                .await
                .map_err(|error| self.incomplete(&format!("请求失败（{}）", error)))?;
            if !response.status().is_success() {
                return Err(
                    self.incomplete(&format!("服务器返回 HTTP {}", response.status().as_u16()))
                );
            }
            if response
                .content_length()
                .is_some_and(|length| length > 4 * 1024 * 1024)
            {
                return Err(self.incomplete("单页响应超过安全大小"));
            }
            let mut stream = response.bytes_stream();
            let mut body = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| {
                    self.incomplete(&format!("读取响应失败（{}）", error.without_url()))
                })?;
                if body.len() + chunk.len() > 4 * 1024 * 1024 {
                    return Err(self.incomplete("单页响应超过安全大小"));
                }
                body.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&body).map_err(|_| self.incomplete("服务器响应不是有效 JSON"))
        };
        tokio::select! {biased;_=self.cancel.cancelled()=>Err(self.incomplete("操作已取消")),result=tokio::time::timeout(remaining,work)=>result.map_err(|_|self.incomplete("目录请求超时"))?}
    }
}

pub async fn collect<S: Source>(source: &mut S, root: &str) -> Result<Vec<Entry>, String> {
    let mut result = Vec::new();
    let mut queue = VecDeque::from([(root.to_owned(), String::new(), 0usize)]);
    let mut visited = HashSet::new();
    let mut unique: HashMap<String, (String, bool, u64)> = HashMap::new();
    let mut total_size = 0u64;
    let mut metadata_bytes = 0usize;
    while let Some((folder, parent_path, depth)) = queue.pop_front() {
        source.context().check()?;
        if !visited.insert(folder.clone()) {
            continue;
        }
        if visited.len() > source.context().limits.folders || depth > source.context().limits.depth
        {
            return Err(source.context().incomplete("目录数量或层级达到保护上限"));
        }
        let mut key = PageKey::default();
        let mut cursors = HashSet::new();
        let mut folder_ids = HashSet::new();
        let mut expected = None;
        let mut pages = 0;
        loop {
            source.context().check()?;
            pages += 1;
            let cursor = key
                .token
                .as_ref()
                .map(|token| format!("token:{token}"))
                .unwrap_or_else(|| format!("page:{}", key.number));
            if pages > source.context().limits.pages || !cursors.insert(cursor) {
                return Err(source.context().incomplete("分页次数超限或分页游标循环"));
            }
            let page = source.page(&folder, key).await?;
            if let Some(total) = page.total {
                if expected.is_some_and(|saved| saved != total) {
                    return Err(source.context().incomplete("分页期间目录总数发生变化"));
                }
                expected = Some(total);
            }
            let before = folder_ids.len();
            for mut entry in page.entries {
                if entry.id.is_empty() || entry.name.is_empty() {
                    return Err(source.context().incomplete("文件条目缺少必要标识"));
                }
                folder_ids.insert(entry.id.clone());
                let identity = (entry.name.clone(), entry.folder, entry.size);
                if let Some(old) = unique.get(&entry.id) {
                    if old != &identity {
                        return Err(source
                            .context()
                            .incomplete("同一文件的元信息在分页期间发生变化"));
                    }
                    continue;
                }
                unique.insert(entry.id.clone(), identity);
                if !entry.folder {
                    total_size = total_size
                        .checked_add(entry.size)
                        .ok_or_else(|| source.context().incomplete("文件总大小溢出"))?;
                }
                if unique.len() > source.context().limits.entries {
                    return Err(source.context().incomplete("条目数量达到保护上限"));
                }
                entry.path = if parent_path.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{}/{}", parent_path, entry.name)
                };
                let bytes = entry
                    .raw
                    .to_string()
                    .len()
                    .saturating_add(entry.path.len())
                    .saturating_add(entry.id.len())
                    .saturating_add(entry.name.len());
                metadata_bytes = metadata_bytes
                    .checked_add(bytes)
                    .filter(|bytes| *bytes <= source.context().limits.metadata_bytes)
                    .ok_or_else(|| source.context().incomplete("目录元数据达到安全大小上限"))?;
                if entry.folder {
                    queue.push_back((entry.id.clone(), entry.path.clone(), depth + 1));
                }
                result.push(entry);
            }
            if page.next.is_some() && folder_ids.len() == before {
                return Err(source
                    .context()
                    .incomplete("分页无进展，服务器可能重复返回同一页"));
            }
            match page.next {
                Some(next) => key = next,
                None => {
                    if expected.is_some_and(|total| total != folder_ids.len() as u64) {
                        return Err(source
                            .context()
                            .incomplete("分页记录缺失、重复或总数不匹配"));
                    }
                    break;
                }
            }
        }
    }
    Ok(result)
}

pub fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
pub fn signed(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}
pub fn optional_number(context: &Context, value: Option<&Value>) -> Result<Option<u64>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => number(value)
            .map(Some)
            .ok_or_else(|| context.incomplete("目录总数字段无效")),
    }
}
pub fn optional_flag(context: &Context, value: Option<&Value>) -> Result<Option<bool>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => flag(value)
            .map(Some)
            .ok_or_else(|| context.incomplete("目录分页状态字段无效")),
    }
}
pub fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .or_else(|| value.as_u64().map(|number| number.to_string()))
}
pub fn flag(value: &Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.as_i64().map(|number| number != 0))
        .or_else(|| match value.as_str()? {
            "0" | "false" => Some(false),
            "1" | "true" => Some(true),
            _ => None,
        })
}
pub fn unique_count(list: &[Value], key: &str) -> usize {
    list.iter()
        .filter_map(|entry| entry.get(key).and_then(text))
        .collect::<HashSet<_>>()
        .len()
}
pub fn numbered_next(
    context: &Context,
    key: &PageKey,
    count: usize,
    total: Option<u64>,
    more: Option<bool>,
    token: Option<String>,
) -> Result<Option<PageKey>, String> {
    let consumed = u64::from(key.number.saturating_sub(1)) * 100 + count as u64;
    let needed = total.is_some_and(|total| total > consumed);
    if (needed && more == Some(false)) || (count == 0 && (needed || more == Some(true))) {
        return Err(context.incomplete("服务器提前终止目录分页"));
    }
    let continued = more.unwrap_or(needed || (total.is_none() && count == 100));
    if !continued {
        return Ok(None);
    }
    if count < 100 && needed {
        return Err(context.incomplete("分页响应少于声明总数，无法保证文件完整"));
    }
    Ok(Some(PageKey {
        number: key
            .number
            .checked_add(1)
            .ok_or_else(|| context.incomplete("分页页码溢出"))?,
        token,
    }))
}
