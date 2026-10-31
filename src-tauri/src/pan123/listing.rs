use crate::cloud_pagination::{self as paging, Context, Entry, Page, PageKey, Source};
pub(super) struct Pan123Source<'a> {
    pub context: Context,
    pub share_key: &'a str,
    pub password: &'a str,
    pub referer: &'a str,
}
impl Source for Pan123Source<'_> {
    fn context(&self) -> &Context {
        &self.context
    }
    async fn page(&mut self, folder: &str, key: PageKey) -> Result<Page, String> {
        let request = self
            .context
            .get(format!("{}/share/get", self.context.base))
            .query(&[
                ("limit", "100".to_owned()),
                ("next", key.token.clone().unwrap_or_else(|| "0".into())),
                ("page", key.number.to_string()),
                ("orderBy", "share_id".into()),
                ("orderDirection", "desc".into()),
                ("shareKey", self.share_key.into()),
                ("SharePwd", self.password.into()),
                ("parentFileId", folder.into()),
            ])
            .header("Referer", self.referer)
            .header("Platform", "web")
            .header("App-Version", "3");
        let value = self.context.json(request).await?;
        let code = value.get("code").and_then(paging::signed).unwrap_or(-1);
        if matches!(code, 5103 | 50001 | 50002 | 401) {
            return Err("NEED_PASS_CODE".into());
        }
        if code != 0 {
            return Err(self
                .context
                .incomplete("123云盘接口拒绝目录读取，请检查分享权限或提取码"));
        }
        let data = value
            .get("data")
            .ok_or_else(|| self.context.incomplete("目录响应缺少 data"))?;
        let list = data
            .get("InfoList")
            .and_then(|v| v.as_array())
            .ok_or_else(|| self.context.incomplete("目录响应缺少 InfoList"))?;
        let total = paging::optional_number(
            &self.context,
            data.get("Total")
                .or_else(|| data.get("total"))
                .or_else(|| data.get("TotalCount")),
        )?;
        let returned = data.get("Next").or_else(|| data.get("next"));
        if returned.is_some_and(|value| !value.is_null() && paging::text(value).is_none()) {
            return Err(self.context.incomplete("目录分页游标无效"));
        }
        let terminal = returned.is_some_and(|v| v.as_i64() == Some(-1) || v.as_str() == Some("-1"));
        let token = returned
            .and_then(paging::text)
            .filter(|value| value != "-1" && value != "0" && !value.is_empty());
        let next = paging::numbered_next(
            &self.context,
            &key,
            paging::unique_count(list, "FileId"),
            total,
            if terminal {
                Some(false)
            } else if token.is_some() {
                Some(true)
            } else {
                None
            },
            token,
        )?;
        let entries = list
            .iter()
            .map(|raw| Entry {
                id: raw.get("FileId").and_then(paging::text).unwrap_or_default(),
                name: raw
                    .get("FileName")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .into(),
                folder: raw.get("Type").and_then(paging::number) == Some(1),
                size: raw.get("Size").and_then(paging::number).unwrap_or(0),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_pagination::fixture::{Fixture, Scenario, Style};
    #[tokio::test]
    async fn pan123_real_pagination_requests_are_complete_or_explicitly_fail() {
        for case in Scenario::ALL {
            let fixture = Fixture::start(Style::Pan123, case).await;
            let mut source = Pan123Source {
                context: fixture.context(),
                share_key: "synthetic-share",
                password: "synthetic-password",
                referer: &fixture.base,
            };
            let result = paging::collect(&mut source, Style::Pan123.root()).await;
            fixture.verify(result);
        }
    }
}
