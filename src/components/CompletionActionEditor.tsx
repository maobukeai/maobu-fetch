import { useMemo } from "react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import type { CompletionAction } from "../types";
import {
  completionActionKind,
  getCopyToData,
  getMoveToData,
  getRunCommandData,
  makeCompletionAction,
  TEMPLATE_VARIABLES,
} from "../types";

/**
 * 完成动作编辑器（Task 17.4）。
 *
 * 在新建任务、设置页和预设编辑器中复用。下拉选择动作类型，
 * 选中 RunCommand/CopyTo/MoveTo 时展开对应的参数输入框与模板变量提示。
 * 紧凑布局，遵循 AGENTS.md §4 的 Windows 11 效率型风格。
 */
export function CompletionActionEditor({
  value,
  onChange,
  allowRunFile,
  hidePowerOptions,
}: {
  value: CompletionAction;
  onChange: (action: CompletionAction) => void;
  /** 是否显示"运行文件"选项（仅桌面端单任务新建时为 true）。 */
  allowRunFile?: boolean;
  /** 是否隐藏关机/休眠选项（预设编辑器等场景可能不需要）。 */
  hidePowerOptions?: boolean;
}) {
  const kind = completionActionKind(value);
  const runCommandData = useMemo(() => getRunCommandData(value), [value]);
  const copyToData = useMemo(() => getCopyToData(value), [value]);
  const moveToData = useMemo(() => getMoveToData(value), [value]);

  const onKindChange = (newKind: string) => {
    // 切换类型时保留同类的已有数据；切到无数据变体时直接构造
    if (newKind === "run-command") {
      onChange(
        makeCompletionAction("run-command", {
          command: runCommandData.command,
          args: runCommandData.args,
          working_dir: runCommandData.working_dir,
        }),
      );
    } else if (newKind === "copy-to") {
      onChange(
        makeCompletionAction("copy-to", {
          target_directory: copyToData.target_directory,
          rename_pattern: copyToData.rename_pattern,
        }),
      );
    } else if (newKind === "move-to") {
      onChange(
        makeCompletionAction("move-to", {
          target_directory: moveToData.target_directory,
          rename_pattern: moveToData.rename_pattern,
        }),
      );
    } else {
      onChange(makeCompletionAction(newKind));
    }
  };

  const pickTargetDir = async (which: "copy-to" | "move-to") => {
    const selected = await pickPath({ directory: true, multiple: false, title: "选择目标目录" });
    if (typeof selected === "string" && selected) {
      if (which === "copy-to") {
        onChange(
          makeCompletionAction("copy-to", {
            target_directory: selected,
            rename_pattern: copyToData.rename_pattern,
          }),
        );
      } else {
        onChange(
          makeCompletionAction("move-to", {
            target_directory: selected,
            rename_pattern: moveToData.rename_pattern,
          }),
        );
      }
    }
  };

  return (
    <div className="completion-action-editor">
      <select value={kind} onChange={(e) => onKindChange(e.target.value)} style={{ width: "100%" }}>
        <option value="none">不执行操作</option>
        {allowRunFile && <option value="open-folder">打开文件夹</option>}
        {allowRunFile && <option value="run-file">运行文件（仅本次任务）</option>}
        {!hidePowerOptions && <option value="shutdown">关机（适合夜间下载）</option>}
        {!hidePowerOptions && <option value="hibernate">休眠</option>}
        {!hidePowerOptions && <option value="quit">退出猫步下载器</option>}
        <option value="run-command">运行自定义命令</option>
        <option value="copy-to">复制到指定目录</option>
        <option value="move-to">移动到指定目录</option>
      </select>

      {kind === "run-command" && (
        <div className="completion-action-fields">
          <label className="form-field">
            <span>命令路径</span>
            <input
              value={runCommandData.command}
              onChange={(e) =>
                onChange(
                  makeCompletionAction("run-command", {
                    command: e.target.value,
                    args: runCommandData.args,
                    working_dir: runCommandData.working_dir,
                  }),
                )
              }
              placeholder="例如：C:\\Windows\\System32\\cmd.exe"
            />
          </label>
          <label className="form-field">
            <span>参数（每行一个）</span>
            <textarea
              rows={3}
              value={runCommandData.args.join("\n")}
              onChange={(e) =>
                onChange(
                  makeCompletionAction("run-command", {
                    command: runCommandData.command,
                    args: e.target.value.split("\n"),
                    working_dir: runCommandData.working_dir,
                  }),
                )
              }
              placeholder={"/c echo $TITLE"}
            />
          </label>
          <label className="form-field">
            <span>工作目录（可选）</span>
            <input
              value={runCommandData.working_dir ?? ""}
              onChange={(e) =>
                onChange(
                  makeCompletionAction("run-command", {
                    command: runCommandData.command,
                    args: runCommandData.args,
                    working_dir: e.target.value || null,
                  }),
                )
              }
              placeholder="留空表示继承当前目录"
            />
          </label>
          <TemplateVariableHint />
        </div>
      )}

      {kind === "copy-to" && (
        <div className="completion-action-fields">
          <label className="form-field">
            <span>目标目录</span>
            <div className="dir-picker-row">
              <input
                value={copyToData.target_directory}
                onChange={(e) =>
                  onChange(
                    makeCompletionAction("copy-to", {
                      target_directory: e.target.value,
                      rename_pattern: copyToData.rename_pattern,
                    }),
                  )
                }
                placeholder="例如：D:\\Backup"
              />
              <button type="button" onClick={() => pickTargetDir("copy-to")}>
                浏览
              </button>
            </div>
          </label>
          <label className="form-field">
            <span>重命名模板（可选）</span>
            <input
              value={copyToData.rename_pattern ?? ""}
              onChange={(e) =>
                onChange(
                  makeCompletionAction("copy-to", {
                    target_directory: copyToData.target_directory,
                    rename_pattern: e.target.value || null,
                  }),
                )
              }
              placeholder="留空保留原文件名，例如：backup-$FILENAME"
            />
          </label>
          <TemplateVariableHint />
        </div>
      )}

      {kind === "move-to" && (
        <div className="completion-action-fields">
          <label className="form-field">
            <span>目标目录</span>
            <div className="dir-picker-row">
              <input
                value={moveToData.target_directory}
                onChange={(e) =>
                  onChange(
                    makeCompletionAction("move-to", {
                      target_directory: e.target.value,
                      rename_pattern: moveToData.rename_pattern,
                    }),
                  )
                }
                placeholder="例如：D:\\Archive"
              />
              <button type="button" onClick={() => pickTargetDir("move-to")}>
                浏览
              </button>
            </div>
          </label>
          <label className="form-field">
            <span>重命名模板（可选）</span>
            <input
              value={moveToData.rename_pattern ?? ""}
              onChange={(e) =>
                onChange(
                  makeCompletionAction("move-to", {
                    target_directory: moveToData.target_directory,
                    rename_pattern: e.target.value || null,
                  }),
                )
              }
              placeholder="留空保留原文件名，例如：archive-$FILENAME"
            />
          </label>
          <TemplateVariableHint />
        </div>
      )}
    </div>
  );
}

/** 模板变量提示行（紧凑布局）。 */
function TemplateVariableHint() {
  return (
    <div className="template-variable-hint" title="可在命令参数或重命名模板中使用以下变量">
      <span className="template-hint-label">模板变量：</span>
      {TEMPLATE_VARIABLES.map((v) => (
        <code key={v.token} title={v.desc}>
          {v.token}
        </code>
      ))}
    </div>
  );
}

/** 完成动作的中文标签（用于列表/详情展示）。 */
export function completionActionLabel(action: CompletionAction | null | undefined): string {
  const kind = completionActionKind(action);
  switch (kind) {
    case "none":
      return "无";
    case "open-folder":
      return "打开文件夹";
    case "run-file":
      return "运行文件";
    case "shutdown":
      return "关机";
    case "hibernate":
      return "休眠";
    case "quit":
      return "退出应用";
    case "run-command":
      return "运行命令";
    case "copy-to":
      return "复制到目录";
    case "move-to":
      return "移动到目录";
    default:
      return "无";
  }
}
