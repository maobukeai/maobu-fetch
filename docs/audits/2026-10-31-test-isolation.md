# 测试隔离边界与自启项修复

测试数据库、下载内容、BT会话及浏览器配置均使用唯一临时目录；原生管理器测试只创建一个移除所有窗口的Wry context，不运行真实单实例插件或主窗口setup，也不占用固定17433。显式开启的浏览器联动测试会另在随机loopback端口运行生产桥Router，不能将默认Harness的隔离说明扩大为所有测试都没有本地桥。

复查仍发现旧`autostart::test_sync_autostart_non_failing`及调用`save_settings`的原生生命周期测试会操作真实HKCU Run项。数据库临时目录不能隔离Windows注册表。该入口已改为`cfg(test)`无全局副作用，实际写入/删除/幂等行为在唯一`Software\MaobuFetchTests\autostart-UUID`专属键测试。

本机核对仅查询正式客户端设置中的`auto_start`一个标量（SQLite只读模式，不实例化Store，不读取Cookie、凭证或任务列表）：原设置为开启，而真实Run项已缺失。已将`app.lumaget.desktop`恢复为正在运行的正式客户端EXE命令，并读回核验；未更改客户端数据库。已向用户说明测试影响及恢复情况。

后续真实release EXE烟雾验证同时设置：

```powershell
$env:MAOBU_FETCH_DATA_DIR = '<唯一临时数据目录>'
$env:MAOBU_FETCH_ISOLATED_TEST = '1'
```

第二个标志禁止该验证会话写真实自启/文件关联全局状态。只有独立测试进程接收这些环境变量，不持久修改用户环境。正式运行缺省仍使用原有系统集成功能与兼容标识；不会新增演示任务、模拟统计、测试令牌或后台联网扫描。

所有测试必须保留实际边界：专属注册表根下的行为证明不能外推为用户已经选定新的系统默认应用；本地swarm不能外推公网BT互通；API参数匹配不能外推所有命令已经逐一触发Windows副作用。
