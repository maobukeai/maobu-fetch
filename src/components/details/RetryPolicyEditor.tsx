import { useState } from "react";
import type { BackoffStrategy, RetryPolicy } from "../../types";
import { Select } from "../Select";
import { SettingRow } from "../common/FormComponents";

export const RETRY_PRESETS = {
  standard: {
    connection_timeout_secs: 60,
    task_timeout_secs: null,
    max_retries: 5,
    backoff: "exponential",
    initial_backoff_ms: 1000,
    max_backoff_ms: 60000,
  },
  quick: {
    connection_timeout_secs: 15,
    task_timeout_secs: 300,
    max_retries: 10,
    backoff: "fixed",
    initial_backoff_ms: 1000,
    max_backoff_ms: 1000,
  },
  persistent: {
    connection_timeout_secs: 30,
    task_timeout_secs: null,
    max_retries: 30,
    backoff: "exponential",
    initial_backoff_ms: 2000,
    max_backoff_ms: 300000,
  },
  none: {
    connection_timeout_secs: 30,
    task_timeout_secs: null,
    max_retries: 0,
    backoff: "fixed",
    initial_backoff_ms: 1000,
    max_backoff_ms: 1000,
  },
};

export const detectRetryPreset = (policy: RetryPolicy): string => {
  if (policy.max_retries === 0) return "none";
  for (const [key, preset] of Object.entries(RETRY_PRESETS)) {
    if (key === "none") continue;
    const p = preset as any;
    if (
      policy.connection_timeout_secs === p.connection_timeout_secs &&
      policy.task_timeout_secs === p.task_timeout_secs &&
      policy.max_retries === p.max_retries &&
      policy.backoff === p.backoff &&
      policy.initial_backoff_ms === p.initial_backoff_ms &&
      policy.max_backoff_ms === p.max_backoff_ms
    ) {
      return key;
    }
  }
  return "custom";
};

export function RetryPolicyEditor({
  value,
  onChange,
  disabled,
  compact,
}: {
  value: RetryPolicy;
  onChange: (value: RetryPolicy) => void;
  disabled?: boolean;
  compact?: boolean;
}) {
  const [localPreset, setLocalPreset] = useState<string>(() =>
    detectRetryPreset(value)
  );

  const update = <K extends keyof RetryPolicy>(key: K, val: RetryPolicy[K]) =>
    onChange({ ...value, [key]: val });
  const updateTaskTimeout = (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === "") {
      update("task_timeout_secs", null);
      return;
    }
    const parsed = Number(trimmed);
    if (Number.isFinite(parsed) && parsed > 0) {
      update("task_timeout_secs", Math.floor(parsed));
    }
  };

  const handlePresetChange = (presetKey: string) => {
    setLocalPreset(presetKey);
    if (presetKey !== "custom") {
      const selectedPreset =
        RETRY_PRESETS[presetKey as keyof typeof RETRY_PRESETS];
      onChange({
        connection_timeout_secs: selectedPreset.connection_timeout_secs,
        task_timeout_secs: selectedPreset.task_timeout_secs,
        max_retries: selectedPreset.max_retries,
        backoff: selectedPreset.backoff as BackoffStrategy,
        initial_backoff_ms: selectedPreset.initial_backoff_ms,
        max_backoff_ms: selectedPreset.max_backoff_ms,
      });
    }
  };

  const taskTimeoutValue =
    value.task_timeout_secs == null ? "" : String(value.task_timeout_secs);
  const isInputsDisabled = disabled || localPreset !== "custom";
  return (
    <div className="settings-group-content">
      <SettingRow label="重试预设">
        <Select
          value={localPreset}
          disabled={disabled}
          onChange={(val: any) => handlePresetChange(val as string)}
          options={[
            { value: "standard", label: "标准重试 (默认)" },
            { value: "quick", label: "快速重试 (针对不稳定 CDN)" },
            { value: "persistent", label: "顽固重试 (挂机且网络极差)" },
            { value: "none", label: "不自动重试" },
            { value: "custom", label: "自定义配置..." },
          ]}
          ariaLabel="重试预设"
        />
      </SettingRow>
      <div className="retry-policy-advanced-fields">
        <SettingRow label={compact ? "单连接超时(秒)" : "单连接超时（秒）"}>
          <input
            type="number"
            min="1"
            max="600"
            value={value.connection_timeout_secs}
            disabled={isInputsDisabled}
            onChange={(e) =>
              update("connection_timeout_secs", Math.max(1, +e.target.value || 1))
            }
          />
        </SettingRow>
        <SettingRow
          label={
            compact ? "任务总超时(秒)" : "任务总超时（秒，留空表示不限制）"
          }
        >
          <input
            type="number"
            min="0"
            placeholder={compact ? "不限" : "不限制"}
            value={taskTimeoutValue}
            disabled={isInputsDisabled}
            onChange={(e) => updateTaskTimeout(e.target.value)}
          />
        </SettingRow>
        <SettingRow
          label={
            compact ? "最大重试次数" : "最大重试次数（每条连接独立计数）"
          }
        >
          <input
            type="number"
            min="0"
            max="32"
            value={value.max_retries}
            disabled={isInputsDisabled}
            onChange={(e) =>
              update(
                "max_retries",
                Math.min(32, Math.max(0, +e.target.value || 0))
              )
            }
          />
        </SettingRow>
        <SettingRow label="退避策略">
          <div className="fluent-segmented-control settings-segmented">
            <button
              type="button"
              disabled={isInputsDisabled}
              className={value.backoff === "fixed" ? "active" : ""}
              onClick={() => update("backoff", "fixed" as BackoffStrategy)}
            >
              固定间隔
            </button>
            <button
              type="button"
              disabled={isInputsDisabled}
              className={value.backoff === "exponential" ? "active" : ""}
              onClick={() =>
                update("backoff", "exponential" as BackoffStrategy)
              }
            >
              指数退避
            </button>
          </div>
        </SettingRow>
        <SettingRow
          label={compact ? "初始退避(毫秒)" : "初始退避时长（毫秒）"}
        >
          <input
            type="number"
            min="1"
            value={value.initial_backoff_ms}
            disabled={isInputsDisabled}
            onChange={(e) =>
              update("initial_backoff_ms", Math.max(1, +e.target.value || 1))
            }
          />
        </SettingRow>
        <SettingRow
          label={compact ? "最大退避(毫秒)" : "最大退避时长（毫秒）"}
        >
          <input
            type="number"
            min={value.initial_backoff_ms}
            value={value.max_backoff_ms}
            disabled={isInputsDisabled}
            onChange={(e) =>
              update(
                "max_backoff_ms",
                Math.max(value.initial_backoff_ms, +e.target.value || value.initial_backoff_ms)
              )
            }
          />
        </SettingRow>
      </div>
    </div>
  );
}
