use super::QuarkFileItem;
use crate::cloud_pagination::{self as paging, Context, Entry, Page, PageKey, Source};
use tokio_util::sync::CancellationToken;
pub(super) struct QuarkSource<'a> {
    pub context: Context,
    pub pwd_id: &'a str,
    pub stoken: &'a str,
    pub cookie: Option<&'a str>,
}
impl Source for QuarkSource<'_> {
    fn context(&self) -> &Context {
        &self.context
    }
    async fn page(&mut self, folder: &str, key: PageKey) -> Result<Page, String> {
        let mut request = self
            .context
            .get(format!(
                "{}/1/clouddrive/share/sharepage/detail",
                self.context.base
            ))
            .query(&[
                ("pwd_id", self.pwd_id.to_owned()),
                ("stoken", self.stoken.to_owned()),
                ("pdir_fid", folder.to_owned()),
                ("_page", key.number.to_string()),
                ("_size", "100".into()),
            ])
            .header("Referer", "https://pan.quark.cn/");
        if let Some(cookie) = self.cookie {
            request = request.header("Cookie", cookie);
        }
        let value = self.context.json(request).await?;
        let status = value
            .get("code")
            .and_then(paging::signed)
            .ok_or_else(|| self.context.incomplete("目录响应状态字段无效"))?;
        if status != 0 {
            return Err(self
                .context
                .incomplete("夸克接口拒绝目录读取，请检查分享权限和登录状态"));
        }
        let data = value
            .get("data")
            .ok_or_else(|| self.context.incomplete("目录响应缺少 data"))?;
        let list = data
            .get("list")
            .and_then(|v| v.as_array())
            .ok_or_else(|| self.context.incomplete("目录响应缺少 list"))?;
        let total = paging::optional_number(
            &self.context,
            data.get("total")
                .or_else(|| value.get("metadata")?.get("_total")),
        )?;
        let more = paging::optional_flag(&self.context, data.get("has_more"))?;
        let next = paging::numbered_next(
            &self.context,
            &key,
            paging::unique_count(list, "fid"),
            total,
            more,
            None,
        )?;
        let entries = list
            .iter()
            .map(|raw| Entry {
                id: raw.get("fid").and_then(paging::text).unwrap_or_default(),
                name: raw
                    .get("file_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .into(),
                folder: raw.get("dir").and_then(paging::flag) == Some(true)
                    || raw.get("file").and_then(paging::flag) == Some(false)
                    || raw.get("is_dir").and_then(paging::flag) == Some(true)
                    || raw.get("obj_type").and_then(|v| v.as_str()) == Some("dir")
                    || raw.get("format_type").and_then(|v| v.as_str()) == Some("dir"),
                size: raw.get("size").and_then(paging::number).unwrap_or(0),
                path: String::new(),
                raw: raw.clone(),
            })
            .collect();
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
    pwd_id: &str,
    stoken: &str,
    root: &str,
    cookie: Option<&str>,
    cancel: CancellationToken,
    user_agent: Option<&str>,
    network: crate::cloud_request::CloudRequestContext,
) -> Result<Vec<QuarkFileItem>, String> {
    let mut source = QuarkSource {
        context: Context::new(client, base, cancel, "夸克网盘")
            .with_user_agent(user_agent)
            .with_network(network),
        pwd_id,
        stoken,
        cookie,
    };
    Ok(paging::collect(&mut source, root)
        .await?
        .into_iter()
        .map(|entry| {
            let text = |key| {
                entry
                    .raw
                    .get(key)
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            };
            QuarkFileItem {
                id: entry.id,
                name: entry.name.clone(),
                kind: if entry.folder {
                    "drive#folder"
                } else {
                    "drive#file"
                }
                .into(),
                size: entry.size,
                path: entry.path,
                share_fid_token: text("share_fid_token").or_else(|| text("fid_token")),
                mime_type: text("mime_type"),
                file_extension: text("file_extension").or_else(|| {
                    entry
                        .name
                        .rsplit_once('.')
                        .map(|(_, extension)| extension.to_owned())
                }),
                thumbnail_url: text("thumbnail").or_else(|| text("icon")),
                format_type: text("format_type"),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_pagination::fixture::{Fixture, Scenario, Style};
    #[tokio::test]
    async fn quark_real_pagination_requests_are_complete_or_explicitly_fail() {
        for case in Scenario::ALL {
            let fixture = Fixture::start(Style::Quark, case).await;
            let mut source = QuarkSource {
                context: fixture.context(),
                pwd_id: "synthetic-share",
                stoken: "synthetic-sign",
                cookie: Some("synthetic-cookie"),
            };
            let result = paging::collect(&mut source, Style::Quark.root()).await;
            fixture.verify(result);
        }
    }
}
