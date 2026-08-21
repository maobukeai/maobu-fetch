/**
 * 猫步看图器 (Maobu Image Viewer) 尺寸计算与工具函数
 */

export interface ViewerOptimalSize {
  width: number;
  height: number;
}

/**
 * 根据图片真实分辨率计算自适应的初始窗口尺寸：
 * - 保持与图片真实宽高比严格一致；
 * - 默认展示为适中的半尺寸浮窗（横屏基准宽约 600px，竖屏基准高约 560px），避免放大全屏过大；
 * - 保证最小宽度有充足空间完整展示全部功能按钮。
 */
export function calculateOptimalViewerSize(
  naturalW: number,
  naturalH: number
): ViewerOptimalSize {
  if (!naturalW || !naturalH) return { width: 600, height: 440 };
  const aspect = naturalW / naturalH;

  let targetW: number;
  let targetH: number;

  if (aspect >= 1) {
    // 横屏图片：基准宽度约为 600px（约为屏幕或大窗口的一半大小左右）
    targetW = Math.min(naturalW, 600);
    targetW = Math.max(420, targetW);
    targetH = Math.round(targetW / aspect);
  } else {
    // 竖屏图片：基准高度约为 560px
    targetH = Math.min(naturalH, 560);
    targetH = Math.max(360, targetH);
    targetW = Math.round(targetH * aspect);
    targetW = Math.max(380, targetW);
  }

  return {
    width: Math.min(800, Math.max(380, targetW)),
    height: Math.min(640, Math.max(280, targetH)),
  };
}
