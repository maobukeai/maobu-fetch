# 简约苹果风布局重构设计规约 (Finder Layout Redesign Spec)

本设计规约描述了将猫步下载器（Maobu Fetch）的 UI 重新调整为 macOS Finder 经典的“单顶栏 + 右侧抽屉详情 + 侧边栏底部状态”布局方案，不改动现有的配色与控件样式，仅重构布局与结构。

## 1. 目标与设计原则

*   **垂直高度释放**：彻底废除占用多层空间的标题栏与工具栏，合并为高度约 52px 的单行 Header。
*   **水平空间利用**：将底部的 Details 面板移到右侧，采用竖直抽屉式设计（260px 宽度）。在折叠状态和展开状态下，任务列表高度均独占整个屏幕高度，消除分割截断。
*   **状态收纳**：将原工作区底端状态栏（24px）移除，所有下载状态信息与速度控制整合至左侧导航栏的底部。

## 2. 界面与组件详细设计

### 2.1 顶栏合并设计 (.titlebar)
原有的 `.titlebar` 和 `.command-bar` 融合成一个单一的 `.titlebar`。
*   **HTML 结构**：
    ```tsx
    <header className="titlebar">
      <h1>{sectionTitle}</h1>
      <div className="toolbar-actions">
        {/* 新建/开始/暂停/删除/打开文件/打开文件夹/刷新 按钮仅展示图标 */}
      </div>
      <label className="search-box">...</label>
      <button className="details-toggle" ...>{/* 详情面板收缩开关 */}</button>
    </header>
    ```
*   **CSS 属性**：
    *   高度固定为 `52px`（原 `78px` 与 `40px` 废除）。
    *   按钮使用 `flex` 居中对齐，移除文字，仅展示图标，并配备原有 tooltip 属性。

### 2.2 右侧详情面板设计 (.details-pane)
*   **HTML 结构**：
    *   主容器 `.content-grid` 包含 `.task-list-panel` 和 `.details-pane`。
    *   `.details-pane` 内部改为垂直布局：
        ```tsx
        <aside className="details-pane">
          <div className="details-header">
            <FileIcon category={task.category} />
            <h2>{task.file_name}</h2>
            <button onClick={onClose}><X size={14} /></button>
          </div>
          <div className="details-scroll">
            <dl className="details-list">
              {/* 垂直排布的属性键值对 */}
            </dl>
            {/* 垂直排列的分段连接进度列表 */}
            {/* 垂直堆叠的操作按钮 */}
          </div>
        </aside>
        ```
*   **CSS 属性**：
    *   `.content-grid` 设置为 `display: grid;`。
    *   当详情展开时：`grid-template-columns: minmax(0, 1fr) 260px;`（主表格液态自适应，详情区固定 260px）。
    *   `.details-pane` 设置 `border-left: 1px solid var(--border)`，高度 100%，`display: flex; flex-direction: column;`，内部支持纵向滚动。

### 2.3 侧边栏底部状态栏设计 (.nav-footer)
在 `.nav-pane` 内部底端构建 `.nav-footer`：
*   **HTML 结构**：
    ```tsx
    <div className="nav-footer">
      <button className="nav-settings" onClick={() => setSettingsOpen(true)}>
        <Settings size={15} /><span>设置</span>
      </button>
      <div className="nav-status" onClick={() => setSettingsOpen(true)}>
        <i className={`status-dot ${isDesktop ? 'online' : 'offline'}`} />
        <span>↓ {formatBytes(totalSpeed)}/s · {active.length} 活动</span>
      </div>
    </div>
    ```
*   **CSS 属性**：
    *   `.nav-footer` 使用 `margin-top: auto; padding-top: 12px; border-top: 1px solid var(--border);`。
    *   `.nav-status` 使用 `font-size: 10px; display: flex; align-items: center; gap: 6px; cursor: pointer; color: var(--subtle); margin-top: 6px;`。

## 3. 兼容性与非功能性约束

*   保留所有的类名、事件、命令与 api 接口调用。
*   不改动原有的苹果风格配色方案（强调色、字号、Toggle 控件等保持一致）。
*   页面最小宽度保持在 1040px 左右以防横向挤压。

## 4. 验证规约

### 4.1 编译检查
*   `pnpm run check` 零 TS 类型错误。
*   `pnpm run build` 打包无异常。

### 4.2 布局验证
*   验证右侧折叠面板展开/收起时主表格的拉伸行为。
*   验证主工作区底部不再有 statusbar。
*   验证侧边栏底部网速和连接指示灯的跳动显示。
