use super::*;

const USER_SHARE: &str = "6.94 NJI:/ m@Q.xS :5pm 10/20 2026年也有闪击波兰？波兰总理被指发动“政变”“波兰政变”？？“波兰蒸便”！！抽象到不能再抽象# 全球深度看抖音 # 零基础看懂全球 # 全球创作者计划 # 青年创作者成长计划 # 来抖音精选App看天下事  https://v.douyin.com/ZUl9gxD5v6s/ 复制此链接，打开Dou音搜索，直接观看视频！";

#[test]
fn complete_douyin_share_preserves_original_short_link() {
    assert_eq!(
        extract_url_from_share_text(USER_SHARE).as_deref(),
        Some("https://v.douyin.com/ZUl9gxD5v6s/")
    );
}

#[tokio::test]
#[ignore = "用户提供的公网作品验收；默认测试不得依赖第三方站点"]
async fn anonymous_user_share_resolves_and_probes_original_work() {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap();
    let short = extract_url_from_share_text(USER_SHARE).unwrap();
    let expanded = expand_short_url_with_client(&short, &client)
        .await
        .expect("用户分享短链应能展开为原作品地址");
    let id = extract_douyin_aweme_id(&expanded).expect("应保留具体作品ID");
    assert_eq!(id, "7702744149895712063");
    let detail = fetch_douyin_aweme_detail_with_client(&id, None, None, None, &client)
        .await
        .expect("匿名原作品元数据应可获取");
    assert_eq!(detail["aweme_detail"]["aweme_id"], id);
    let converted = convert_douyin_aweme_to_yt_dlp_json(&detail);
    let formats = converted["formats"].as_array().unwrap();
    assert!(!formats.is_empty(), "原作品必须包含实际下载候选");
    eprintln!(
        "DOUYIN_USER_SAMPLE {}",
        serde_json::json!({
            "id":id,"title":converted["title"],"duration":converted["duration"],
            "formats":formats.len(),"anonymous":true,"used_personal_cookie":false
        })
    );
}
