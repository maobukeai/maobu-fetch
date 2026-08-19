import { useEffect, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { api } from "../../api";
import type { Tag } from "../../types";
import { Modal } from "../common/Modal";
import { SettingsGroup } from "../common/FormComponents";
import { newTagId } from "../../formatters";

export function TagManagementPanel({
  notify,
}: {
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [tags, setTags] = useState<Tag[]>([]);
  const [loading, setLoading] = useState(true);
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState("#3B82F6");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingName, setEditingName] = useState("");
  const [editingColor, setEditingColor] = useState("#3B82F6");
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const load = async () => {
    setLoading(true);
    try {
      setTags(await api.tagList());
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, [notify]);

  const add = async () => {
    const name = newName.trim();
    if (!name) {
      notify("标签名称不能为空", "error");
      return;
    }
    if (!/^#[0-9A-Fa-f]{6}$/.test(newColor)) {
      notify("颜色格式必须为 #RRGGBB", "error");
      return;
    }
    try {
      await api.tagAdd({ id: newTagId(), name, color: newColor });
      setNewName("");
      setNewColor("#3B82F6");
      notify("标签已创建");
      await load();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const beginEdit = (tag: Tag) => {
    setEditingId(tag.id);
    setEditingName(tag.name);
    setEditingColor(tag.color);
  };

  const cancelEdit = () => {
    setEditingId(null);
    setEditingName("");
    setEditingColor("#3B82F6");
  };

  const saveEdit = async () => {
    if (!editingId) return;
    const name = editingName.trim();
    if (!name) {
      notify("标签名称不能为空", "error");
      return;
    }
    if (!/^#[0-9A-Fa-f]{6}$/.test(editingColor)) {
      notify("颜色格式必须为 #RRGGBB", "error");
      return;
    }
    try {
      await api.tagUpdate({ id: editingId, name, color: editingColor });
      cancelEdit();
      notify("标签已更新");
      await load();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const remove = async (id: string) => {
    try {
      await api.tagDelete(id);
      setConfirmDelete(null);
      notify("标签已删除");
      await load();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  return (
    <SettingsGroup title="标签管理">
      <p className="settings-note">
        标签用于对下载任务进行分类管理。删除标签只会解除关联，不会删除实际任务或文件。
      </p>
      <div className="settings-group-content">
        <div className="tag-management-add-row">
          <div className="tag-color-col">
            <input
              type="color"
              value={newColor}
              onChange={(e) => setNewColor(e.target.value.toUpperCase())}
              aria-label="新标签颜色"
              title="标签颜色"
            />
          </div>
          <input
            type="text"
            placeholder="新标签名称"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            maxLength={20}
            aria-label="新标签名称"
          />
          <span className="tag-color-hex">{newColor}</span>
          <button
            className="primary"
            onClick={() => void add()}
            disabled={!newName.trim()}
          >
            添加
          </button>
        </div>
        {loading ? (
          <div className="center-state">
            <LoaderCircle className="spin" />
          </div>
        ) : tags.length === 0 ? (
          <p className="muted">尚未创建任何标签。</p>
        ) : (
          <div className="tag-management-list">
            {tags.map((tag) => (
              <div key={tag.id} className="tag-management-row">
                {editingId === tag.id ? (
                  <>
                    <div className="tag-color-col">
                      <input
                        type="color"
                        value={editingColor}
                        onChange={(e) =>
                          setEditingColor(e.target.value.toUpperCase())
                        }
                        aria-label="编辑标签颜色"
                      />
                    </div>
                    <input
                      type="text"
                      value={editingName}
                      onChange={(e) => setEditingName(e.target.value)}
                      maxLength={20}
                      aria-label="编辑标签名称"
                    />
                    <button className="primary" onClick={() => void saveEdit()}>
                      保存
                    </button>
                    <button className="secondary-btn" onClick={() => cancelEdit()}>
                      取消
                    </button>
                  </>
                ) : (
                  <>
                    <div className="tag-color-col">
                      <span
                        className="tag-swatch"
                        style={{ background: tag.color }}
                        aria-hidden="true"
                      />
                    </div>
                    <span className="tag-name" title={tag.name}>
                      {tag.name}
                    </span>
                    <span className="tag-color-hex muted">{tag.color}</span>
                    <button
                      className="secondary-btn"
                      onClick={() => beginEdit(tag)}
                      title="编辑"
                    >
                      编辑
                    </button>
                    <button
                      className="danger-action"
                      onClick={() => setConfirmDelete(tag.id)}
                      title="删除"
                    >
                      删除
                    </button>
                  </>
                )}
              </div>
            ))}
          </div>
        )}
        {confirmDelete !== null && (
          <Modal
            title="删除标签"
            onClose={() => setConfirmDelete(null)}
            style={{ width: "360px" }}
          >
            <p
              style={{
                fontSize: "11.5px",
                color: "var(--muted)",
                margin: "0 0 16px",
                lineHeight: "1.5",
              }}
            >
              确定要删除此标签吗？这仅会移除所有任务与该标签的关联，不会删除下载文件。
            </p>
            <div className="dialog-actions">
              <button onClick={() => setConfirmDelete(null)}>取消</button>
              <button
                className="danger"
                onClick={() => void remove(confirmDelete)}
              >
                删除
              </button>
            </div>
          </Modal>
        )}
      </div>
    </SettingsGroup>
  );
}
