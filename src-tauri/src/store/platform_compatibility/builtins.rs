use rusqlite::{params, Connection};

// 这些是静态能力入口说明，不是经过本次网络测试的证据。
const BUILTINS: &[(&str, &str, &[&str])] = &[
    ("baidupan", "提供百度网盘分享解析与下载入口，受限分享需要用户提供凭证。", &["百度网盘公开与加密分享解析，自动转存提取直链与多线程下载，需提供 Cookie"]),
    ("bilibili", "提供哔哩哔哩视频、选集和直播解析入口；各内容类型需分别验证。", &["哔哩哔哩普通视频、多P合集、番剧剧集与实时直播流均可正常下载", "哔哩哔哩普通视频、番剧、直播回放可正常下载"]),
    ("douyin", "提供抖音短链、视频、图集和直播解析入口；登录状态和风控可能影响结果。", &["抖音短链、图集、单视频及实时直播流全功能正常支持", "抖音短链、图集、单视频全功能正常支持"]),
    ("lanzou", "提供蓝奏云分享解析和文件下载入口；文件及目录分享需分别验证。", &["蓝奏云单文件与目录树分享解析，免登录提取 32 线程高速直链"]),
    ("pan123", "提供123云盘分享解析与多文件下载入口；受限分享需要用户提供凭证。", &["123云盘公开与受限分享解析，目录树多选，公开免登录/受限需提供凭证，支持 32 线程并发下载"]),
    ("pikpak", "提供PikPak分享解析和文件下载入口；公开与加密分享需分别验证。", &["PikPak 公开与加密分享、海量目录树解析，支持 16/32 线程极速并发下载"]),
    ("quark", "提供夸克网盘分享解析和文件下载入口；登录、权限和分享状态可能影响结果。", &["夸克网盘公开与加密分享、目录树解析，支持 16/32 线程并发下载"]),
    ("tiktok", "提供TikTok视频和图集解析入口；地区、登录状态和风控可能影响结果。", &["TikTok 普通视频、图集可下载，部分内容受地区限制"]),
    ("twitter", "提供Twitter/X媒体与Spaces解析入口；不同媒体类型和登录权限需分别验证。", &["Twitter/X 普通推文、图集及 Spaces 音频可下载，敏感内容需提供 Cookie", "Twitter/X 普通推文视频可下载，需提供 Cookie"]),
    ("weibo", "提供微博媒体解析入口；登录权限和内容类型需分别验证。", &["微博普通视频、图集可下载，需提供 Cookie"]),
    ("youtube", "提供YouTube媒体解析入口；登录、地区与反爬策略可能影响结果。", &["YouTube 普通视频、直播回放、短视频受反爬机制影响", "YouTube 普通视频、直播回放、短视频可正常下载"]),
    ("xiaohongshu", "提供小红书短链、图文笔记与视频解析入口；图集支持批量下载，风控可能影响未登录访问。", &[]),
    ("kuaishou", "提供快手短链与视频解析入口；登录状态和风控可能影响未登录访问。", &[]),
    ("xigua", "提供西瓜视频短链与视频解析入口；部分高清画质需登录凭证。", &[]),
];

pub(super) fn seed(connection: &Connection) -> Result<(), String> {
    for (platform, notes, previous_notes) in BUILTINS {
        for previous in *previous_notes {
            connection.execute(
                "UPDATE platform_compatibility SET level='experimental', notes=?2 WHERE platform=?1 AND notes=?3 AND level IN ('verified','experimental') AND TRIM(last_tested_at)='' AND known_issues_json='[]' AND (verification_json IS NULL OR verification_json='')",
                params![platform, notes, previous]
            ).map_err(|e| format!("迁移旧兼容性说明失败：{e}"))?;
        }
        connection.execute(
            "INSERT OR IGNORE INTO platform_compatibility(platform,level,notes,known_issues_json,last_tested_at,verification_json) VALUES(?1,'experimental',?2,'[]','',NULL)",
            params![platform, notes]
        ).map_err(|e| format!("初始化兼容性说明失败：{e}"))?;
    }
    Ok(())
}
