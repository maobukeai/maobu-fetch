use super::{PikPakFileItem, PIKPAK_CLIENT_ID, PIKPAK_CLIENT_VERSION};
use crate::cloud_pagination::{self as paging, Context, Entry, Page, PageKey, Source};
use tokio_util::sync::CancellationToken;
pub(super) struct PikPakSource<'a> {
    pub context: Context,
    pub share_id: &'a str,
    pub password: Option<&'a str>,
    pub pass_token: String,
    pub captcha: &'a str,
    pub sign: &'a str,
    pub device: &'a str,
}
impl Source for PikPakSource<'_> {
    fn context(&self) -> &Context {
        &self.context
    }
    async fn page(&mut self, folder: &str, key: PageKey) -> Result<Page, String> {
        let endpoint = if folder.is_empty() {
            "/drive/v1/share"
        } else {
            "/drive/v1/share/detail"
        };
        let mut query = vec![
            ("share_id", self.share_id.to_owned()),
            ("limit", "100".into()),
        ];
        if !folder.is_empty() {
            query.push(("parent_id", folder.into()));
        }
        if let Some(token) = &key.token {
            query.push(("page_token", token.clone()));
        }
        if !self.pass_token.is_empty() {
            query.push(("pass_code_token", self.pass_token.clone()));
        } else if let Some(password) = self.password {
            query.push(("pass_code", password.into()));
        }
        let mut request = self
            .context
            .get(format!("{}{}", self.context.base, endpoint))
            .query(&query)
            .header("X-Client-Id", PIKPAK_CLIENT_ID)
            .header("X-Client-Version", PIKPAK_CLIENT_VERSION)
            .header("X-Device-Id", self.device)
            .header("X-Device-Sign", self.sign)
            .header("Referer", "https://mypikpak.com/");
        if !self.captcha.is_empty() {
            request = request.header("X-Captcha-Token", self.captcha);
        }
        let value = self.context.json(request).await.map_err(|error| {
            if error.contains("HTTP 403") {
                "NEED_PASS_CODE".into()
            } else {
                error
            }
        })?;
        if value
            .get("error")
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(self.context.incomplete("接口错误字段无效"));
        }
        let status = value
            .get("share_status")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let error = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if status == "PASS_CODE_EMPTY" || error == "need_pass_code" {
            return Err("NEED_PASS_CODE".into());
        }
        if status == "PASS_CODE_ERROR" || error == "invalid_pass_code" {
            return Err("提取码错误，请重新输入".into());
        }
        if !error.is_empty()
            || value
                .get("error_description")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
        {
            return Err(self
                .context
                .incomplete("PikPak 接口拒绝目录读取，请检查分享状态和访问权限"));
        }
        if self.pass_token.is_empty() {
            if let Some(token) = value.get("pass_code_token").and_then(|v| v.as_str()) {
                self.pass_token = token.into();
            }
        }
        let list = value
            .get("files")
            .and_then(|v| v.as_array())
            .ok_or_else(|| self.context.incomplete("目录响应缺少 files"))?;
        if value
            .get("next_page_token")
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(self.context.incomplete("目录分页游标无效"));
        }
        let token = value
            .get("next_page_token")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty());
        let next = token.map(|token| PageKey {
            number: key.number.saturating_add(1),
            token: Some(token.into()),
        });
        let total = paging::optional_number(&self.context, value.get("total"))?;
        let mut entries = Vec::new();
        for raw in list {
            let kind = raw
                .get("kind")
                .and_then(|v| v.as_str())
                .ok_or_else(|| self.context.incomplete("文件条目缺少 kind"))?;
            if !matches!(kind, "drive#folder" | "drive#file") {
                return Err(self.context.incomplete("文件条目类型无效"));
            }
            entries.push(Entry {
                id: raw.get("id").and_then(paging::text).unwrap_or_default(),
                name: raw
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .into(),
                folder: kind == "drive#folder",
                size: raw.get("size").and_then(paging::number).unwrap_or(0),
                path: String::new(),
                raw: raw.clone(),
            });
        }
        Ok(Page {
            entries,
            next,
            total,
        })
    }
}
pub(super) async fn fetch(
    client: reqwest::Client,
    base: &str,
    share_id: &str,
    root: &str,
    password: Option<&str>,
    pass_token: &mut String,
    captcha: &str,
    sign: &str,
    device: &str,
    cancel: CancellationToken,
    user_agent: Option<&str>,
    network: crate::cloud_request::CloudRequestContext,
) -> Result<Vec<PikPakFileItem>, String> {
    let mut source = PikPakSource {
        context: Context::new(client, base, cancel, "PikPak")
            .with_user_agent(user_agent)
            .with_network(network),
        share_id,
        password,
        pass_token: pass_token.clone(),
        captcha,
        sign,
        device,
    };
    let entries = paging::collect(&mut source, root).await?;
    *pass_token = source.pass_token;
    Ok(entries
        .into_iter()
        .map(|entry| {
            let text = |key| {
                entry
                    .raw
                    .get(key)
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            };
            PikPakFileItem {
                id: entry.id,
                name: entry.name,
                kind: if entry.folder {
                    "drive#folder"
                } else {
                    "drive#file"
                }
                .into(),
                size: entry.size,
                path: entry.path,
                mime_type: text("mime_type"),
                file_extension: text("file_extension"),
                thumbnail_url: text("thumbnail_link").or_else(|| text("icon_link")),
                web_content_link: text("web_content_link"),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_pagination::fixture::{Fixture, Scenario, Style};
    #[tokio::test]
    async fn pikpak_real_pagination_requests_are_complete_or_explicitly_fail() {
        for case in Scenario::ALL {
            let fixture = Fixture::start(Style::PikPak, case).await;
            let mut source = PikPakSource {
                context: fixture.context(),
                share_id: "synthetic-share",
                password: Some("synthetic-password"),
                pass_token: String::new(),
                captcha: "synthetic-sign",
                sign: "synthetic-sign",
                device: "synthetic-device",
            };
            let result = paging::collect(&mut source, Style::PikPak.root()).await;
            fixture.verify(result);
        }
    }
}
