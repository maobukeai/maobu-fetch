import { useEffect, useState } from "react";
import { ChevronDown, ChevronUp, LoaderCircle, Plus, RefreshCw, Sparkles, Trash2 } from "lucide-react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import type { CategoryRule, CategoryRuleType } from "../../types";
import { Select } from "../Select";
import { Modal } from "../common/Modal";
import { Field, SettingsGroup } from "../common/FormComponents";

const CATEGORY_RULE_TYPE_LABELS: Record<CategoryRuleType, string> = {
  domain: "域名",
  mime: "MIME 主类型",
  regex: "文件名正则",
};

function newRuleId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `rule-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function emptyRule(priority: number): CategoryRule {
  return {
    id: newRuleId(),
    name: "",
    rule_type: "domain",
    pattern: "",
    target_directory: "",
    enabled: true,
    priority,
  };
}

export function CategoryRulesPanel({
  notify,
}: {
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [rules, setRules] = useState<CategoryRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<CategoryRule | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testUrl, setTestUrl] = useState("");
  const [testFileName, setTestFileName] = useState("");
  const [testContentType, setTestContentType] = useState("");
  const [testResult, setTestResult] = useState<{ matched: boolean; target_directory: string } | null>(null);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.categoryRuleList();
      setRules(list);
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
    const maxPriority = rules.reduce((max, r) => Math.max(max, r.priority), -1);
    setEditing(emptyRule(maxPriority + 1));
    setIsNew(true);
  };

  const startEdit = (rule: CategoryRule) => {
    setEditing({ ...rule });
    setIsNew(false);
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.name.trim()) {
      notify("规则名称不能为空", "error");
      return;
    }
    if (!editing.pattern.trim()) {
      notify("匹配模式不能为空", "error");
      return;
    }
    if (!editing.target_directory.trim()) {
      notify("目标目录不能为空", "error");
      return;
    }
    try {
      if (isNew) {
        await api.categoryRuleAdd(editing);
        notify("已新增分类规则");
      } else {
        await api.categoryRuleUpdate(editing);
        notify("已更新分类规则");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeRule = async (id: string) => {
    if (!confirm("确定删除此分类规则？")) return;
    try {
      await api.categoryRuleDelete(id);
      notify("已删除分类规则");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const toggleEnabled = async (rule: CategoryRule) => {
    const updated = { ...rule, enabled: !rule.enabled };
    try {
      await api.categoryRuleUpdate(updated);
      setRules((list) => list.map((r) => (r.id === rule.id ? updated : r)));
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const movePriority = async (rule: CategoryRule, direction: -1 | 1) => {
    const sorted = [...rules].sort((a, b) => a.priority - b.priority);
    const index = sorted.findIndex((r) => r.id === rule.id);
    if (index < 0) return;
    const targetIndex = direction === -1 ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= sorted.length) return;
    const other = sorted[targetIndex];
    const updatedSelf = { ...rule, priority: other.priority };
    const updatedOther = { ...other, priority: rule.priority };
    try {
      await api.categoryRuleUpdate(updatedSelf);
      await api.categoryRuleUpdate(updatedOther);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const runTest = async () => {
    if (!editing) return;
    if (!testUrl.trim() && !testFileName.trim()) {
      notify("请输入 URL 或文件名以测试规则", "error");
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      const result = await api.categoryRuleTest(
        editing,
        testUrl.trim(),
        testFileName.trim(),
        testContentType.trim() || undefined
      );
      setTestResult(result);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setTesting(false);
    }
  };

  return (
    <SettingsGroup title="分类规则">
      <p className="settings-note">
        按优先级匹配域名、文件类型或文件名正则，自动设置新任务的保存目录（不覆盖手动指定的目录）。
      </p>
      <div className="category-rules-toolbar">
        <button className="input-button" onClick={startAdd}>
          <Plus size={13} />
          <span>新增规则</span>
        </button>
        <button className="input-button" onClick={() => void reload()}>
          <RefreshCw size={13} />
          <span>刷新</span>
        </button>
      </div>
      {loading ? (
        <LoaderCircle className="spin" />
      ) : rules.length === 0 ? (
        <p className="settings-note">暂无分类规则。</p>
      ) : (
        <div className="category-rules-list" role="table">
          <div className="category-rule-row category-rule-row-header" role="row">
            <span className="category-rule-priority">优先级</span>
            <span className="category-rule-name">名称</span>
            <span className="category-rule-type">类型</span>
            <span className="category-rule-pattern">模式</span>
            <span className="category-rule-target">目标目录</span>
            <span className="category-rule-enabled">启用</span>
            <span className="category-rule-actions">操作</span>
          </div>
          {rules.map((rule, index) => (
            <div key={rule.id} className="category-rule-row" role="row">
              <span className="category-rule-priority" role="cell">
                {rule.priority}
              </span>
              <span className="category-rule-name" role="cell" title={rule.name}>
                {rule.name}
              </span>
              <span className="category-rule-type" role="cell">
                {CATEGORY_RULE_TYPE_LABELS[rule.rule_type]}
              </span>
              <span className="category-rule-pattern" role="cell" title={rule.pattern}>
                <code>{rule.pattern}</code>
              </span>
              <span className="category-rule-target" role="cell" title={rule.target_directory}>
                {rule.target_directory}
              </span>
              <span className="category-rule-enabled" role="cell">
                <input
                  type="checkbox"
                  className="toggle"
                  checked={rule.enabled}
                  onChange={() => void toggleEnabled(rule)}
                  aria-label={`${rule.name} 启用状态`}
                />
              </span>
              <span className="category-rule-actions" role="cell">
                <button
                  title="上移"
                  disabled={index === 0}
                  onClick={() => void movePriority(rule, -1)}
                >
                  <ChevronUp size={13} />
                </button>
                <button
                  title="下移"
                  disabled={index === rules.length - 1}
                  onClick={() => void movePriority(rule, 1)}
                >
                  <ChevronDown size={13} />
                </button>
                <button title="编辑" onClick={() => startEdit(rule)}>
                  编辑
                </button>
                <button
                  title="删除"
                  className="danger"
                  onClick={() => void removeRule(rule.id)}
                >
                  <Trash2 size={13} />
                </button>
              </span>
            </div>
          ))}
        </div>
      )}
      {editing && (
        <Modal
          title={isNew ? "新增分类规则" : "编辑分类规则"}
          headerAction={
            <label className="dialog-header-action">
              <span>启用此规则</span>
              <input
                className="toggle"
                type="checkbox"
                checked={editing.enabled}
                onChange={(e) => setEditing({ ...editing, enabled: e.target.checked })}
              />
            </label>
          }
          onClose={() => setEditing(null)}
          style={{ width: "520px" }}
        >
          <div className="category-rule-edit-form">
            <div className="form-grid-2">
              <Field label="名称">
                <input
                  value={editing.name}
                  onChange={(e) => setEditing({ ...editing, name: e.target.value })}
                  placeholder="例如：GitHub 仓库"
                />
              </Field>
              <Field label="匹配类型">
                <Select
                  value={editing.rule_type}
                  onChange={(val: any) =>
                    setEditing({ ...editing, rule_type: val as CategoryRuleType })
                  }
                  options={[
                    { value: "domain", label: "域名（支持子域名）" },
                    { value: "mime", label: "MIME 主类型" },
                    { value: "regex", label: "文件名正则" },
                  ]}
                  ariaLabel="匹配类型"
                />
              </Field>
            </div>

            <div className="form-grid-2">
              <Field
                label={
                  editing.rule_type === "domain"
                    ? "域名（如 github.com）"
                    : editing.rule_type === "mime"
                    ? "主类型（如 video）"
                    : "正则表达式（如 \\.mp4$）"
                }
              >
                <input
                  value={editing.pattern}
                  onChange={(e) => setEditing({ ...editing, pattern: e.target.value })}
                  placeholder={
                    editing.rule_type === "domain"
                      ? "github.com"
                      : editing.rule_type === "mime"
                      ? "video"
                      : "\\.mp4$"
                  }
                />
              </Field>
              <Field label="优先级（越小越先）">
                <input
                  type="number"
                  value={editing.priority}
                  onChange={(e) => setEditing({ ...editing, priority: +e.target.value })}
                />
              </Field>
            </div>

            <Field label="目标目录">
              <div className="input-group">
                <input
                  value={editing.target_directory}
                  onChange={(e) =>
                    setEditing({ ...editing, target_directory: e.target.value })
                  }
                  placeholder="例如：D:\\Downloads\\GitHub"
                />
                <button
                  className="input-button"
                  onClick={async () => {
                    const picked = await pickPath({
                      directory: true,
                      multiple: false,
                      title: "选择目标目录",
                    });
                    if (typeof picked === "string")
                      setEditing({ ...editing, target_directory: picked });
                  }}
                >
                  选择目录
                </button>
              </div>
            </Field>

            <details className="category-rule-test-details">
              <summary
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                }}
              >
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: "6px",
                  }}
                >
                  <Sparkles size={12} /> 测试规则匹配
                </span>
                {testResult && (
                  <small
                    className={`category-rule-test-badge ${
                      testResult.matched ? "ok" : "miss"
                    }`}
                    style={{
                      fontSize: "10px",
                      padding: "1px 6px",
                      borderRadius: "4px",
                      fontWeight: "normal",
                      background: testResult.matched
                        ? "rgba(52, 199, 89, 0.15)"
                        : "rgba(255, 59, 48, 0.15)",
                      color: testResult.matched ? "var(--success)" : "var(--danger)",
                    }}
                  >
                    {testResult.matched ? "✓ 已命中" : "× 未命中"}
                  </small>
                )}
              </summary>
              <div className="category-rule-test-body">
                <Field label="测试 URL">
                  <input
                    value={testUrl}
                    onChange={(e) => setTestUrl(e.target.value)}
                    placeholder="https://api.github.com/users/octocat"
                  />
                </Field>
                <div className="form-grid-2">
                  <Field label="测试文件名">
                    <input
                      value={testFileName}
                      onChange={(e) => setTestFileName(e.target.value)}
                      placeholder="octocat.json"
                    />
                  </Field>
                  <Field label="Content-Type（可选）">
                    <input
                      value={testContentType}
                      onChange={(e) => setTestContentType(e.target.value)}
                      placeholder="application/json"
                    />
                  </Field>
                </div>
                <div className="category-rule-test-actions">
                  <button
                    className="input-button primary-border"
                    disabled={testing}
                    onClick={() => void runTest()}
                  >
                    {testing ? "测试中…" : "测试命中"}
                  </button>
                  <button
                    className="input-button"
                    type="button"
                    onClick={() => {
                      const rawPat = editing.pattern.trim();
                      let sampleUrl = "";
                      let sampleFile = "";
                      let sampleMime = "";

                      if (editing.rule_type === "domain") {
                        let dom = rawPat.replace(/^https?:\/\//i, "").split("/")[0].trim();
                        dom = dom || "github.com";
                        sampleUrl = `https://${dom}/archive/download_sample.zip`;
                        sampleFile = "download_sample.zip";
                        sampleMime = "application/zip";
                      } else if (editing.rule_type === "mime") {
                        let mime = rawPat.split("/")[0].trim().toLowerCase() || "video";
                        const ext =
                          mime === "video"
                            ? "mp4"
                            : mime === "image"
                            ? "png"
                            : mime === "audio"
                            ? "mp3"
                            : "bin";
                        sampleUrl = `https://example.com/media/sample_file.${ext}`;
                        sampleFile = `sample_file.${ext}`;
                        sampleMime = rawPat.includes("/")
                          ? rawPat
                          : `${mime}/octet-stream`;
                      } else {
                        let sampleName = "download_sample.zip";
                        if (rawPat) {
                          const extMatch =
                            /\.(mp4|mkv|avi|mov|mp3|flac|wav|zip|rar|7z|tar|gz|exe|msi|pdf|epub|png|jpg|jpeg|webp)\b/i.exec(
                              rawPat
                            );
                          if (extMatch) {
                            sampleName = `sample_file${extMatch[0]}`;
                          } else {
                            const cleaned = rawPat
                              .replace(/[\^$\\().*+?\[\]{}|]/g, "")
                              .trim();
                            if (cleaned) sampleName = cleaned;
                          }
                        }
                        sampleUrl = `https://example.com/files/${sampleName}`;
                        sampleFile = sampleName;
                        sampleMime = "application/octet-stream";
                      }

                      setTestUrl(sampleUrl);
                      setTestFileName(sampleFile);
                      setTestContentType(sampleMime);

                      void (async () => {
                        setTesting(true);
                        try {
                          const res = await api.categoryRuleTest(
                            editing,
                            sampleUrl,
                            sampleFile,
                            sampleMime || undefined
                          );
                          setTestResult(res);
                        } catch {
                          setTestResult({ matched: false, target_directory: "" });
                        } finally {
                          setTesting(false);
                        }
                      })();
                    }}
                  >
                    填入示例并测试
                  </button>
                  {testResult && (
                    <span
                      className={`category-rule-test-result ${
                        testResult.matched ? "ok" : "miss"
                      }`}
                      role="status"
                    >
                      {testResult.matched ? (
                        <>
                          命中 · 目标目录：
                          <code>{testResult.target_directory}</code>
                        </>
                      ) : (
                        "未命中"
                      )}
                    </span>
                  )}
                </div>
              </div>
            </details>

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
