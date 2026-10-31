use crate::models::{AppSettings, DownloadTask};

/// Proxy choices are shared by native metadata requests and the external-process tunnel.
/// Credentials stay in memory and must never be formatted into diagnostics.
#[derive(Clone)]
pub(super) struct NetworkPolicy {
    proxy: Option<url::Url>,
    bypass_private: bool,
}

impl NetworkPolicy {
    pub fn resolve(settings: &AppSettings, task: Option<&DownloadTask>) -> Result<Self, String> {
        let explicit = task.and_then(|task| task.proxy_override.as_deref());
        let address = match explicit {
            Some("") => None,
            Some(address) => Some(crate::proxy::normalize_proxy_scheme(address)),
            None if settings.proxy_mode == "manual" && !settings.proxy_url.trim().is_empty() => {
                Some(crate::proxy::normalize_proxy_scheme(&settings.proxy_url))
            }
            None if settings.proxy_mode == "system" => crate::proxy::get_effective_system_proxy(),
            _ => None,
        };
        let mut proxy = address
            .map(|address| {
                crate::proxy::validate_proxy_url(&address)?;
                url::Url::parse(&address).map_err(|_| "媒体代理地址无效".to_owned())
            })
            .transpose()?;
        if let Some(proxy) = proxy.as_mut() {
            let auth = if explicit.is_some_and(|address| !address.is_empty()) {
                match task.and_then(|task| task.proxy_auth.as_ref()) {
                    Some(auth) => Some(
                        crate::proxy::decode_proxy_auth(auth)
                            .ok_or("媒体任务代理认证无法解密，请重新设置代理密码")?,
                    ),
                    None => None,
                }
            } else if settings.proxy_mode == "manual" && !settings.proxy_username.is_empty() {
                Some(crate::models::ProxyAuth {
                    username: settings.proxy_username.clone(),
                    password: settings.proxy_password.clone(),
                })
            } else {
                None
            };
            if let Some(auth) = auth {
                proxy
                    .set_username(&auth.username)
                    .map_err(|_| "媒体代理用户名无效")?;
                proxy
                    .set_password(Some(&auth.password))
                    .map_err(|_| "媒体代理密码无效")?;
            }
        }
        Ok(Self {
            proxy,
            bypass_private: explicit.is_none(),
        })
    }

    pub fn upstream(&self, host: &str) -> Option<&url::Url> {
        if self.bypass_private && crate::proxy::is_private_or_tailscale_host(host) {
            None
        } else {
            self.proxy.as_ref()
        }
    }

    pub fn client(
        &self,
        target_url: &str,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::Client, String> {
        let host = url::Url::parse(target_url)
            .map_err(|_| "媒体地址无效")?
            .host_str()
            .ok_or("媒体地址缺少主机")?
            .to_owned();
        let mut builder = builder.no_proxy();
        if let Some(proxy) = self.upstream(&host) {
            builder =
                builder.proxy(reqwest::Proxy::all(proxy.as_str()).map_err(|_| "媒体代理配置无效")?);
        }
        builder
            .build()
            .map_err(|_| "创建媒体网络连接失败，请检查代理与系统网络设置".into())
    }
}
