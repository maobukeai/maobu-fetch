# 实网失败提示的证据边界修复

日期：2026-10-31。来自后续公开样本的实际失败：匿名抖音/西瓜请求被写成“登录已失效”，YouTube访问验证被确定归因为PO Token，还附有Cookie不轮换、12小时恢复及不是软件bug的承诺。这些都不能从该次错误单独证明。

## 实际修改

- 认证类用户标题统一为“认证或访问校验未通过”。401/403说明本次请求被拒绝，列出访问校验、资源权限、链接有效期与凭据条件等可能范围，不断言此前已登录或Cookie过期。
- 移除未经证实的PO Token根因、软件无bug结论、robots.txt导出Cookie不轮换及12小时恢复承诺。建议仅为确认浏览器中有权访问、检查当前工具版本/网络，以及在需要时配置自己合法凭据，不自动安装或更改访问策略。
- 保留实际工具证据：Fresh cookies / Cookies (not necessarily logged in)明确提示新鲜站点Cookie不一定需要登录；明确login required等才描述账号权限要求；bot/captcha提示只说明进一步访问校验，不推断某个具体技术原因。
- probe与download继续先脱敏，再将证据交给独立小模块选择固定文案；不反射原始Cookie、URL或认证头。DRM分类优先和拒绝文案保持。

代码在manager/diagnose/access_messages.rs，既有diagnose分类模块与两个媒体翻译调用点。未改网络、代理、下载、重试或安装行为。

## 兼容与验证

保留历史序列化类别auth-expired、login-expired；它们是兼容标识，不再被当作已确认凭据过期的事实。命令、协议、请求、建议动作ID和DRM优先级保持。原有固化错误文案的断言改为新契约，没有删除失败用例。

新增7项回归覆盖Fresh Cookie与明确账号要求的区别、纯403无登录/PO Token断言、YouTube bot提示没有恢复承诺、DRM优先、诊断脱敏与两个旧序列化ID。已通过rustfmt解析、git diff --check，并纳入本补丁首轮round9实际执行：**1461通过、0失败、16显式忽略，100.02秒；另Range9通过、0失败，5.06秒**。日志为%TEMP%/maobu-final-cargo-round9.log；没有将此前旧round8数字作为本补丁的结果。

随后蓝奏端点及抖音作品源选择补丁已纳入最终round10再次完整执行：**1471通过、0失败、16显式忽略，97.61秒；另Range9通过、0失败，5.07秒**，日志%TEMP%/maobu-final-cargo-round10.log，274份Rust源码快照Changed=0。上述7项诊断回归保留并再次通过；先前round9保留为本补丁首轮行为证据，不冒充后续完整源码的唯一门禁。

公开样本原始失败保留，不因为错误提示改善就改写为成功。抖音作品源选择是另一个实际下载逻辑修复，其后两样本×直连/环境代理4次完整复测通过，不能归功于文案变化；详见[最新公网结果](2026-10-31-public-media-followup.md)、[四平台补充](2026-10-31-public-four-platform-followup.md)和[平台矩阵](2026-10-31-platform-validation-matrix.md)。release round3及原生EXE smoke仍待验，本文不宣布整体发布完成。
