import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ChevronLeft,
  ChevronRight,
  Copy,
  FlipHorizontal,
  FolderOpen,
  Grid,
  Image as ImageIcon,
  Info,
  Maximize2,
  Minimize2,
  Minus,
  MoreHorizontal,
  Pin,
  PinOff,
  Play,
  Pause,
  Plus,
  RotateCw,
  Sliders,
  Sun,
  X,
} from "lucide-react";
import { api, isDesktop } from "../../api";
import type { ImageFileInfo, ImageItem } from "../../types";
import { formatBytes, formatDate } from "../../formatters";
import { calculateOptimalViewerSize } from "./image-viewer-utils";
import "./ImageViewer.css";

interface ImageViewerProps {
  initialFile?: string;
  initialTitle?: string;
}

type FilterMode = "none" | "contrast" | "grayscale" | "invert";

export function ImageViewerView({ initialFile, initialTitle }: ImageViewerProps) {
  const [currentPath, setCurrentPath] = useState<string>(() => {
    if (initialFile) return initialFile;
    const params = new URLSearchParams(window.location.search);
    return params.get("file") || "";
  });

  const [imageTitle, setImageTitle] = useState<string>(() => {
    if (initialTitle) return initialTitle;
    const params = new URLSearchParams(window.location.search);
    return params.get("title") || "";
  });

  // 同目录图集列表
  const [gallery, setGallery] = useState<ImageItem[]>([]);
  const [currentIndex, setCurrentIndex] = useState<number>(0);

  // 变换状态：缩放、位移、旋转、翻转
  const [scale, setScale] = useState<number>(1);
  const [position, setPosition] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const [rotation, setRotation] = useState<number>(0);
  const [flipH, setFlipH] = useState<boolean>(false);
  const [filterMode, setFilterMode] = useState<FilterMode>("none");
  const [isCheckerboard, setIsCheckerboard] = useState<boolean>(false);

  // 拖拽平移与单击判定
  const [isDragging, setIsDragging] = useState<boolean>(false);
  const dragStartRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const posStartRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const clickStartRef = useRef<{ time: number; x: number; y: number }>({ time: 0, x: 0, y: 0 });

  // 窗口与交互状态
  const [isAlwaysOnTop, setIsAlwaysOnTop] = useState<boolean>(false);
  const [isFullscreen, setIsFullscreen] = useState<boolean>(false);
  const [showControls, setShowControls] = useState<boolean>(true);
  const [isManualHidden, setIsManualHidden] = useState<boolean>(false);
  const [showMoreMenu, setShowMoreMenu] = useState<boolean>(false);
  const [showThumbnailStrip, setShowThumbnailStrip] = useState<boolean>(false);
  const [showInfoModal, setShowInfoModal] = useState<boolean>(false);
  const [isPlayingSlideshow, setIsPlayingSlideshow] = useState<boolean>(false);
  const [imageInfo, setImageInfo] = useState<ImageFileInfo | null>(null);
  const [naturalSize, setNaturalSize] = useState<{ width: number; height: number } | null>(null);
  const [toastMessage, setToastMessage] = useState<string>("");
  const [loadFallback, setLoadFallback] = useState<boolean>(false);

  const stageRef = useRef<HTMLDivElement | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const slideshowTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const isInitialLoadRef = useRef<boolean>(true);

  const triggerToast = useCallback((msg: string) => {
    setToastMessage(msg);
    if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    toastTimerRef.current = setTimeout(() => setToastMessage(""), 2200);
  }, []);

  // 重置/适应窗口
  const resetFit = useCallback(() => {
    setScale(1);
    setPosition({ x: 0, y: 0 });
  }, []);

  // 滤镜循环切换
  const cycleFilter = useCallback(() => {
    setFilterMode((prev) => {
      if (prev === "none") {
        triggerToast("滤镜: 高对比度增强");
        return "contrast";
      }
      if (prev === "contrast") {
        triggerToast("滤镜: 经典黑白");
        return "grayscale";
      }
      if (prev === "grayscale") {
        triggerToast("滤镜: 反色底片");
        return "invert";
      }
      triggerToast("滤镜: 原始色彩");
      return "none";
    });
  }, [triggerToast]);

  // 加载图片并扫描同目录图集
  const loadImage = useCallback(
    async (filePath: string, title?: string) => {
      if (!filePath) return;
      setCurrentPath(filePath);
      setLoadFallback(false);
      const name = filePath.split(/[\\/]/).pop() || filePath;
      setImageTitle(title || name);
      setRotation(0);
      setFlipH(false);
      setPosition({ x: 0, y: 0 });
      setNaturalSize(null);

      // 加载物理元信息
      try {
        const info = await api.imageViewerGetInfo(filePath);
        setImageInfo(info);
      } catch {
        setImageInfo(null);
      }

      // 扫描图集
      try {
        const items = await api.imageViewerGetFolderImages(filePath);
        setGallery(items);
        const idx = items.findIndex((it) => it.path === filePath);
        setCurrentIndex(idx >= 0 ? idx : 0);
      } catch (e) {
        console.error("扫描同目录图片失败:", e);
      }
    },
    []
  );

  // 初始化加载
  useEffect(() => {
    if (currentPath) {
      void loadImage(currentPath, imageTitle);
    } else {
      void api.imageViewerGetCurrentFile().then((res) => {
        if (res && res[0]) {
          void loadImage(res[0], res[1] || undefined);
        }
      });
    }

    // 监听切图事件
    const unlisten = listen<{ file: string; title?: string }>(
      "image-viewer-load-file",
      (event) => {
        if (event.payload?.file) {
          void loadImage(event.payload.file, event.payload.title);
        }
      }
    );

    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [currentPath, imageTitle, loadImage]);

  // 切上一张
  const handlePrev = useCallback(() => {
    if (gallery.length <= 1) return;
    const nextIdx = (currentIndex - 1 + gallery.length) % gallery.length;
    const target = gallery[nextIdx];
    if (target) {
      void loadImage(target.path, target.name);
    }
  }, [gallery, currentIndex, loadImage]);

  // 切下一张
  const handleNext = useCallback(() => {
    if (gallery.length <= 1) return;
    const nextIdx = (currentIndex + 1) % gallery.length;
    const target = gallery[nextIdx];
    if (target) {
      void loadImage(target.path, target.name);
    }
  }, [gallery, currentIndex, loadImage]);

  // 幻灯片自动轮播
  useEffect(() => {
    if (!isPlayingSlideshow) {
      if (slideshowTimerRef.current) clearInterval(slideshowTimerRef.current);
      return;
    }
    slideshowTimerRef.current = setInterval(() => {
      handleNext();
    }, 3000);

    return () => {
      if (slideshowTimerRef.current) clearInterval(slideshowTimerRef.current);
    };
  }, [isPlayingSlideshow, handleNext]);

  // 隐藏控制栏计时器（鼠标静止 1.5 秒后自动渐隐）
  const resetHideControlsTimer = useCallback(() => {
    if (isManualHidden) return;
    setShowControls(true);
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    hideTimerRef.current = setTimeout(() => {
      if (!showThumbnailStrip && !showInfoModal && !showMoreMenu) {
        setShowControls(false);
      }
    }, 1500);
  }, [isManualHidden, showThumbnailStrip, showInfoModal, showMoreMenu]);

  // 单击图片区域切换控制栏显示/隐藏
  const toggleControls = useCallback(() => {
    setIsManualHidden((prev) => {
      const nextManual = !prev;
      if (nextManual) {
        setShowControls(false);
        setShowMoreMenu(false);
        setShowThumbnailStrip(false);
        setShowInfoModal(false);
        if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
      } else {
        setShowControls(true);
        if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
        hideTimerRef.current = setTimeout(() => {
          if (!showThumbnailStrip && !showInfoModal && !showMoreMenu) {
            setShowControls(false);
          }
        }, 1500);
      }
      return nextManual;
    });
  }, [showThumbnailStrip, showInfoModal, showMoreMenu]);

  // 滚轮缩放 (以指针为焦点)
  const handleWheel = (e: ReactWheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY < 0 ? 1.15 : 0.87;
    setScale((prev) => {
      const next = Math.min(10, Math.max(0.1, parseFloat((prev * delta).toFixed(2))));
      return next;
    });
    resetHideControlsTimer();
  };

  // 抓手拖拽与单击检测
  const handleMouseDown = (e: ReactMouseEvent) => {
    if (e.button !== 0) return;
    setIsDragging(true);
    dragStartRef.current = { x: e.clientX, y: e.clientY };
    posStartRef.current = { ...position };
    clickStartRef.current = { time: Date.now(), x: e.clientX, y: e.clientY };
  };

  const handleMouseMove = (e: ReactMouseEvent) => {
    // 若用户主动处于纯图隐藏模式，仅当鼠标移到顶部或底部边缘时才恢复
    if (isManualHidden) {
      if (e.clientY < 36 || e.clientY > window.innerHeight - 42) {
        setIsManualHidden(false);
        setShowControls(true);
      } else {
        if (!isDragging) return;
        const dx = e.clientX - dragStartRef.current.x;
        const dy = e.clientY - dragStartRef.current.y;
        setPosition({
          x: posStartRef.current.x + dx,
          y: posStartRef.current.y + dy,
        });
        return;
      }
    }

    resetHideControlsTimer();
    if (!isDragging) return;
    const dx = e.clientX - dragStartRef.current.x;
    const dy = e.clientY - dragStartRef.current.y;
    setPosition({
      x: posStartRef.current.x + dx,
      y: posStartRef.current.y + dy,
    });
  };

  const handleMouseUp = (e: ReactMouseEvent) => {
    setIsDragging(false);
    // 判断是否为纯单击（移动 < 5px 且持续 < 250ms）
    const duration = Date.now() - clickStartRef.current.time;
    const dx = Math.abs(e.clientX - clickStartRef.current.x);
    const dy = Math.abs(e.clientY - clickStartRef.current.y);
    if (duration < 250 && dx < 5 && dy < 5) {
      toggleControls();
    }
  };

  // 复制图片到剪贴板
  const copyImage = async () => {
    if (!currentPath) return;
    try {
      if (imgRef.current) {
        const canvas = document.createElement("canvas");
        canvas.width = imgRef.current.naturalWidth;
        canvas.height = imgRef.current.naturalHeight;
        const ctx = canvas.getContext("2d");
        if (ctx) {
          ctx.drawImage(imgRef.current, 0, 0);
          canvas.toBlob(async (blob) => {
            if (blob) {
              await navigator.clipboard.write([
                new ClipboardItem({ "image/png": blob }),
              ]);
              triggerToast("已复制图片到剪贴板");
            }
          }, "image/png");
          return;
        }
      }
      await navigator.clipboard.writeText(currentPath);
      triggerToast("已复制图片路径");
    } catch {
      await navigator.clipboard.writeText(currentPath);
      triggerToast("已复制图片绝对路径");
    }
  };

  // 复制路径
  const copyPath = async () => {
    if (!currentPath) return;
    await navigator.clipboard.writeText(currentPath);
    triggerToast("已复制文件路径");
  };

  // 窗口控制
  const handleMinimize = () => void api.imageViewerWindowMinimize();
  const handleToggleMaximize = () => void api.imageViewerWindowToggleMaximize();
  const handleClose = () => void api.imageViewerWindowClose();
  const handleToggleFullscreen = async () => {
    const fs = await api.imageViewerWindowToggleFullscreen();
    setIsFullscreen(fs);
  };
  const handleToggleAlwaysOnTop = async () => {
    const top = await api.imageViewerWindowToggleAlwaysOnTop();
    setIsAlwaysOnTop(top);
    triggerToast(top ? "已开启窗口置顶" : "已取消置顶");
  };

  // 键盘快捷键监听
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;

      switch (e.key) {
        case "ArrowLeft":
        case "a":
        case "A":
        case "PageUp":
          e.preventDefault();
          handlePrev();
          break;
        case "ArrowRight":
        case "d":
        case "D":
        case "PageDown":
        case " ":
          e.preventDefault();
          handleNext();
          break;
        case "p":
        case "P":
          e.preventDefault();
          void handleToggleAlwaysOnTop();
          break;
        case "r":
        case "R":
          e.preventDefault();
          setRotation((prev) => (prev + 90) % 360);
          break;
        case "h":
        case "H":
          e.preventDefault();
          setFlipH((prev) => !prev);
          break;
        case "b":
        case "B":
          e.preventDefault();
          setIsCheckerboard((prev) => !prev);
          triggerToast(!isCheckerboard ? "已开启透明棋盘格" : "已恢复暗夜背景");
          break;
        case "1":
          e.preventDefault();
          setScale(1);
          setPosition({ x: 0, y: 0 });
          triggerToast("100% 原始尺寸");
          break;
        case "0":
        case "f":
        case "F":
          e.preventDefault();
          resetFit();
          triggerToast("已适应窗口");
          break;
        case "i":
        case "I":
          e.preventDefault();
          setShowInfoModal((prev) => !prev);
          break;
        case "t":
        case "T":
          e.preventDefault();
          setShowThumbnailStrip((prev) => !prev);
          break;
        case "c":
        case "C":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            if (e.shiftKey) {
              void copyPath();
            } else {
              void copyImage();
            }
          }
          break;
        case "F11":
          e.preventDefault();
          void handleToggleFullscreen();
          break;
        case "Escape":
          e.preventDefault();
          if (showMoreMenu) {
            setShowMoreMenu(false);
          } else if (showInfoModal) {
            setShowInfoModal(false);
          } else if (showThumbnailStrip) {
            setShowThumbnailStrip(false);
          } else if (isFullscreen) {
            void handleToggleFullscreen();
          } else {
            handleClose();
          }
          break;
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    handlePrev,
    handleNext,
    resetFit,
    showMoreMenu,
    showInfoModal,
    showThumbnailStrip,
    isFullscreen,
    isCheckerboard,
    copyImage,
    copyPath,
    handleClose,
    handleToggleAlwaysOnTop,
    triggerToast,
  ]);

  // 计算图片 URL：优先使用 Tauri convertFileSrc，备用 stream 协议
  const computeImageUrl = (path: string, fallback: boolean) => {
    if (!path) return "";
    if (fallback) {
      return `stream://localhost?file=${encodeURIComponent(path)}`;
    }
    if (isDesktop()) {
      try {
        return convertFileSrc(path);
      } catch {
        return `stream://localhost?file=${encodeURIComponent(path)}`;
      }
    }
    return `stream://localhost?file=${encodeURIComponent(path)}`;
  };

  const imgUrl = computeImageUrl(currentPath, loadFallback);

  return (
    <div
      className={`maobu-viewer-root ${isCheckerboard ? "checkerboard-bg" : ""}`}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={handleMouseUp}
    >
      {/* 顶部自定义旗舰微晶标题栏 */}
      <div
        data-tauri-drag-region
        className={`maobu-viewer-titlebar ${!showControls ? "hidden" : ""}`}
        onMouseEnter={() => {
          if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
        }}
        onMouseLeave={resetHideControlsTimer}
      >
        <div className="maobu-viewer-title-info" data-tauri-drag-region>
          <div className="maobu-viewer-brand-badge" data-tauri-drag-region>
            <ImageIcon className="maobu-viewer-title-icon" />
            <span className="maobu-viewer-brand-name">猫步看图</span>
          </div>
          <span className="maobu-viewer-title-text" data-tauri-drag-region>
            {imageTitle || "Maobu Image Viewer"}
          </span>
          {gallery.length > 0 && (
            <span className="maobu-viewer-counter-badge">
              {currentIndex + 1} / {gallery.length}
            </span>
          )}
        </div>

        <div className="maobu-viewer-window-controls">
          <button
            type="button"
            onClick={handleToggleAlwaysOnTop}
            title={isAlwaysOnTop ? "取消置顶 (P)" : "置顶窗口 (P)"}
            className={`maobu-viewer-icon-btn ${isAlwaysOnTop ? "active" : ""}`}
          >
            {isAlwaysOnTop ? <Pin size={13} /> : <PinOff size={13} />}
          </button>
          {isDesktop() && (
            <>
              <button
                type="button"
                onClick={handleMinimize}
                title="最小化"
                className="maobu-viewer-icon-btn"
              >
                <Minus size={13} />
              </button>
              <button
                type="button"
                onClick={handleToggleMaximize}
                title="最大化 / 还原"
                className="maobu-viewer-icon-btn"
              >
                <Maximize2 size={13} />
              </button>
              <button
                type="button"
                onClick={handleClose}
                title="关闭 (Esc)"
                className="maobu-viewer-icon-btn close"
              >
                <X size={14} />
              </button>
            </>
          )}
        </div>
      </div>

      {/* 核心视口与画布区域 (单击切换显隐，双击重置) */}
      <div
        ref={stageRef}
        className={`maobu-viewer-stage ${isDragging ? "dragging" : ""}`}
        onMouseDown={handleMouseDown}
        onWheel={handleWheel}
        onDoubleClick={resetFit}
      >
        {/* 左右侧快速切图微晶翼卡 */}
        {gallery.length > 1 && (
          <>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                handlePrev();
              }}
              title="上一张 (← / A)"
              className={`maobu-viewer-nav-btn left ${!showControls ? "hidden-controls" : ""}`}
            >
              <ChevronLeft size={20} />
            </button>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                handleNext();
              }}
              title="下一张 (→ / D / 空格)"
              className={`maobu-viewer-nav-btn right ${!showControls ? "hidden-controls" : ""}`}
            >
              <ChevronRight size={20} />
            </button>
          </>
        )}

        {/* 画布缩放平移变换层 */}
        <div
          className="maobu-viewer-canvas-wrap"
          style={{
            transform: `translate(${position.x}px, ${position.y}px) scale(${scale}) rotate(${rotation}deg) scaleX(${flipH ? -1 : 1})`,
          }}
        >
          {imgUrl && (
            <img
              ref={imgRef}
              src={imgUrl}
              alt={imageTitle}
              className={`maobu-viewer-image ${filterMode !== "none" ? `filter-${filterMode}` : ""}`}
              onLoad={(e) => {
                const img = e.currentTarget;
                const naturalW = img.naturalWidth;
                const naturalH = img.naturalHeight;
                setNaturalSize({ width: naturalW, height: naturalH });

                // 首次打开看图器时居中；后续切图时绝对不强制居中，保持用户当前放置的窗口位置！
                if (isDesktop() && naturalW > 0 && naturalH > 0) {
                  const optimal = calculateOptimalViewerSize(naturalW, naturalH);
                  const shouldCenter = isInitialLoadRef.current;
                  isInitialLoadRef.current = false;
                  void api.imageViewerWindowSetSize(optimal.width, optimal.height, shouldCenter);
                }

                // 初始化居中
                setScale(1);
                setPosition({ x: 0, y: 0 });
              }}
              onError={() => {
                if (!loadFallback) {
                  setLoadFallback(true);
                } else {
                  triggerToast("加载图片失败，可能文件已移动或损坏");
                }
              }}
            />
          )}
        </div>
      </div>

      {/* 底部「更多工具」微晶浮岛弹出箱 (Toolbox Island) */}
      {showControls && showMoreMenu && (
        <div
          className="maobu-viewer-more-menu"
          onClick={(e) => e.stopPropagation()}
          onMouseEnter={() => {
            if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
          }}
          onMouseLeave={resetHideControlsTimer}
        >
          {/* 水平翻转 */}
          <button
            type="button"
            onClick={() => {
              setFlipH((prev) => !prev);
            }}
            title="水平翻转 (H)"
            className={`maobu-viewer-icon-btn ${flipH ? "active" : ""}`}
          >
            <FlipHorizontal size={13} />
          </button>

          {/* 色彩滤镜 */}
          <button
            type="button"
            onClick={cycleFilter}
            title="色彩滤镜 (对比度/黑白/反色)"
            className={`maobu-viewer-icon-btn ${filterMode !== "none" ? "active" : ""}`}
          >
            <Sun size={13} />
          </button>

          {/* 透明棋盘格 */}
          <button
            type="button"
            onClick={() => {
              setIsCheckerboard((prev) => !prev);
              triggerToast(!isCheckerboard ? "已开启透明棋盘格" : "已恢复暗夜背景");
            }}
            title="切换透明棋盘格背景 (B)"
            className={`maobu-viewer-icon-btn ${isCheckerboard ? "active" : ""}`}
          >
            <Grid size={13} />
          </button>

          {/* 幻灯片轮播 */}
          <button
            type="button"
            onClick={() => {
              setIsPlayingSlideshow((prev) => !prev);
              triggerToast(!isPlayingSlideshow ? "开始幻灯片播放" : "已暂停幻灯片");
            }}
            title={isPlayingSlideshow ? "暂停幻灯片" : "幻灯片播放"}
            className={`maobu-viewer-icon-btn ${isPlayingSlideshow ? "active" : ""}`}
          >
            {isPlayingSlideshow ? <Pause size={13} /> : <Play size={13} />}
          </button>

          {/* 胶卷抽屉 */}
          <button
            type="button"
            onClick={() => {
              setShowThumbnailStrip((prev) => !prev);
            }}
            title="缩略图胶卷 (T)"
            className={`maobu-viewer-icon-btn ${showThumbnailStrip ? "active" : ""}`}
          >
            <Sliders size={13} />
          </button>

          {/* 图片信息 */}
          <button
            type="button"
            onClick={() => {
              setShowInfoModal((prev) => !prev);
            }}
            title="图片信息 (I)"
            className={`maobu-viewer-icon-btn ${showInfoModal ? "active" : ""}`}
          >
            <Info size={13} />
          </button>

          {/* 复制图片 */}
          <button
            type="button"
            onClick={() => void copyImage()}
            title="复制图片 (Ctrl+C)"
            className="maobu-viewer-icon-btn"
          >
            <Copy size={13} />
          </button>
        </div>
      )}

      {/* 底部微晶悬浮主控制岛台 (超紧凑 260px 宽度，任何窄窗口 100% 完整展示绝不截断) */}
      <div
        className={`maobu-viewer-bottom-bar ${!showControls ? "hidden" : ""}`}
        onMouseEnter={() => {
          if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
        }}
        onMouseLeave={resetHideControlsTimer}
      >
        {/* 上一张 / 下一张 */}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            handlePrev();
          }}
          disabled={gallery.length <= 1}
          title="上一张 (←)"
          className="maobu-viewer-icon-btn"
        >
          <ChevronLeft size={14} />
        </button>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            handleNext();
          }}
          disabled={gallery.length <= 1}
          title="下一张 (→)"
          className="maobu-viewer-icon-btn"
        >
          <ChevronRight size={14} />
        </button>

        <div className="maobu-viewer-bar-divider" />

        {/* 缩放控制 */}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            setScale((prev) => Math.max(0.1, parseFloat((prev * 0.8).toFixed(2))));
          }}
          title="缩小"
          className="maobu-viewer-icon-btn"
        >
          <Minus size={13} />
        </button>
        <span
          className="maobu-viewer-scale-text"
          onClick={(e) => {
            e.stopPropagation();
            resetFit();
          }}
          title="点击重置为适应窗口 (0/F)"
        >
          {Math.round(scale * 100)}%
        </span>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            setScale((prev) => Math.min(10, parseFloat((prev * 1.25).toFixed(2))));
          }}
          title="放大"
          className="maobu-viewer-icon-btn"
        >
          <Plus size={13} />
        </button>

        <div className="maobu-viewer-bar-divider" />

        {/* 1:1 原图 */}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            setScale(1);
            setPosition({ x: 0, y: 0 });
            triggerToast("100% 原始尺寸");
          }}
          title="1:1 实际像素 (1)"
          className={`maobu-viewer-icon-btn ${scale === 1 ? "active" : ""}`}
          style={{ fontSize: "10px", fontFamily: "monospace", fontWeight: 600 }}
        >
          1:1
        </button>

        {/* 旋转 90° */}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            setRotation((prev) => (prev + 90) % 360);
          }}
          title="顺时针旋转 90° (R)"
          className={`maobu-viewer-icon-btn ${rotation !== 0 ? "active" : ""}`}
        >
          <RotateCw size={13} />
        </button>

        {/* 窗口置顶 */}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            void handleToggleAlwaysOnTop();
          }}
          title={isAlwaysOnTop ? "取消置顶 (P)" : "置顶窗口 (P)"}
          className={`maobu-viewer-icon-btn ${isAlwaysOnTop ? "active" : ""}`}
        >
          <Pin size={13} />
        </button>

        {/* 更多功能微晶抽屉 (滤镜/翻转/网格/幻灯片/胶卷/属性/复制) */}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            setShowMoreMenu((prev) => !prev);
          }}
          title="更多工具 (滤镜/翻转/网格/幻灯片/胶卷/信息/复制)"
          className={`maobu-viewer-icon-btn ${showMoreMenu ? "active" : ""}`}
        >
          <MoreHorizontal size={14} />
        </button>

        {/* 全屏切换 */}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            void handleToggleFullscreen();
          }}
          title="全屏 (F11)"
          className="maobu-viewer-icon-btn"
        >
          {isFullscreen ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
        </button>
      </div>

      {/* 底部缩略图胶卷 Strip */}
      {showThumbnailStrip && gallery.length > 0 && (
        <div className="maobu-viewer-thumbnail-strip" onClick={(e) => e.stopPropagation()}>
          {gallery.map((item, idx) => {
            const isActive = idx === currentIndex;
            const thumbUrl = computeImageUrl(item.path, false);
            return (
              <div
                key={item.path}
                onClick={() => loadImage(item.path, item.name)}
                className={`maobu-viewer-thumbnail-item ${isActive ? "active" : ""}`}
                title={`${item.name} (${formatBytes(item.size_bytes)})`}
              >
                <img src={thumbUrl} alt={item.name} loading="lazy" />
              </div>
            );
          })}
        </div>
      )}

      {/* 图片信息元数据浮窗 */}
      {showInfoModal && (
        <div className="maobu-viewer-info-modal" onClick={(e) => e.stopPropagation()}>
          <div className="maobu-viewer-info-title">
            <span>图片属性</span>
            <button
              type="button"
              onClick={() => setShowInfoModal(false)}
              className="maobu-viewer-icon-btn"
              style={{ width: "20px", height: "20px", padding: 0 }}
            >
              <X size={12} />
            </button>
          </div>
          <div className="maobu-viewer-info-row">
            <span className="maobu-viewer-info-label">文件名</span>
            <span className="maobu-viewer-info-val" title={imageInfo?.name || imageTitle}>
              {imageInfo?.name || imageTitle}
            </span>
          </div>
          {naturalSize && (
            <div className="maobu-viewer-info-row">
              <span className="maobu-viewer-info-label">分辨率</span>
              <span className="maobu-viewer-info-val">
                {naturalSize.width} × {naturalSize.height} px
              </span>
            </div>
          )}
          {imageInfo && imageInfo.size_bytes > 0 && (
            <div className="maobu-viewer-info-row">
              <span className="maobu-viewer-info-label">文件大小</span>
              <span className="maobu-viewer-info-val">{formatBytes(imageInfo.size_bytes)}</span>
            </div>
          )}
          {imageInfo?.ext && (
            <div className="maobu-viewer-info-row">
              <span className="maobu-viewer-info-label">图像格式</span>
              <span className="maobu-viewer-info-val">{imageInfo.ext.toUpperCase()}</span>
            </div>
          )}
          {imageInfo && imageInfo.modified_ms > 0 && (
            <div className="maobu-viewer-info-row">
              <span className="maobu-viewer-info-label">修改时间</span>
              <span className="maobu-viewer-info-val">
                {formatDate(imageInfo.modified_ms)}
              </span>
            </div>
          )}
          <div className="maobu-viewer-info-row" style={{ marginTop: "4px" }}>
            <button
              type="button"
              onClick={copyPath}
              className="maobu-viewer-icon-btn"
              style={{ width: "100%", height: "24px", fontSize: "11px", gap: "4px" }}
            >
              <FolderOpen size={12} />
              复制绝对路径
            </button>
          </div>
        </div>
      )}

      {/* 轻提示 Toast */}
      {toastMessage && <div className="maobu-viewer-toast">{toastMessage}</div>}
    </div>
  );
}
