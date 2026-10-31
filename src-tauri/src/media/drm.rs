use serde_json::Value;

fn declared_drm(value: &Value) -> bool {
    ["_has_drm", "has_drm"]
        .iter()
        .any(|key| value.get(key).and_then(Value::as_bool) == Some(true))
}

/// 只检查提取器明确声明的状态，覆盖根对象、格式和已选择的分离轨道。
pub(super) fn ensure_unprotected(value: &Value) -> Result<(), String> {
    let protected = declared_drm(value)
        || ["formats", "requested_formats"].iter().any(|key| {
            value
                .get(key)
                .and_then(Value::as_array)
                .is_some_and(|formats| formats.iter().any(declared_drm))
        });
    if protected {
        return Err("MEDIA_DRM_PROTECTED: 检测到 DRM 保护内容，猫步下载器不处理此内容；请使用平台授权的离线功能".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn root_and_format_drm_are_refused_even_when_another_flag_is_false() {
        for metadata in [
            json!({"_has_drm":true}),
            json!({"has_drm":true}),
            json!({"_has_drm":false,"has_drm":true}),
            json!({"has_drm":false,"formats":[{"has_drm":true}]}),
            json!({"formats":[{"_has_drm":false,"has_drm":true}]}),
            json!({"requested_formats":[{"has_drm":true}]}),
        ] {
            assert!(ensure_unprotected(&metadata)
                .unwrap_err()
                .starts_with("MEDIA_DRM_PROTECTED:"));
        }
    }

    #[test]
    fn clear_and_unknown_metadata_do_not_claim_drm_protection() {
        for metadata in [
            json!({}),
            json!({"has_drm":null}),
            json!({"_has_drm":false,"has_drm":false,"formats":[{"has_drm":false},{}]}),
        ] {
            assert!(ensure_unprotected(&metadata).is_ok());
        }
    }
}
