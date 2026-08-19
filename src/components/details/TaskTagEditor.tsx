import { useEffect, useState } from "react";
import { Plus, Tag as TagIcon } from "lucide-react";
import { api } from "../../api";
import type { DownloadTask, Tag } from "../../types";
import { newTagId } from "../../formatters";

export function TaskTagEditor({
  task,
  notify,
  onTagsChanged,
}: {
  task: DownloadTask;
  notify: (text: string, kind?: "ok" | "error") => void;
  onTagsChanged?: () => void;
}) {
  const [allTags, setAllTags] = useState<Tag[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState("#3B82F6");
  const [showCreateRow, setShowCreateRow] = useState(false);
  const taskId = task.id;

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setLoading(true);
      try {
        const [all, mine] = await Promise.all([
          api.tagList(),
          api.taskTagsGet(taskId),
        ]);
        if (cancelled) return;
        setAllTags(all);
        setSelectedIds(new Set(mine.map((t) => t.id)));
      } catch (error) {
        if (!cancelled) notify(String(error), "error");
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [taskId, notify]);

  const toggle = (id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const save = async () => {
    setSaving(true);
    try {
      await api.taskTagsSet(taskId, [...selectedIds]);
      notify("标签已更新");
      onTagsChanged?.();
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const createAndAttach = async () => {
    const name = newName.trim();
    if (!name) {
      notify("标签名称不能为空", "error");
      return;
    }
    if (!/^#[0-9A-Fa-f]{6}$/.test(newColor)) {
      notify("颜色格式必须为 #RRGGBB", "error");
      return;
    }
    setSaving(true);
    try {
      const created = await api.tagAdd({
        id: newTagId(),
        name,
        color: newColor,
      });
      setAllTags((current) =>
        [...current, created].sort((a, b) => a.name.localeCompare(b.name))
      );
      setSelectedIds((current) => new Set([...current, created.id]));
      await api.taskTagsSet(taskId, [...selectedIds, created.id]);
      setNewName("");
      setShowCreateRow(false);
      notify("已创建并附加标签");
      onTagsChanged?.();
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="details-section">
      <div className="details-section-title">
        <TagIcon size={13} />
        <span>标签</span>
      </div>
      {loading ? (
        <p className="muted">加载中…</p>
      ) : allTags.length === 0 ? (
        <p className="muted">暂无标签</p>
      ) : (
        <div className="tag-editor-grid" role="group" aria-label="选择标签">
          {allTags.map((tag) => {
            const checked = selectedIds.has(tag.id);
            return (
              <label
                key={tag.id}
                className={`tag-editor-chip${checked ? " checked" : ""}`}
              >
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={() => toggle(tag.id)}
                  aria-label={tag.name}
                />
                <span
                  className="tag-editor-swatch"
                  style={{ background: tag.color }}
                  aria-hidden="true"
                />
                <span className="tag-editor-name">{tag.name}</span>
              </label>
            );
          })}
        </div>
      )}
      <div className="tag-editor-actions">
        <button
          className="secondary"
          onClick={() => setShowCreateRow((v) => !v)}
          disabled={saving}
        >
          <Plus size={12} />
          新建标签
        </button>
        <button
          className="primary"
          onClick={() => void save()}
          disabled={saving || loading}
        >
          {saving ? "保存中…" : "保存"}
        </button>
      </div>
      {showCreateRow && (
        <div className="tag-editor-create-row">
          <input
            type="text"
            placeholder="标签名称"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            maxLength={20}
            aria-label="新标签名称"
          />
          <div className="tag-editor-create-tools">
            <input
              type="color"
              value={newColor}
              onChange={(e) => setNewColor(e.target.value.toUpperCase())}
              aria-label="新标签颜色"
              title="标签颜色"
            />
            <span className="tag-color-hex">{newColor}</span>
            <button
              className="primary"
              onClick={() => void createAndAttach()}
              disabled={saving || !newName.trim()}
            >
              创建并附加
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
