# 透明进度悬浮窗：显示效果细调

日期：2026-11-01（Asia/Shanghai）。项目版本0.10.11。本轮只调整[局部样式](../../src/components/progress/progress-window.css)与[任务行显示](../../src/components/progress/ProgressTaskRow.tsx)，没有修改原生窗口、下载逻辑、API、持久设置或依赖。

## 当前显示

- 常态继续为**224×28逻辑像素**的透明细条；悬停/聚焦后的3条与6条详情仍为224×190、224×298，底部锚点不变。
- 去掉数字周围的模糊描边，仅在任务数和百分比下增加小块半透明衬底。保持其余常态区域透明，避免不同背景下文字发虚。
- 浅、深主题分别设置轨道和细边颜色；主条5px、任务条3px，保留真实进度，不加入跑动或模拟填充。
- 文件名使用500字重，百分比改为整数，速度固定宽度并对齐；操作按钮调整至22px，保留键盘焦点与禁用反馈。
- 校验、等待网络及获取元数据时优先显示状态，避免把下载字节百分比误读为校验进度；未知长度仍不伪造百分比。

## 实际验证

| 项目 | 结果及证据 | 范围 |
| --- | --- | --- |
| 前端检查与构建 | 根任务确认check、build、extension:build均通过；[check日志](C:/Users/20269/AppData/Local/Temp/maobu-polish-check.log)、[build日志](C:/Users/20269/AppData/Local/Temp/maobu-polish-build.log)、[扩展构建日志](C:/Users/20269/AppData/Local/Temp/maobu-polish-extension.log) | 已读取核对27项前端规则、162命令/161调用契约及扩展193通过/0失败；主脚本体积提示仍保留。 |
| 核心浮窗交互 | **22组通过、18图，errors=0**；[实际结果JSON](C:/Users/20269/AppData/Local/Temp/maobu-compact-ui-jWO1dT/results.json) | 包括新增校验状态表达，以及透明、底锚、3/6条、键盘、快速进出、操作忙碌、错误重试和正式入口背景隔离。 |
| 不同背景显示 | **6组通过、12图，errors=0**；[背景检查JSON](C:/Users/20269/AppData/Local/Temp/maobu-progress-backgrounds-qOKk2a/results.json) | 浅/深背景交叉主题、文档背景、100%/125%和6条详情；尺寸、控件边界、底条位置及文档透明断言通过。根任务与背景检查agent另有截图目检记录。 |
| Rust证据复用 | **293份Rust源文件SHA与full15一致，Changed=0**；[源快照](C:/Users/20269/AppData/Local/Temp/maobu-final-round15-b5773ea817bc4c7baf806221a48489e8/rust-source-snapshot.json) | 本轮没有重新执行cargo。沿用此前full15的1497通过/0失败/16忽略及Range9通过，忽略项不计成功，详见[极简版实现报告](2026-10-31-compact-progress-window.md)。 |
| release7候选 | **构建实际exit 0，fresh静态13/13通过**；[实际检查JSON](C:/Users/20269/AppData/Local/Temp/maobu-release-round7-fcbb0e37a6494db986ea5a528fa19873/release-inspection.json) | 已读取核对构建结果与fresh证据，起点UTC`2026-10-31T17:16:43.7650822Z`。当前NSIS5,318,249B、MSI7,090,176B、便携EXE14,182,912B；新SHA及清单见[发布专项](2026-10-31-release-inspection.md)，不使用release6旧SHA。 |

## 证据边界

以上为实际React在隔离浏览器中的检查，使用API替身并模拟延迟HWND尺寸更新；背景为测试用页面内容。截图、透明CSS和边界断言不能证明Windows原生透明合成、置顶、焦点、多屏/DPI或点击穿透。本轮没有恢复此前停止的桌面控制，没有启动新客户端或安装器，也没有变更当前用户任务。

[当前细条预览](C:/Users/20269/AppData/Local/Temp/maobu-compact-ui-jWO1dT/compact-preview.png)仍为224×28；本次3任务、50%进度示例中，4814/6272像素完全透明，约76.8%。这个比例只描述该浏览器导出示例，不是任意任务数、主题、状态或原生Windows合成效果的保证。

交互、尺寸与数据来源继续沿用[极简透明浮窗专项](2026-10-31-compact-progress-window.md)。源码及自动化通过不等于原生桌面与安装生命周期已经验收；整体未验项继续保留在[整改台账](2026-10-31-remediation-status.md)。
