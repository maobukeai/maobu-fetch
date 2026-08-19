import { useEffect, useState } from "react";
import { LoaderCircle, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api } from "../../api";
import type { CompletionAction, DownloadPreset } from "../../types";
import { Select } from "../Select";
import { Modal } from "../common/Modal";
import { Field, SettingsGroup } from "../common/FormComponents";
import { CompletionActionEditor } from "../CompletionActionEditor";

export function PresetsPanel({
  notify,
}: {
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [presets, setPresets] = useState<DownloadPreset[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<DownloadPreset | null>(null);
  const [isNew, setIsNew] = useState(false);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.presetList();
      setPresets(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const actionLabel = (action?: CompletionAction | null): string => {
    if (!action || action === "none") return "无";
    if (action === "open-folder") return "打开文件夹";
    if (action === "run-file") return "运行文件";
    if (action === "shutdown") return "关机";
    if (action === "hibernate") return "休眠";
    return "无";
  };

  const startAdd = () => {
    const newPreset: DownloadPreset = {
      id: `custom-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: "",
      connections: 8,
      speed_limit: null,
      completion_action: null,
      verify_checksum: false,
      scheduled_at: null,
      is_builtin: false,
    };
    setEditing(newPreset);
    setIsNew(true);
  };

  const startEdit = (preset: DownloadPreset) => {
    setEditing({ ...preset });
    setIsNew(false);
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.name.trim()) {
      notify("预设名称不能为空", "error");
      return;
    }
    if (!editing.id.trim()) {
      notify("预设 ID 不能为空", "error");
      return;
    }
    if (![1, 2, 4, 8, 16, 32].includes(editing.connections)) {
      notify("连接数只能是 1 / 2 / 4 / 8 / 16 / 32", "error");
      return;
    }
    if (editing.scheduled_at && !/^\d{1,2}:\d{2}$/.test(editing.scheduled_at.trim())) {
      notify("计划时间格式应为 HH:MM（24 小时制）", "error");
      return;
    }
    try {
      if (isNew) {
        await api.presetAdd(editing);
        notify("已新增下载预设");
      } else {
        await api.presetUpdate(editing);
        notify("已更新下载预设");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removePreset = async (id: string) => {
    if (!confirm("确定删除此下载预设？")) return;
    try {
      await api.presetDelete(id);
      notify("已删除下载预设");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  return (
    <SettingsGroup title="下载预设">
      <p className="settings-note">
        预设用于快速套用一组下载参数。内置预设可编辑但不可删除，自定义预设可任意增删改。
      </p>
      <div className="category-rules-toolbar">
        <button className="input-button" onClick={startAdd}>
          <Plus size={13} />
          <span>新增预设</span>
        </button>
        <button className="input-button" onClick={() => void reload()}>
          <RefreshCw size={13} />
          <span>刷新</span>
        </button>
      </div>
      {loading ? (
        <LoaderCircle className="spin" />
      ) : presets.length === 0 ? (
        <p className="settings-note">暂无下载预设。</p>
      ) : (
        <div className="category-rules-list" role="table">
          <div className="category-rule-row category-rule-row-header preset-row" role="row">
            <span>名称</span>
            <span>连接数</span>
            <span>限速</span>
            <span>完成动作</span>
            <span>校验</span>
            <span>计划时间</span>
            <span>操作</span>
          </div>
          {presets.map((preset) => (
            <div key={preset.id} className="category-rule-row preset-row" role="row">
              <span role="cell" title={preset.name}>
                {preset.name}
                {preset.is_builtin && <span className="preset-builtin-badge">内置</span>}
              </span>
              <span className="preset-connections" role="cell">
                {preset.connections} 路
              </span>
              <span role="cell">
                {preset.speed_limit
                  ? `${Math.round(preset.speed_limit / 1024)} KB/s`
                  : "不限速"}
              </span>
              <span role="cell">{actionLabel(preset.completion_action)}</span>
              <span className="preset-verify" role="cell">
                {preset.verify_checksum ? "是" : "否"}
              </span>
              <span className="preset-scheduled" role="cell">
                {preset.scheduled_at || "—"}
              </span>
              <span className="category-rule-actions" role="cell">
                <button title="编辑" onClick={() => startEdit(preset)}>
                  编辑
                </button>
                <button
                  title="删除"
                  className="danger"
                  disabled={preset.is_builtin}
                  onClick={() => void removePreset(preset.id)}
                  aria-label={`删除预设 ${preset.name}`}
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
          title={isNew ? "新增下载预设" : "编辑下载预设"}
          onClose={() => setEditing(null)}
          style={{ width: "520px" }}
        >
          <div className="category-rule-edit-form">
            <Field label="ID（不可修改）">
              <input value={editing.id} disabled />
            </Field>
            <Field label="名称">
              <input
                value={editing.name}
                onChange={(e) => setEditing({ ...editing, name: e.target.value })}
                placeholder="例如：影视下载"
              />
            </Field>
            <Field label="连接数（仅允许 1 / 2 / 4 / 8 / 16 / 32）">
              <Select
                value={editing.connections}
                onChange={(val: any) =>
                  setEditing({ ...editing, connections: +val })
                }
                options={[
                  { value: 1, label: "1 路（单连接）" },
                  { value: 2, label: "2 路" },
                  { value: 4, label: "4 路" },
                  { value: 8, label: "8 路（默认）" },
                  { value: 16, label: "16 路" },
                  { value: 32, label: "32 路（大文件）" },
                ]}
                ariaLabel="连接数"
              />
            </Field>
            <Field label="单任务限速（KB/s，0 表示不限速）">
              <input
                type="number"
                min="0"
                value={
                  editing.speed_limit
                    ? Math.round(editing.speed_limit / 1024)
                    : 0
                }
                onChange={(e) =>
                  setEditing({
                    ...editing,
                    speed_limit: +e.target.value ? +e.target.value * 1024 : null,
                  })
                }
              />
            </Field>
            <Field label="计划时间（HH:MM 24 小时制，留空表示立即开始）">
              <input
                value={editing.scheduled_at ?? ""}
                onChange={(e) =>
                  setEditing({
                    ...editing,
                    scheduled_at: e.target.value || null,
                  })
                }
                placeholder="例如：22:00"
              />
            </Field>
            <Field className="wide" label="完成后动作">
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
            <label className="setting-row">
              <div>
                <strong>完成后校验 SHA-256</strong>
              </div>
              <input
                className="toggle"
                type="checkbox"
                checked={editing.verify_checksum}
                onChange={(e) =>
                  setEditing({ ...editing, verify_checksum: e.target.checked })
                }
              />
            </label>
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
