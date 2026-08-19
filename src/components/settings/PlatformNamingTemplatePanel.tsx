import { useEffect, useMemo, useState } from "react";
import { LoaderCircle, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api } from "../../api";
import type { PlatformNamingTemplate } from "../../types";
import { Select } from "../Select";
import { Modal } from "../common/Modal";
import { Field, SettingsGroup } from "../common/FormComponents";

const PLATFORM_LABELS: Array<[string, string]> = [
  ["douyin", "抖音"],
  ["tiktok", "TikTok"],
  ["twitter", "Twitter/X"],
  ["youtube", "YouTube"],
  ["bilibili", "哔哩哔哩"],
  ["weibo", "微博"],
  ["unknown", "未知平台"],
];

const platformLabel = (key: string): string =>
  PLATFORM_LABELS.find(([k]) => k === key)?.[1] ?? key;

export function PlatformNamingTemplatePanel({
  notify,
}: {
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [templates, setTemplates] = useState<PlatformNamingTemplate[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<PlatformNamingTemplate | null>(null);
  const [isNew, setIsNew] = useState(false);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.platformNamingTemplateList();
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
    const empty: PlatformNamingTemplate = {
      id: `template-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      platform: "douyin",
      template: "{author}_{title}_{date}",
      enabled: true,
      is_builtin: false,
    };
    setEditing(empty);
    setIsNew(true);
  };

  const startEdit = (template: PlatformNamingTemplate) => {
    setEditing({ ...template });
    setIsNew(false);
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.id.trim()) {
      notify("模板 ID 不能为空", "error");
      return;
    }
    if (!editing.platform.trim()) {
      notify("平台不能为空", "error");
      return;
    }
    if (!editing.template.trim()) {
      notify("模板内容不能为空", "error");
      return;
    }
    try {
      if (isNew) {
        await api.platformNamingTemplateAdd(editing);
        notify("已新增平台命名模板");
      } else {
        await api.platformNamingTemplateUpdate(editing);
        notify("已更新平台命名模板");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeTemplate = async (template: PlatformNamingTemplate) => {
    if (template.is_builtin) {
      notify("内置模板不可删除，可禁用或编辑", "error");
      return;
    }
    if (!confirm("确定删除此平台命名模板？")) return;
    try {
      await api.platformNamingTemplateDelete(template.id);
      notify("已删除平台命名模板");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const toggleEnabled = async (template: PlatformNamingTemplate) => {
    const updated = { ...template, enabled: !template.enabled };
    try {
      await api.platformNamingTemplateUpdate(updated);
      setTemplates((list) => list.map((t) => (t.id === template.id ? updated : t)));
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const previewFileName = useMemo(() => {
    if (!editing) return "";
    const sampleVars: Record<string, string> = {
      author: "张三",
      title: "示例标题",
      date: "20260720",
      platform: editing.platform,
      id: "7012345678901234567",
      channel: "示例频道",
      bvid: "BV1xx411c7mD",
    };
    let result = editing.template.replace(
      /\{(author|title|date|platform|id|channel|bvid)\}/g,
      (_, key: string) => sampleVars[key] ?? ""
    );
    result = result.replace(/_+/g, "_").replace(/^_+|_+$/g, "");
    return result || "media";
  }, [editing]);

  return (
    <SettingsGroup title="平台命名模板">
      <p className="settings-note">
        媒体下载完成后套用此文件名模板。仅对新建任务生效，支持变量替换，限制在 100 字符内。
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
        <p className="settings-note">暂无平台命名模板。</p>
      ) : (
        <div className="category-rules-list" role="table">
          <div className="category-rule-row category-rule-row-header" role="row">
            <span className="category-rule-name">平台</span>
            <span className="category-rule-pattern">模板</span>
            <span className="category-rule-enabled">启用</span>
            <span className="category-rule-actions">操作</span>
          </div>
          {templates.map((template) => (
            <div key={template.id} className="category-rule-row" role="row">
              <span className="category-rule-name" role="cell" title={template.platform}>
                {platformLabel(template.platform)}
                {template.is_builtin && <small className="preset-builtin-badge">内置</small>}
              </span>
              <span className="category-rule-pattern" role="cell" title={template.template}>
                <code>{template.template}</code>
              </span>
              <span className="category-rule-enabled" role="cell">
                <input
                  type="checkbox"
                  className="toggle"
                  checked={template.enabled}
                  onChange={() => void toggleEnabled(template)}
                  aria-label={`${platformLabel(template.platform)} 模板启用状态`}
                />
              </span>
              <span className="category-rule-actions" role="cell">
                <button title="编辑" onClick={() => startEdit(template)}>
                  编辑
                </button>
                <button
                  title={template.is_builtin ? "内置模板不可删除" : "删除"}
                  className="danger"
                  disabled={template.is_builtin}
                  onClick={() => void removeTemplate(template)}
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
          title={isNew ? "新增平台命名模板" : "编辑平台命名模板"}
          onClose={() => setEditing(null)}
          style={{ width: "540px" }}
        >
          <div className="category-rule-edit-form">
            <Field label="平台">
              <Select
                value={editing.platform}
                onChange={(val: any) =>
                  setEditing({ ...editing, platform: String(val) })
                }
                options={PLATFORM_LABELS.map(([key, label]) => ({
                  value: key,
                  label,
                }))}
                ariaLabel="平台"
              />
            </Field>
            <Field label="模板字符串">
              <input
                value={editing.template}
                onChange={(e) => setEditing({ ...editing, template: e.target.value })}
                placeholder="{author}_{title}_{date}"
              />
            </Field>
            <label className="setting-row">
              <div>
                <strong>启用</strong>
              </div>
              <input
                className="toggle"
                type="checkbox"
                checked={editing.enabled}
                onChange={(e) => setEditing({ ...editing, enabled: e.target.checked })}
              />
            </label>
            <div className="category-rule-test-section">
              <h3>变量说明</h3>
              <ul className="settings-note" style={{ paddingLeft: "20px", lineHeight: 1.7 }}>
                <li>
                  <code>{"{author}"}</code>：作者/上传者昵称（yt-dlp uploader/channel/uploader_id 优先级回退）
                </li>
                <li>
                  <code>{"{title}"}</code>：媒体标题
                </li>
                <li>
                  <code>{"{date}"}</code>：上传日期（YYYYMMDD 格式）
                </li>
                <li>
                  <code>{"{platform}"}</code>：平台 key（如 douyin / youtube）
                </li>
                <li>
                  <code>{"{id}"}</code>：站点视频 ID（如推文 ID、YouTube 视频 ID）
                </li>
                <li>
                  <code>{"{channel}"}</code>：频道名（YouTube 等平台有意义，与 author 区分）
                </li>
                <li>
                  <code>{"{bvid}"}</code>：B 站 BV 号（B 站场景下为 display_id）
                </li>
              </ul>
              <p className="settings-note">
                未知变量（如 <code>{"{foo}"}</code>）原样保留；缺失的已知变量替换为空。
                非法字符 <code>\ / : * ? &quot; &lt; &gt; |</code> 与控制字符替换为 <code>_</code>，压缩连续下划线。
              </p>
              <h3>预览（示例变量）</h3>
              <p className="category-rule-test-result ok" role="status">
                预览文件名：<code>{previewFileName}</code>
              </p>
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
