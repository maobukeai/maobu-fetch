import { useEffect, useState } from "react";
import { ChevronDown, ChevronUp, LoaderCircle, Plus, RefreshCw, Trash2 } from "lucide-react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import type { CompletionAction, TaskTemplate, TaskTemplateTestResult } from "../../types";
import { Select } from "../Select";
import { Modal } from "../common/Modal";
import { Field, SettingsGroup } from "../common/FormComponents";
import { CompletionActionEditor } from "../CompletionActionEditor";

export function TaskTemplatesPanel({
  notify,
}: {
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [templates, setTemplates] = useState<TaskTemplate[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<TaskTemplate | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testUrl, setTestUrl] = useState("");
  const [testResult, setTestResult] = useState<TaskTemplateTestResult | null>(null);
  const [headersText, setHeadersText] = useState("");

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.taskTemplateList();
      setTemplates(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const startAdd = () => {
    const maxPriority = templates.reduce((max, t) => Math.max(max, t.priority), -1);
    const tpl: TaskTemplate = {
      id: `tpl-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: "",
      domain_pattern: "",
      connections: null,
      speed_limit: null,
      headers: null,
      destination: null,
      completion_action: null,
      enabled: true,
      priority: maxPriority + 1,
    };
    setEditing(tpl);
    setHeadersText("");
    setIsNew(true);
  };

  const startEdit = (tpl: TaskTemplate) => {
    setEditing({ ...tpl });
    setHeadersText(
      tpl.headers
        ? Object.entries(tpl.headers)
            .map(([k, v]) => `${k}: ${v}`)
            .join("\n")
        : ""
    );
    setIsNew(false);
  };

  const parseHeaders = (text: string): Record<string, string> | null => {
    const trimmed = text.trim();
    if (!trimmed) return null;
    const result: Record<string, string> = {};
    for (const line of trimmed.split(/\r?\n/)) {
      const lineTrim = line.trim();
      if (!lineTrim) continue;
      const idx = lineTrim.indexOf(":");
      if (idx <= 0) {
        throw new Error(`请求头格式错误：${line}（应为 Key: Value）`);
      }
      const key = lineTrim.slice(0, idx).trim();
      const value = lineTrim.slice(idx + 1).trim();
      if (!key) throw new Error(`请求头键不能为空：${line}`);
      result[key] = value;
    }
    return Object.keys(result).length > 0 ? result : null;
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.name.trim()) {
      notify("模板名称不能为空", "error");
      return;
    }
    if (!editing.domain_pattern.trim()) {
      notify("域名匹配模式不能为空", "error");
      return;
    }
    if (
      editing.connections != null &&
      ![1, 2, 4, 8, 16, 32].includes(editing.connections)
    ) {
      notify("连接数只能是 1 / 2 / 4 / 8 / 16 / 32", "error");
      return;
    }
    let headers: Record<string, string> | null;
    try {
      headers = parseHeaders(headersText);
    } catch (error) {
      notify(String(error), "error");
      return;
    }
    const toSave: TaskTemplate = {
      ...editing,
      headers,
      destination: editing.destination?.trim() || null,
    };
    try {
      if (isNew) {
        await api.taskTemplateAdd(toSave);
        notify("已新增任务模板");
      } else {
        await api.taskTemplateUpdate(toSave);
        notify("已更新任务模板");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeTemplate = async (id: string) => {
    if (!confirm("确定删除此任务模板？")) return;
    try {
      await api.taskTemplateDelete(id);
      notify("已删除任务模板");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const toggleEnabled = async (tpl: TaskTemplate) => {
    const updated = { ...tpl, enabled: !tpl.enabled };
    try {
      await api.taskTemplateUpdate(updated);
      setTemplates((list) => list.map((t) => (t.id === tpl.id ? updated : t)));
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const movePriority = async (tpl: TaskTemplate, direction: -1 | 1) => {
    const sorted = [...templates].sort((a, b) => a.priority - b.priority);
    const index = sorted.findIndex((t) => t.id === tpl.id);
    if (index < 0) return;
    const targetIndex = direction === -1 ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= sorted.length) return;
    const other = sorted[targetIndex];
    const updatedSelf = { ...tpl, priority: other.priority };
    const updatedOther = { ...other, priority: tpl.priority };
    try {
      await api.taskTemplateUpdate(updatedSelf);
      await api.taskTemplateUpdate(updatedOther);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const runTest = async () => {
    if (!testUrl.trim()) {
      notify("请输入 URL 以测试模板匹配", "error");
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      const result = await api.taskTemplateTest(testUrl.trim());
      setTestResult(result);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setTesting(false);
    }
  };

  const fmtSpeed = (v?: number | null): string =>
    v ? `${Math.round(v / 1024)} KB/s` : "—";
  const fmtConnections = (v?: number | null): string => (v ? `${v} 路` : "—");
  const fmtDestination = (v?: string | null): string =>
    v && v.trim() ? v : "—";

  const actionLabel = (action?: CompletionAction | null): string => {
    if (!action || action === "none") return "—";
    if (action === "open-folder") return "打开文件夹";
    if (action === "run-file") return "运行文件";
    if (action === "shutdown") return "关机";
    if (action === "hibernate") return "休眠";
    if (action === "quit") return "退出应用";
    if (typeof action === "object" && "run-command" in action) return "运行命令";
    if (typeof action === "object" && "copy-to" in action) return "复制到";
    if (typeof action === "object" && "move-to" in action) return "移动到";
    return "—";
  };

  return (
    <SettingsGroup title="任务模板">
      <p className="settings-note">
        新任务根据域名（支持 *.example.com 通配）自动套用连接数、限速、目录等设置。已手动配置的字段不会被覆盖。
      </p>
      <div className="category-rules-toolbar">
        <button className="input-button" onClick={startAdd}>
          <Plus size={13} />
          <span>新增模板</span>
        </button>
        <button className="input-button" onClick={() => void reload()}>
          <RefreshCw size={13} />
          <span>刷新</span>
        </button>
      </div>
      {loading ? (
        <LoaderCircle className="spin" />
      ) : templates.length === 0 ? (
        <p className="settings-note">暂无任务模板。</p>
      ) : (
        <div className="category-rules-list" role="table">
          <div className="category-rule-row category-rule-row-header" role="row">
            <span className="category-rule-priority">优先级</span>
            <span className="category-rule-name">名称</span>
            <span className="category-rule-pattern">域名模式</span>
            <span className="preset-connections">连接数</span>
            <span>限速</span>
            <span>保存目录</span>
            <span>完成动作</span>
            <span className="category-rule-enabled">启用</span>
            <span className="category-rule-actions">操作</span>
          </div>
          {templates.map((tpl, index) => (
            <div key={tpl.id} className="category-rule-row" role="row">
              <span className="category-rule-priority" role="cell">
                {tpl.priority}
              </span>
              <span className="category-rule-name" role="cell" title={tpl.name}>
                {tpl.name}
              </span>
              <span
                className="category-rule-pattern"
                role="cell"
                title={tpl.domain_pattern}
              >
                <code>{tpl.domain_pattern}</code>
              </span>
              <span className="preset-connections" role="cell">
                {fmtConnections(tpl.connections)}
              </span>
              <span role="cell">{fmtSpeed(tpl.speed_limit)}</span>
              <span role="cell" title={tpl.destination ?? ""}>
                {fmtDestination(tpl.destination)}
              </span>
              <span role="cell">{actionLabel(tpl.completion_action)}</span>
              <span className="category-rule-enabled" role="cell">
                <input
                  type="checkbox"
                  className="toggle"
                  checked={tpl.enabled}
                  onChange={() => void toggleEnabled(tpl)}
                  aria-label={`${tpl.name} 启用状态`}
                />
              </span>
              <span className="category-rule-actions" role="cell">
                <button
                  title="上移"
                  disabled={index === 0}
                  onClick={() => void movePriority(tpl, -1)}
                >
                  <ChevronUp size={13} />
                </button>
                <button
                  title="下移"
                  disabled={index === templates.length - 1}
                  onClick={() => void movePriority(tpl, 1)}
                >
                  <ChevronDown size={13} />
                </button>
                <button title="编辑" onClick={() => startEdit(tpl)}>
                  编辑
                </button>
                <button
                  title="删除"
                  className="danger"
                  onClick={() => void removeTemplate(tpl.id)}
                >
                  <Trash2 size={13} />
                </button>
              </span>
            </div>
          ))}
        </div>
      )}
      <div className="category-rule-test-section" style={{ marginTop: 16 }}>
        <h3>测试模板匹配</h3>
        <Field label="URL">
          <input
            value={testUrl}
            onChange={(e) => setTestUrl(e.target.value)}
            placeholder="https://api.github.com/users/octocat"
          />
        </Field>
        <button
          className="input-button"
          disabled={testing}
          onClick={() => void runTest()}
        >
          {testing ? "测试中…" : "测试命中"}
        </button>
        {testResult && (
          <p
            className={`category-rule-test-result ${
              testResult.matched ? "ok" : "miss"
            }`}
            role="status"
          >
            {testResult.matched ? (
              <>
                命中 · 模板：
                <code>
                  {testResult.matched_template_name ??
                    testResult.matched_template_id}
                </code>
              </>
            ) : (
              "未命中"
            )}
          </p>
        )}
      </div>
      {editing && (
        <Modal
          title={isNew ? "新增任务模板" : "编辑任务模板"}
          onClose={() => setEditing(null)}
          style={{ width: "560px" }}
        >
          <div className="category-rule-edit-form">
            <div className="template-edit-grid">
              <Field label="名称">
                <input
                  value={editing.name}
                  onChange={(e) => setEditing({ ...editing, name: e.target.value })}
                  placeholder="例如：GitHub 大文件"
                />
              </Field>
              <Field label="域名匹配模式">
                <input
                  value={editing.domain_pattern}
                  onChange={(e) =>
                    setEditing({ ...editing, domain_pattern: e.target.value })
                  }
                  placeholder="github.com 或 *.github.com"
                />
              </Field>
              <Field label="优先级（数字越小越优先）">
                <input
                  type="number"
                  value={editing.priority}
                  onChange={(e) =>
                    setEditing({ ...editing, priority: +e.target.value })
                  }
                />
              </Field>
              <Field label="连接数（留空表示不覆盖；仅允许 1 / 2 / 4 / 8 / 16 / 32）">
                <Select
                  value={editing.connections ?? ""}
                  onChange={(val: any) => {
                    setEditing({
                      ...editing,
                      connections: val === "" ? null : +val,
                    });
                  }}
                  options={[
                    { value: "", label: "不覆盖" },
                    { value: 1, label: "1 路（单连接）" },
                    { value: 2, label: "2 路" },
                    { value: 4, label: "4 路" },
                    { value: 8, label: "8 路" },
                    { value: 16, label: "16 路" },
                    { value: 32, label: "32 路" },
                  ]}
                  ariaLabel="连接数"
                />
              </Field>
              <Field label="单任务限速（KB/s，0 或留空表示不限速）">
                <input
                  type="number"
                  min="0"
                  value={
                    editing.speed_limit
                      ? Math.round(editing.speed_limit / 1024)
                      : 0
                  }
                  onChange={(e) => {
                    const v = +e.target.value;
                    setEditing({
                      ...editing,
                      speed_limit: v > 0 ? v * 1024 : null,
                    });
                  }}
                />
              </Field>
              <Field label="启用">
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    height: "28px",
                  }}
                >
                  <input
                    className="toggle"
                    type="checkbox"
                    checked={editing.enabled}
                    onChange={(e) =>
                      setEditing({ ...editing, enabled: e.target.checked })
                    }
                  />
                </div>
              </Field>
              <Field className="wide" label="保存目录（留空表示不覆盖）">
                <div className="input-group">
                  <input
                    value={editing.destination ?? ""}
                    onChange={(e) =>
                      setEditing({
                        ...editing,
                        destination: e.target.value || null,
                      })
                    }
                    placeholder="例如：D:\\Downloads\\GitHub"
                  />
                  <button
                    className="input-button"
                    onClick={async () => {
                      const picked = await pickPath({
                        directory: true,
                        multiple: false,
                        title: "选择保存目录",
                      });
                      if (typeof picked === "string")
                        setEditing({ ...editing, destination: picked });
                    }}
                  >
                    选择目录
                  </button>
                </div>
              </Field>
              <Field className="wide" label="完成后动作（留空表示不覆盖）">
                <CompletionActionEditor
                  value={editing.completion_action ?? "none"}
                  onChange={(a) =>
                    setEditing({
                      ...editing,
                      completion_action: a === "none" ? null : a,
                    })
                  }
                  hidePowerOptions
                />
              </Field>
              <Field
                className="wide"
                label="请求头（每行一个，格式 Key: Value；留空表示不覆盖）"
              >
                <textarea
                  rows={3}
                  value={headersText}
                  onChange={(e) => setHeadersText(e.target.value)}
                  placeholder={
                    "Authorization: Bearer token\nUser-Agent: MaobuFetch"
                  }
                  style={{ width: "100%", fontFamily: "monospace" }}
                />
              </Field>
            </div>
            <div className="dialog-actions">
              <button onClick={() => setEditing(null)}>取消</button>
              <button className="primary" onClick={() => void saveEdit()}>
                保存
              </button>
            </div>
          </div>
        </Modal>
      )}
    </SettingsGroup>
  );
}
