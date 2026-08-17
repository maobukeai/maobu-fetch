// "下载被放行（未接管）原因" 的统一中文文案表。
//
// popup 的诊断区与 options 的规则测试器共用同一份文案，
// 避免两处各自维护后口径漂移。新增 evaluateDownload 的 reason 时必须同步这里。

export const IGNORE_REASONS = {
  disabled: "「接管浏览器下载」开关未开启",
  "desktop-disabled": "桌面端已关闭浏览器接管（猫步下载器 → 设置 → 浏览器）",
  bypass: "处于临时暂停接管时段",
  self: "扩展本身发起的下载（防循环限制）",
  "other-extension": "下载由其它浏览器扩展发起（避免与其它下载管理器冲突）",
  scheme: "链接非 HTTP/HTTPS 协议（如 blob/data/file 协议）",
  "blocked-host": "文件所在站点位于禁止接管的主机列表",
  "not-allowed-host": "文件所在站点不在允许接管的主机列表内",
  "site-bypass": "此站点已记住选择：始终由浏览器下载",
  extension: "文件后缀名不在接管后缀名规则列表内",
  size: "文件体积小于设置的接管大小",
  "restored-history": "浏览器重启或会话恢复的历史下载（自动忽略）",
  unpaired: "尚未与桌面端配对，已由浏览器直接下载",
  offline: "桌面端离线，已回退浏览器下载",
};

/// 把 evaluateDownload 的 reason 转为用户可读文案。
/// `error:` 前缀（发送失败）单独拼接底层原因；`minSizeMb` 用于 size 提示。
export function describeIgnoredReason(reason, minSizeMb) {
  if (typeof reason === "string" && reason.startsWith("error:")) {
    return `桌面桥接连接失败（${reason.slice(6)}），已自动回退到浏览器默认下载`;
  }
  if (reason === "size") {
    return `文件体积小于设置的接管大小（当前设为了 ${minSizeMb ?? 1} MB）`;
  }
  return IGNORE_REASONS[reason] || reason || "未知原因";
}

// 分组统计用的短标签（popup 诊断区一行内并列展示多个原因，完整文案过长）。
export const SHORT_IGNORE_REASONS = {
  disabled: "接管关闭",
  "desktop-disabled": "桌面端关闭接管",
  bypass: "临时绕过",
  self: "本扩展发起",
  "other-extension": "其它扩展发起",
  scheme: "非 HTTP 链接",
  "blocked-host": "黑名单站点",
  "not-allowed-host": "不在白名单",
  "site-bypass": "站点记忆放行",
  extension: "类型不符",
  size: "体积过小",
  "restored-history": "历史恢复",
  unpaired: "未配对",
  offline: "桌面端离线",
};

/// 分组统计的短标签：`error:` 前缀统一归为"发送失败"；未知 reason 原样返回。
export function shortReasonLabel(reason) {
  if (typeof reason === "string" && reason.startsWith("error:")) return "发送失败";
  return SHORT_IGNORE_REASONS[reason] || reason || "未知原因";
}
