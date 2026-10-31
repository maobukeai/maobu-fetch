use crate::media_platforms::MediaPlatform;
use serde_json::Value;
pub(super) const TWITTER_WEB_BEARER:&str="Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

pub(super) fn classify(platform: MediaPlatform, body: &[u8]) -> Result<(bool, String), String> {
    if platform == MediaPlatform::YouTube {
        let text = std::str::from_utf8(body).map_err(|_| "YouTube凭证响应不是有效文本")?;
        return Ok(
            if text.contains("\"LOGGED_IN\":true") || text.contains("\"LOGGED_IN\": true") {
                (true, "凭证有效：已检测到YouTube登录会话".into())
            } else {
                (
                    false,
                    "YouTube未确认登录会话，请更新Cookie后重试；仅包含SID等字段不代表登录成功"
                        .into(),
                )
            },
        );
    }
    if !matches!(
        platform,
        MediaPlatform::Bilibili
            | MediaPlatform::Douyin
            | MediaPlatform::Twitter
            | MediaPlatform::BaiduPan
    ) {
        return Ok((
            false,
            "服务器已连通；该站点没有已支持的身份校验接口，无法确认登录凭证是否有效".into(),
        ));
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| "站点没有返回有效身份JSON，可能是验证页面或网络限制；未确认登录")?;
    let valid = match platform {
        MediaPlatform::Bilibili => {
            value["code"] == 0
                && value.pointer("/data/isLogin").and_then(Value::as_bool) == Some(true)
        }
        MediaPlatform::Douyin => value
            .pointer("/data/nickname")
            .or_else(|| value.pointer("/data/screen_name"))
            .and_then(Value::as_str)
            .is_some_and(|name| !name.is_empty()),
        MediaPlatform::Twitter => value
            .get("screen_name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.is_empty()),
        MediaPlatform::BaiduPan => {
            value["errno"] == 0
                && value.get("total").and_then(Value::as_u64).is_some()
                && value.get("used").and_then(Value::as_u64).is_some()
        }
        _ => false,
    };
    Ok((
        valid,
        if valid {
            "凭证有效：官方身份接口已确认登录".into()
        } else {
            "站点未确认登录，请在浏览器登录后重新同步Cookie；网络连通或Cookie字段存在不能证明登录有效".into()
        },
    ))
}
