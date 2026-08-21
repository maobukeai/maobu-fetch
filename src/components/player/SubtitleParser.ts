/**
 * 轻量 SRT / WebVTT 字幕解析器。
 * 纯 TypeScript 实现，零第三方库依赖。
 */

export interface SubtitleCue {
  id?: string;
  start: number; // 秒 (含小数)
  end: number;   // 秒 (含小数)
  text: string;
}

/**
 * 将时间戳字符串（00:01:23,456 或 00:01:23.456）解析为秒数。
 */
export function parseTimestamp(timeStr: string): number {
  const clean = timeStr.trim().replace(",", ".");
  const parts = clean.split(":");
  if (parts.length === 3) {
    const hours = parseFloat(parts[0]) || 0;
    const minutes = parseFloat(parts[1]) || 0;
    const seconds = parseFloat(parts[2]) || 0;
    return hours * 3600 + minutes * 60 + seconds;
  } else if (parts.length === 2) {
    const minutes = parseFloat(parts[0]) || 0;
    const seconds = parseFloat(parts[1]) || 0;
    return minutes * 60 + seconds;
  }
  return 0;
}

/**
 * 解析 SRT 或 WebVTT 格式字幕文本为 Cue 数组。
 */
export function parseSubtitles(content: string): SubtitleCue[] {
  const cues: SubtitleCue[] = [];
  if (!content || !content.trim()) return cues;

  // 标准化换行符
  const normalized = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const blocks = normalized.split(/\n\n+/);

  for (const block of blocks) {
    const lines = block.trim().split("\n");
    if (lines.length === 0) continue;

    let timeLineIndex = -1;
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].includes("-->")) {
        timeLineIndex = i;
        break;
      }
    }

    if (timeLineIndex === -1) continue;

    const timeLine = lines[timeLineIndex];
    const [startRaw, endRawWithSettings] = timeLine.split("-->");
    if (!startRaw || !endRawWithSettings) continue;

    const start = parseTimestamp(startRaw.trim());
    // WebVTT 后面可能跟有 position:50% line:0 等设置，取空格前第一个 token
    const endRaw = endRawWithSettings.trim().split(/\s+/)[0];
    const end = parseTimestamp(endRaw);

    const textLines = lines.slice(timeLineIndex + 1);
    // 清理 HTML 标签如 <b>, <i>, <font>
    const text = textLines
      .join("\n")
      .replace(/<[^>]+>/g, "")
      .trim();

    if (text && end > start) {
      cues.push({ start, end, text });
    }
  }

  // 按时间升序排序
  cues.sort((a, b) => a.start - b.start);
  return cues;
}

/**
 * 解析 ASS / SSA 格式字幕文本为 Cue 数组。
 */
export function parseAssSubtitles(content: string): SubtitleCue[] {
  const cues: SubtitleCue[] = [];
  if (!content || !content.trim()) return cues;

  const normalized = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const lines = normalized.split("\n");

  let formatFields: string[] = [];

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line) continue;

    if (line.toLowerCase().startsWith("format:")) {
      formatFields = line.substring(7).split(",").map((f) => f.trim().toLowerCase());
      continue;
    }

    if (line.toLowerCase().startsWith("dialogue:")) {
      const rest = line.substring(9).trim();
      const parts = rest.split(",");
      if (parts.length < 3) continue;

      let start = 0;
      let end = 0;
      let text = "";

      if (formatFields.length >= 3) {
        const startIdx = formatFields.indexOf("start");
        const endIdx = formatFields.indexOf("end");
        const textIdx = formatFields.indexOf("text");

        if (startIdx !== -1 && parts[startIdx]) {
          start = parseTimestamp(parts[startIdx].trim());
        }
        if (endIdx !== -1 && parts[endIdx]) {
          end = parseTimestamp(parts[endIdx].trim());
        }
        if (textIdx !== -1 && parts.length > textIdx) {
          text = parts.slice(textIdx).join(",");
        }
      } else {
        // 默认 ASS 布局：Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
        start = parseTimestamp(parts[1]?.trim() || "");
        end = parseTimestamp(parts[2]?.trim() || "");
        text = parts.slice(9).join(",");
      }

      // 清理 ASS 样式标签如 {\pos(100,200)}, {\an8}, {\c&Hffffff&} 等
      let cleanText = text
        .replace(/\{[^}]*\}/g, "")
        .replace(/\\N/g, "\n")
        .replace(/\\n/g, "\n")
        .replace(/\\h/g, " ")
        .trim();

      if (cleanText && end > start) {
        cues.push({ start, end, text: cleanText });
      }
    }
  }

  cues.sort((a, b) => a.start - b.start);
  return cues;
}

/**
 * 统一字幕解析门面：智能根据扩展名或内容特征解析 SRT / WebVTT / ASS / SSA 字幕。
 */
export function parseAnySubtitles(content: string, ext?: string): SubtitleCue[] {
  if (!content) return [];
  const extLower = (ext || "").toLowerCase().replace(/^\./, "");
  if (extLower === "ass" || extLower === "ssa" || content.includes("[Events]") || content.includes("Dialogue:")) {
    return parseAssSubtitles(content);
  }
  return parseSubtitles(content);
}

/**
 * 根据当前时间戳（秒）获取当前应该展示的字幕文本（含时轴偏移 offset）。
 */
export function getActiveCueText(cues: SubtitleCue[], currentTime: number, offset = 0): string {
  const effectiveTime = currentTime + offset;
  for (const cue of cues) {
    if (effectiveTime >= cue.start && effectiveTime <= cue.end) {
      return cue.text;
    }
  }
  return "";
}
