import { useEffect, useState } from "react";
import { ChevronDown, ChevronUp, LoaderCircle, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api } from "../../api";
import type { FilenameCleanupRule } from "../../types";
import { Modal } from "../common/Modal";
import { Field, SettingsGroup } from "../common/FormComponents";

const BUILTIN_RULE_IDS = new Set([
  "remove-bracket-site",
  "remove-chinese-bracket-site",
  "remove-chinese-bracket-promo",
  "remove-paren-quality",
  "remove-square-bracket-quality",
  "remove-media-codec-tags",
  "remove-underscore-site",
  "remove-copy-suffix",
  "collapse-spaces",
  "strip-trailing-spaces",
]);

export function FilenameCleanupPanel({
  notify,
}: {
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [rules, setRules] = useState<FilenameCleanupRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<FilenameCleanupRule | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testFileName, setTestFileName] = useState("");
  const [testResult, setTestResult] = useState<string | null>(null);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.filenameCleanupRuleList();
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
    const maxPriority = rules.reduce((max, r) => Math.max(max, r.priority), 39);
    const empty: FilenameCleanupRule = {
      id: `custom-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: "",
      pattern: "",
      replacement: "",
      enabled: true,
      priority: maxPriority + 10,
    };
    setEditing(empty);
    setIsNew(true);
    setTestResult(null);
  };

  const startEdit = (rule: FilenameCleanupRule) => {
    setEditing({ ...rule });
    setIsNew(false);
    setTestResult(null);
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.name.trim()) {
      notify("规则名称不能为空", "error");
      return;
    }
    if (!editing.pattern.trim()) {
      notify("正则模式不能为空", "error");
      return;
    }
    if (!editing.id.trim()) {
      notify("规则 ID 不能为空", "error");
      return;
    }
    try {
      if (isNew) {
        await api.filenameCleanupRuleAdd(editing);
        notify("已新增文件名清理规则");
      } else {
        await api.filenameCleanupRuleUpdate(editing);
        notify("已更新文件名清理规则");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeRule = async (id: string) => {
    if (BUILTIN_RULE_IDS.has(id)) {
      notify("内置规则不可删除，可禁用或编辑", "error");
      return;
    }
    if (!confirm("确定删除此文件名清理规则？")) return;
    try {
      await api.filenameCleanupRuleDelete(id);
      notify("已删除文件名清理规则");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const toggleEnabled = async (rule: FilenameCleanupRule) => {
    const updated = { ...rule, enabled: !rule.enabled };
    try {
      await api.filenameCleanupRuleUpdate(updated);
      setRules((list) => list.map((r) => (r.id === rule.id ? updated : r)));
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const movePriority = async (rule: FilenameCleanupRule, direction: -1 | 1) => {
    const sorted = [...rules].sort((a, b) => a.priority - b.priority);
    const index = sorted.findIndex((r) => r.id === rule.id);
    if (index < 0) return;
    const targetIndex = direction === -1 ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= sorted.length) return;
    const other = sorted[targetIndex];
    const updatedSelf = { ...rule, priority: other.priority };
    const updatedOther = { ...other, priority: rule.priority };
    try {
      await api.filenameCleanupRuleUpdate(updatedSelf);
      await api.filenameCleanupRuleUpdate(updatedOther);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const runTest = async () => {
    if (!editing) return;
    if (!testFileName.trim()) {
      notify("请输入测试文件名", "error");
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      const result = await api.filenameCleanupPreview(testFileName.trim(), [editing]);
      setTestResult(result);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setTesting(false);
    }
  };

  return (
    <SettingsGroup title="文件名清理规则">
      <p className="settings-note">
        保存文件前按优先级进行正则替换，用以去除水印、标记或多余空格。仅对未手动编辑文件名的任务生效。
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
        <p className="settings-note">暂无文件名清理规则。</p>
      ) : (
        <div className="category-rules-list" role="table">
          <div className="category-rule-row category-rule-row-header filename-cleanup-row" role="row">
            <span className="category-rule-priority">优先级</span>
            <span className="category-rule-name">名称</span>
            <span className="category-rule-pattern">正则模式</span>
            <span className="category-rule-target">替换为</span>
            <span className="category-rule-enabled">启用</span>
            <span className="category-rule-actions">操作</span>
          </div>
          {rules.map((rule, index) => (
            <div key={rule.id} className="category-rule-row filename-cleanup-row" role="row">
              <span className="category-rule-priority" role="cell">
                {rule.priority}
              </span>
              <span className="category-rule-name" role="cell" title={rule.name}>
                {rule.name}
                {BUILTIN_RULE_IDS.has(rule.id) && (
                  <small className="preset-builtin-badge">内置</small>
                )}
              </span>
              <span className="category-rule-pattern" role="cell" title={rule.pattern}>
                <code>{rule.pattern}</code>
              </span>
              <span
                className="category-rule-target"
                role="cell"
                title={rule.replacement || "（删除匹配内容）"}
              >
                {rule.replacement || "（删除）"}
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
                  title={BUILTIN_RULE_IDS.has(rule.id) ? "内置规则不可删除" : "删除"}
                  className="danger"
                  disabled={BUILTIN_RULE_IDS.has(rule.id)}
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
          title={isNew ? "新增文件名清理规则" : "编辑文件名清理规则"}
          headerAction={
            <label className="dialog-header-action">
              <span>启用规则</span>
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
            <div className="form-row-group">
              <Field label="名称">
                <input
                  value={editing.name}
                  onChange={(e) => setEditing({ ...editing, name: e.target.value })}
                  placeholder="例如：去除站点方括号"
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
            <div className="form-row-group">
              <Field label="正则模式（regex crate 语法）">
                <input
                  value={editing.pattern}
                  onChange={(e) => setEditing({ ...editing, pattern: e.target.value })}
                  placeholder="\\[(www\\.)?[\\w.-]+\\]"
                />
              </Field>
              <Field label="替换为（留空为删除）">
                <input
                  value={editing.replacement}
                  onChange={(e) => setEditing({ ...editing, replacement: e.target.value })}
                  placeholder="例如：$1 或空"
                />
              </Field>
            </div>
            <div className="category-rule-test-section">
              <h3>测试规则</h3>
              <div className="test-inline-row">
                <input
                  value={testFileName}
                  onChange={(e) => setTestFileName(e.target.value)}
                  placeholder="movie [www.example.com] (1080p).mp4"
                />
                <button
                  className="input-button"
                  disabled={testing}
                  onClick={() => void runTest()}
                >
                  {testing ? "测试中…" : "测试清理"}
                </button>
              </div>
              {testResult !== null && (
                <p className="category-rule-test-result ok" role="status">
                  清理结果：<code>{testResult || "（空）"}</code>
                </p>
              )}
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
