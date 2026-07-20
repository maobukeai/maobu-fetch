/**
 * Task 33: 简易 i18n 框架（不引入 i18next 等大型库，AGENTS.md §8）。
 *
 * 设计目标：
 * - 单一 `t(key, params?)` 函数从 locale JSON 读取键值，支持 `{name}` 占位符替换
 * - 维护 `currentLocale` 全局状态，提供 `setLocale(locale)` / `getLocale()`
 * - 默认 `zh-CN`，未识别的 locale 回退到 `zh-CN`
 * - 提供 `useLocale()` hook 让 React 组件订阅语言切换并触发重渲染
 *
 * 数据源：
 * - `src/locales/zh-CN.json`：简体中文文案
 * - `src/locales/en.json`：英文文案（用户硬性要求：完全无中文字符）
 *
 * 兼容性：
 * - 缺失 key 时返回 key 本身（前端不抛错，方便增量迁移）
 * - 占位符缺失时保留原 `{name}` 文本
 * - locale JSON 在构建期通过 Vite 静态 import 打包（resolveJsonModule 已启用）
 */
import { useEffect, useSyncExternalStore } from "react";
import zhCN from "./locales/zh-CN.json";
import en from "./locales/en.json";

/** 支持的语言列表。新增语言需同步更新此类型与 `messages` 字典。 */
export type Locale = "zh-CN" | "en";

/** locale 字典：键为 locale 标识，值为对应 JSON 文件解析后的对象。 */
const messages: Record<Locale, Record<string, unknown>> = {
  "zh-CN": zhCN as Record<string, unknown>,
  en: en as Record<string, unknown>,
};

/** 默认语言。新增语言不得改变默认值，避免向后兼容问题。 */
const DEFAULT_LOCALE: Locale = "zh-CN";

/** 当前生效的 locale。模块级单例，跨组件共享。 */
let currentLocale: Locale = DEFAULT_LOCALE;

/**
 * 订阅者回调列表。语言切换时通过 `emitChange()` 通知所有订阅者。
 * 使用数组而非 Set 以兼容老版本运行时；订阅者数量极少（每个 useLocale 一次），性能可接受。
 */
const subscribers: Array<() => void> = [];

/** 通知所有订阅者当前 locale 已变化。 */
const emitChange = () => {
  for (const subscriber of subscribers) {
    try {
      subscriber();
    } catch {
      // 单个订阅者抛错不影响其他订阅者；React 错误边界会接管 UI 报错。
    }
  }
};

/** 订阅 locale 变化。返回取消订阅函数。 */
const subscribe = (callback: () => void): (() => void) => {
  subscribers.push(callback);
  return () => {
    const index = subscribers.indexOf(callback);
    if (index >= 0) subscribers.splice(index, 1);
  };
};

/** 获取当前 locale（用于 useSyncExternalStore 的 getSnapshot）。 */
const getLocaleSnapshot = (): Locale => currentLocale;

/**
 * 设置当前 locale。
 *
 * - 未识别的 locale 静默回退到默认 locale（zh-CN），不抛错
 * - locale 相同时不触发通知，避免无意义重渲染
 * - 调用此函数不会自动持久化；持久化由调用方负责（如 AppSettings.language -> setLocale）
 */
export function setLocale(locale: string): void {
  const next = isSupportedLocale(locale) ? locale : DEFAULT_LOCALE;
  if (next === currentLocale) return;
  currentLocale = next;
  emitChange();
}

/** 获取当前 locale。 */
export function getLocale(): Locale {
  return currentLocale;
}

/** 判断字符串是否为受支持的 locale。 */
function isSupportedLocale(locale: string): locale is Locale {
  return locale === "zh-CN" || locale === "en";
}

/**
 * 在 locale JSON 中按点分路径查找 key。
 *
 * 例如 `t("common.ok")` 会在 `messages[locale]` 中查找 `common -> ok`。
 * - 任意一层不存在或类型不是对象/字符串时返回 `null`
 * - 最终值不是字符串时返回 `null`（不支持数字/布尔值翻译）
 */
function lookup(locale: Locale, key: string): string | null {
  const parts = key.split(".");
  let current: unknown = messages[locale];
  for (const part of parts) {
    if (current == null || typeof current !== "object") return null;
    current = (current as Record<string, unknown>)[part];
  }
  return typeof current === "string" ? current : null;
}

/**
 * 替换 `{name}` 占位符。
 *
 * - 占位符名称严格匹配 params 键名（区分大小写）
 * - params 中缺失的占位符保留原 `{name}` 文本
 * - params 值会被 `String(...)` 转换为字符串，支持 number / boolean / string
 */
function interpolate(template: string, params?: Record<string, string | number | boolean>): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    if (Object.prototype.hasOwnProperty.call(params, name)) {
      return String(params[name]);
    }
    return match;
  });
}

/**
 * 翻译 key 到当前 locale 的字符串。
 *
 * - `key` 为点分路径，例如 `"common.ok"` / `"nav.allTasks"`
 * - `params` 为可选的占位符替换字典，例如 `{ count: 3 }` 会替换 `{count}`
 * - 缺失 key 时返回 key 本身（不抛错，方便增量迁移）
 * - 当前 locale 缺失 key 但默认 locale (zh-CN) 存在时，回退到默认 locale
 */
export function t(key: string, params?: Record<string, string | number | boolean>): string {
  const value = lookup(currentLocale, key) ?? lookup(DEFAULT_LOCALE, key);
  if (value == null) return key;
  return interpolate(value, params);
}

/**
 * React hook：订阅 locale 变化并返回当前 locale。
 *
 * 使用 `useSyncExternalStore` 保证并发安全（React 18+）。
 * 调用 `setLocale` 后所有使用 `useLocale()` 的组件会自动重渲染。
 */
export function useLocale(): Locale {
  return useSyncExternalStore(subscribe, getLocaleSnapshot, getLocaleSnapshot);
}

/**
 * React hook：在组件挂载时设置 locale，并在卸载时恢复原 locale。
 *
 * 主要用于测试或临时切换语言场景；正常 UI 中应通过 setLocale + AppSettings 持久化。
 *
 * 注意：此 hook 使用 useEffect + useState 兼容旧版本 React，
 * 而非 useSyncExternalStore，以便在测试中可控地切换语言。
 */
export function useForceLocale(locale: Locale): void {
  useEffect(() => {
    const previous = currentLocale;
    setLocale(locale);
    return () => {
      if (previous !== locale) setLocale(previous);
    };
  }, [locale]);
}

/**
 * 检查 locale JSON 中是否包含中文字符。
 *
 * 用户硬性要求：英文 locale 下完全无中文字符。
 * 此函数用于测试中扫描 en.json，确保没有任何 CJK 字符。
 *
 * 返回找到的中文字符数组；空数组表示无中文。
 */
export function findChineseCharactersInLocale(locale: Locale): string[] {
  const json = messages[locale];
  const serialized = JSON.stringify(json);
  const chineseRegex = /[\u4e00-\u9fa5]/g;
  const matches = serialized.match(chineseRegex);
  return matches ? Array.from(new Set(matches)) : [];
}

// 兼容性导出：让 React 组件可以在不使用 hook 的场景下访问当前 locale（用于事件回调等）。
// 这些函数与 setLocale / getLocale 等价，仅作为别名存在以方便迁移。
export const getCurrentLocale = getLocale;
export const changeLocale = setLocale;
