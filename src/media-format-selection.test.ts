import { collectionFormatSelector, formatTrackDescription, selectDefaultVideoFormat, selectOriginalFileFormat } from "./media-format-selection";
import type { MediaFormat, MediaProbeResult } from "./types";

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
console.log("Passed 17 media default/collection/original-file/unknown-audio selector checks.");
