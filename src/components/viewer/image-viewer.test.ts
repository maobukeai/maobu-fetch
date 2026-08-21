import { isImageFile } from "../common/TaskRow";

function assert(condition: boolean, msg: string) {
  if (!condition) {
    throw new Error(`断言失败: ${msg}`);
  }
}

console.log("▶ 测试 isImageFile 图像格式识别...");

// 常见图片格式
assert(isImageFile("photo.png"), "photo.png 应该被识别为图片");
assert(isImageFile("avatar.JPG"), "avatar.JPG (大写) 应该被识别为图片");
assert(isImageFile("banner.jpeg"), "banner.jpeg 应该被识别为图片");
assert(isImageFile("comic_01.webp"), "comic_01.webp 应该被识别为图片");
assert(isImageFile("meme.gif"), "meme.gif 应该被识别为图片");
assert(isImageFile("icon.bmp"), "icon.bmp 应该被识别为图片");
assert(isImageFile("vector.svg"), "vector.svg 应该被识别为图片");
assert(isImageFile("logo.ico"), "logo.ico 应该被识别为图片");
assert(isImageFile("photo.avif"), "photo.avif 应该被识别为图片");
assert(isImageFile("scan.tiff"), "scan.tiff 应该被识别为图片");
assert(isImageFile("scan.tif"), "scan.tif 应该被识别为图片");
assert(isImageFile("camera.jfif"), "camera.jfif 应该被识别为图片");

// 非图片格式
assert(!isImageFile("video.mp4"), "video.mp4 不应被识别为图片");
assert(!isImageFile("archive.zip"), "archive.zip 不应被识别为图片");
assert(!isImageFile("document.pdf"), "document.pdf 不应被识别为图片");
assert(!isImageFile("subtitle.srt"), "subtitle.srt 不应被识别为图片");
assert(!isImageFile("music.mp3"), "music.mp3 不应被识别为图片");

console.log("✔ 全部 17 项 isImageFile 格式测试通过！");

import { calculateOptimalViewerSize } from "./image-viewer-utils";

console.log("▶ 测试 calculateOptimalViewerSize 图片自适应比例与尺寸计算...");

// 1. 常见 16:9 横屏高清图 (1920x1080)
const land169 = calculateOptimalViewerSize(1920, 1080);
assert(land169.width === 600, `16:9 宽度应为 600，实际: ${land169.width}`);
assert(Math.abs(land169.height - 338) <= 2, `16:9 高度应约为 338，实际: ${land169.height}`);

// 2. 常见 9:16 竖屏手机图 (1080x1920)
const port916 = calculateOptimalViewerSize(1080, 1920);
assert(port916.height <= 620 && port916.height >= 500, `9:16 高度应适中，实际: ${port916.height}`);
assert(port916.width >= 340 && port916.width <= 400, `9:16 宽度应约为 340~400，实际: ${port916.width}`);

// 3. 正方形头像 (800x800)
const square = calculateOptimalViewerSize(800, 800);
assert(square.width === square.height, `正方形长宽应相等，实际: ${square.width}x${square.height}`);
assert(square.width === 600, `正方形尺寸应为 600，实际: ${square.width}`);

console.log("✔ 全部 calculateOptimalViewerSize 尺寸与比例测试通过！");
