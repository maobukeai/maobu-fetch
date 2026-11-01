import { collectionFormatSelector, formatTrackDescription, selectDefaultVideoFormat, selectInitialVideoFormat, selectOriginalFileFormat } from "./media-format-selection";
import type { MediaFormat, MediaProbeResult } from "./types";
import { isDouyinMediaPage } from "./media-page-url";

const assert = {
  equal(actual: unknown, expected: unknown) {
    if (actual !== expected) throw new Error(`expected ${String(expected)}, got ${String(actual)}`);
  },
  throws(action: () => unknown, pattern: RegExp) {
    try { action(); } catch (error) {
      if (pattern.test(String(error))) return;
      throw error;
    }
    throw new Error(`expected error matching ${String(pattern)}`);
  },
};

const format = (id: string, height: number, video: boolean, audio: boolean, merge = false): MediaFormat => ({
  id, label: id, height, has_video: video, has_audio: audio, requires_ffmpeg: merge,
});
const formats = [format("silent-4k", 2160, true, false), format("direct-360", 360, true, true), format("merged-1080", 1080, true, true, true)];
assert.equal(selectDefaultVideoFormat(formats, true).id, "merged-1080");
assert.equal(selectDefaultVideoFormat(formats, false).id, "direct-360");
assert.equal(selectDefaultVideoFormat([format("merged", 1080, true, true, true), format("direct", 1080, true, true)], true).id, "direct");
assert.throws(() => selectDefaultVideoFormat([format("silent", 1080, true, false)], true), /音轨/);
assert.throws(() => selectDefaultVideoFormat([format("audio", 0, false, true)], true), /音轨/);
const unknown = { ...format("unknown-4k",2160,true,false),audio_unknown:true };
assert.equal(selectDefaultVideoFormat([unknown,format("audible-360",360,true,true)],true).id,"audible-360");
assert.throws(()=>selectDefaultVideoFormat([unknown],true),/未确认/);
assert.equal(formatTrackDescription(unknown).includes("无声"),false);
assert.throws(() => selectDefaultVideoFormat([format("merge", 1080, true, true, true)], false), /FFmpeg/);
assert.equal(collectionFormatSelector("720p", true), "bestvideo*[height<=720]+bestaudio/best[height<=720][vcodec!=none][acodec!=none]");
assert.equal(collectionFormatSelector("480p", false), "best[height<=480][vcodec!=none][acodec!=none]");
const highBitrateMerge = { ...format("merge-high-bitrate", 1080, true, true, true), bitrate_kbps: 3000 };
const lowerBitrateDirect = { ...format("direct-lower-bitrate", 1080, true, true), bitrate_kbps: 1000 };
assert.equal(selectDefaultVideoFormat([lowerBitrateDirect, highBitrateMerge], true).id, "merge-high-bitrate");
assert.equal(selectDefaultVideoFormat([lowerBitrateDirect, highBitrateMerge], false).id, "direct-lower-bitrate");
assert.equal(selectDefaultVideoFormat([{ ...lowerBitrateDirect, bitrate_kbps: 3000 }, highBitrateMerge], true).id, "direct-lower-bitrate");
const original: MediaProbeResult = {title:"actual-360p.mp4",extractor:"pikpak",formats:[{...format("original",0,false,false),original_file:true}],subtitles:[],drm:false,media_type:"video"};
assert.equal(selectOriginalFileFormat(original).id,"original");
assert.throws(() => selectDefaultVideoFormat(original.formats,true),/音轨/);
assert.throws(() => selectOriginalFileFormat({...original,extractor:"youtube"}),/网盘/);
const manual = selectInitialVideoFormat([unknown], false);
assert.equal(manual.formatId, "");
assert.equal(manual.notice, "已找到原片，来源未提供音轨信息，请选择原片后下载");
assert.equal(selectInitialVideoFormat([unknown], true).formatId, "");
assert.equal(selectInitialVideoFormat([unknown, format("audible", 360, true, true)], false).formatId, "audible");
assert.equal(selectInitialVideoFormat(formats, true).formatId, "merged-1080");
assert.throws(() => selectInitialVideoFormat([unknown, format("merge", 1080, true, true, true)], false), /FFmpeg/);
assert.throws(() => selectInitialVideoFormat([{ ...unknown, requires_ffmpeg: true }], false), /FFmpeg/);
assert.throws(() => selectInitialVideoFormat([], false), /未找到/);
assert.throws(() => selectInitialVideoFormat([format("silent", 1080, true, false)], false), /音轨/);
console.log("Passed 26 media default/collection/original-file/unknown-audio selector checks.");

const douyinPages = [
  "https://v.douyin.com/SyntheticShare/",
  "http://V.DOUYIN.COM/SyntheticShare/",
  "https://douyin.com/video/1234567890",
  "https://www.douyin.com/video/1234567890/",
  "https://www.douyin.com/note/1234567890?source=share",
  "https://m.douyin.com/video/1234567890",
  "https://www.douyin.com/?modal_id=1234567890",
  "https://www.douyin.com/user/author?modal_id=1234567890",
  "https://iesdouyin.com/share/video/1234567890/",
  "https://www.iesdouyin.com/share/note/1234567890/",
];
const ordinaryUrls = [
  "https://example.test/file.bin",
  "https://v3-web.douyinvod.com/video/tos/file.mp4",
  "https://vod.douyin.com/video/1234567890",
  "https://douyin.com.example.test/video/1234567890",
  "https://douyin.com@example.test/video/1234567890",
  "https://www.douyin.com/user/author",
  "https://www.douyin.com/?modal_id=invalid",
  "https://www.iesdouyin.com/share/user/1234567890",
  "https://example.test/file.bin?source=https://v.douyin.com/code",
  "ftp://www.douyin.com/video/1234567890",
  "not a URL",
];
for (const url of douyinPages) assert.equal(isDouyinMediaPage(url), true);
for (const url of ordinaryUrls) assert.equal(isDouyinMediaPage(url), false);
console.log(`Passed ${douyinPages.length + ordinaryUrls.length} Douyin media-page URL classification checks.`);
