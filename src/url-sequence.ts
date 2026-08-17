/**
 * 批量序号 URL 展开（对标 IDM/FDM 的批量下载语法，2026-08-17）。
 *
 * 语法：URL 中以 `[start-end]` 或 `[start-end:step]` 标记序号区间，
 * 例如 `https://example.com/file[001-120].zip` 展开为 120 个任务。
 *
 * 规则：
 * - 仅对 http(s) URL 生效；`magnet:` 与本地 `.torrent` 行原样返回（BT 单任务语义）；
 * - 零填充按操作数最大宽度推断：`[001-120]` → 001…120，`[7-9]` → 7…9；
 * - 多个序号组做笛卡尔积，展开总数上限 {@link MAX_SEQUENCE_EXPANSION}；
 * - 起点大于终点或步长为 0 视为语法错误，返回可读错误而不是静默丢弃。
 *
 * 纯函数模块，不依赖 React，测试见 `url-sequence.test.ts`。
 */

/** 单行展开产物数量上限，防止误输入 `[1-999999]` 撑爆批量提交。 */
export const MAX_SEQUENCE_EXPANSION = 200;

const SEQUENCE_GROUP = /\[(\d+)-(\d+)(?::(\d+))?\]/g;

export interface SequenceResult {
  /** 展开后的 URL 列表；无序号组时为原行（单元素）。 */
  urls: string[];
  /** 展开失败时的可读中文错误（提交前由 UI 展示）。 */
  error?: string;
}

/** 判断一行是否为 BT 来源（magnet: URI 或本地 .torrent 路径），此类行不参与展开与 HTTP 提取。 */
export function isBtSourceLine(line: string): boolean {
  const trimmed = line.trim();
  if (/^magnet:/i.test(trimmed)) return true;
  if (/^https?:/i.test(trimmed)) return false;
  return /\.torrent$/i.test(trimmed);
}

/** 计算单个 `[start-end:step]` 组的取值序列，按操作数最大宽度零填充。 */
function groupValues(startText: string, endText: string, stepText: string | undefined): { values: string[]; error?: string } {
  const start = Number.parseInt(startText, 10);
  const end = Number.parseInt(endText, 10);
  const step = stepText !== undefined ? Number.parseInt(stepText, 10) : 1;
  if (!Number.isFinite(start) || !Number.isFinite(end) || !Number.isFinite(step)) {
    return { values: [], error: `序号区间 [${startText}-${endText}${stepText ? ":" + stepText : ""}] 格式无效` };
  }
  if (step <= 0) {
    return { values: [], error: `序号步长必须为正整数：[${startText}-${endText}:${stepText}]` };
  }
  if (start > end) {
    return { values: [], error: `序号区间起点不能大于终点：[${startText}-${endText}]` };
  }
  // 先算取值数量再构建数组：误输入超大区间（如 [1-99999999]）时必须立即报错，
  // 不能先物化上亿个字符串再由调用方的总数检查兜底（会造成 UI 冻结/OOM）。
  const count = Math.floor((end - start) / step) + 1;
  if (count > MAX_SEQUENCE_EXPANSION) {
    return {
      values: [],
      error: `序号区间 [${startText}-${endText}${stepText ? ":" + stepText : ""}] 取值数量（${count}）超过上限 ${MAX_SEQUENCE_EXPANSION}，请缩小范围或分批添加`,
    };
  }
  const width = Math.max(startText.length, endText.length);
  const values: string[] = [];
  for (let value = start; value <= end; value += step) {
    values.push(String(value).padStart(width, "0"));
  }
  return { values };
}

/**
 * 展开单行 URL 中的全部序号组（多个组做笛卡尔积）。
 * 非 http(s) 行（magnet/种子路径/普通文本）原样返回，由调用方决定处理方式。
 */
export function expandSequenceUrls(line: string): SequenceResult {
  const trimmed = line.trim();
  if (!trimmed) return { urls: [] };
  if (!/^https?:/i.test(trimmed)) return { urls: [trimmed] };

  const matches = [...trimmed.matchAll(SEQUENCE_GROUP)];
  if (matches.length === 0) return { urls: [trimmed] };

  // 每组先求值列表；任一组失败立即返回错误。
  const groups: { values: string[]; literal: string }[] = [];
  let total = 1;
  for (const match of matches) {
    const { values, error } = groupValues(match[1], match[2], match[3]);
    if (error) return { urls: [], error };
    total *= values.length;
    if (total > MAX_SEQUENCE_EXPANSION) {
      return { urls: [], error: `序号展开数量（${total}）超过上限 ${MAX_SEQUENCE_EXPANSION}，请缩小范围或分批添加` };
    }
    groups.push({ values, literal: match[0] });
  }

  // 笛卡尔积：逐组替换字面量。
  let results: string[] = [trimmed];
  for (const group of groups) {
    const next: string[] = [];
    for (const partial of results) {
      for (const value of group.values) {
        next.push(partial.replace(group.literal, value));
      }
    }
    results = next;
  }
  return { urls: results };
}
