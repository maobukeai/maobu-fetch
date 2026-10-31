use super::BaiduFileItem;
use crate::cloud_pagination::{self as paging, Context, Entry, Page, PageKey, Source};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
pub(super) struct BaiduSource<'a> {
    pub context: Context,
    pub surl: &'a str,
    pub uk: &'a str,
    pub share_id: &'a str,
    pub cookie: &'a str,
    pub referer: &'a str,
    directories: HashMap<String, String>,
}
impl Source for BaiduSource<'_> {
    fn context(&self) -> &Context {
        &self.context
    }
    async fn page(&mut self, folder: &str, key: PageKey) -> Result<Page, String> {
        let directory = self
            .directories
            .get(folder)
            .cloned()
            .ok_or_else(|| self.context.incomplete("目录定位无效"))?;
        let request = self
            .context
            .get(format!("{}/share/list", self.context.base))
            .query(&[
                ("shorturl", self.surl.to_owned()),
                ("uk", self.uk.to_owned()),
                ("shareid", self.share_id.to_owned()),
                ("root", if directory == "/" { "1" } else { "0" }.to_owned()),
                ("order", "other".into()),
                ("desc", "1".into()),
                ("showempty", "0".into()),
                ("web", "1".into()),
                ("page", key.number.to_string()),
                ("num", "100".into()),
                ("dir", directory.clone()),
                ("channel", "chunlei".into()),
                ("app_id", "250528".into()),
            ])
            .header("Cookie", self.cookie)
            .header("Referer", self.referer);
        let value = self.context.json(request).await?;
        let status = value
            .get("errno")
            .and_then(paging::signed)
            .ok_or_else(|| self.context.incomplete("目录响应状态字段无效"))?;
        if status != 0 {
            return Err(self
                .context
                .incomplete("百度接口拒绝目录读取，请检查分享权限和登录状态"));
        }
        let list = value
            .get("list")
            .and_then(|v| v.as_array())
            .ok_or_else(|| self.context.incomplete("目录响应缺少 list"))?;
        let total = paging::optional_number(&self.context, value.get("total"))?;
        let more = paging::optional_flag(
            &self.context,
            value.get("has_more").or_else(|| value.get("hasmore")),
        )?;
        let next = paging::numbered_next(
            &self.context,
            &key,
            paging::unique_count(list, "fs_id"),
            total,
            more,
            None,
        )?;
        let mut entries = Vec::new();
        for raw in list {
            let id = raw.get("fs_id").and_then(paging::text).unwrap_or_default();
            let name = raw
                .get("server_filename")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let folder = raw.get("isdir").and_then(paging::flag).unwrap_or(false);
            let size = raw.get("size").and_then(paging::number).unwrap_or(0);
            if folder {
                let path = raw
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{}/{}", directory.trim_end_matches('/'), name));
                self.directories.insert(id.clone(), path);
            }
            entries.push(Entry {
                id,
                name,
                folder,
                size,
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
    surl: &str,
    uk: &str,
    share_id: &str,
    cookie: &str,
    referer: &str,
    cancel: CancellationToken,
    user_agent: Option<&str>,
    network: crate::cloud_request::CloudRequestContext,
) -> Result<Vec<BaiduFileItem>, String> {
    let context = Context::new(client, base, cancel, "百度网盘")
        .with_user_agent(user_agent)
        .with_network(network);
    let mut source = BaiduSource {
        context,
        surl,
        uk,
        share_id,
        cookie,
        referer,
        directories: HashMap::from([("/".into(), "/".into())]),
    };
    Ok(paging::collect(&mut source, "/")
        .await?
        .into_iter()
        .map(|entry| BaiduFileItem {
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
            md5: entry
                .raw
                .get("md5")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            category: entry
                .raw
                .get("category")
                .and_then(|v| v.as_i64())
                .or_else(|| entry.raw.get("category")?.as_str()?.parse().ok()),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_pagination::fixture::{Fixture, Scenario, Style};
    #[tokio::test]
    async fn baidu_real_pagination_requests_are_complete_or_explicitly_fail() {
        for case in Scenario::ALL {
            let fixture = Fixture::start(Style::Baidu, case).await;
            let mut source = BaiduSource {
                context: fixture.context(),
                surl: "synthetic-share",
                uk: "1",
                share_id: "1",
                cookie: "synthetic-cookie",
                referer: &fixture.base,
                directories: HashMap::from([("/".into(), "/".into())]),
            };
            let result = paging::collect(&mut source, Style::Baidu.root()).await;
            fixture.verify(result);
        }
    }
}
