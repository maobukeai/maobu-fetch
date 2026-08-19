import { CheckSquare, Pause, Play, Trash2, X } from "lucide-react";
import { t, useLocale } from "../../i18n";

export function BulkActionBar({
  selectedCount,
  onStartAll,
  onPauseAll,
  onDeleteRecords,
  onDeleteFiles,
  onDeselectAll,
}: {
  selectedCount: number;
  onStartAll: () => void;
  onPauseAll: () => void;
  onDeleteRecords: () => void;
  onDeleteFiles: () => void;
  onDeselectAll: () => void;
}) {
  useLocale();
  if (selectedCount <= 1) return null;

  return (
    <div
      className="bulk-action-bar"
      role="toolbar"
      aria-label={t("toolbar.bulkActionTitle") || "批量操作"}
    >
      <div className="bulk-badge">
        <CheckSquare size={13} />
        <span>{t("dialogs.selectedItemsCount", { count: selectedCount }) || `已选择 ${selectedCount} 项`}</span>
      </div>

      <div className="bulk-separator" />

      <div className="bulk-buttons">
        <button
          type="button"
          className="bulk-btn"
          onClick={onStartAll}
          title={t("toolbar.startTask") || "全部开始"}
        >
          <Play size={12} />
          <span>{t("toolbar.bulkStart") || "开始"}</span>
        </button>

        <button
          type="button"
          className="bulk-btn"
          onClick={onPauseAll}
          title={t("toolbar.pauseTask") || "全部暂停"}
        >
          <Pause size={12} />
          <span>{t("toolbar.bulkPause") || "暂停"}</span>
        </button>

        <button
          type="button"
          className="bulk-btn danger"
          onClick={onDeleteRecords}
          title={t("dialogs.deleteRecordOnly") || "删除记录"}
        >
          <Trash2 size={12} />
          <span>{t("dialogs.deleteRecordOnly") || "删除记录"}</span>
        </button>

        <button
          type="button"
          className="bulk-btn danger"
          onClick={onDeleteFiles}
          title={t("dialogs.deleteRecordAndFile") || "删除记录与文件"}
        >
          <Trash2 size={12} />
          <span>{t("dialogs.deleteRecordAndFile") || "删除记录与文件"}</span>
        </button>
      </div>

      <button
        type="button"
        className="bulk-close-btn"
        onClick={onDeselectAll}
        title={t("common.close") || "取消选择"}
      >
        <X size={12} />
      </button>
    </div>
  );
}
