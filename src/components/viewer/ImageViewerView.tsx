import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ChevronLeft,
  ChevronRight,
  Film,
  Info,
  Maximize2,
  Minimize2,
  Minus,
  MoreHorizontal,
  Pin,
  PinOff,
  Plus,
  RotateCw,
  X,
} from "lucide-react";
import { api, isDesktop } from "../../api";
import type { ImageFileInfo, ImageItem } from "../../types";
import { calculateOptimalViewerSize } from "./image-viewer-utils";
import {
  clampScale,
  computeCursorAnchoredZoom,
  computeViewerImageUrl,
  nextFilterMode,
  VIEWER_FILTER_LABELS,
  type ViewerFilterMode,
  type ViewerPosition,
} from "./image-viewer-model";
import { ViewerThumbnailStrip } from "./ViewerThumbnailStrip";
import { ViewerInfoDrawer } from "./ViewerInfoDrawer";
import { ViewerMoreMenu } from "./ViewerMoreMenu";
import "./ImageViewer.css";

interface ImageViewerProps {
  initialFile?: string;
  initialTitle?: string;
}

interface ViewportTransform {
  scale: number;
  position: ViewerPosition;
}

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

  const [gallery, setGallery] = useState<ImageItem[]>([]);
  const [currentIndex, setCurrentIndex] = useState<number>(0);

  const [viewport, setViewport] = useState<ViewportTransform>({
    scale: 1,
    position: { x: 0, y: 0 },
  });
  const [rotation, setRotation] = useState<number>(0);
  const [flipH, setFlipH] = useState<boolean>(false);
  const [filterMode, setFilterMode] = useState<ViewerFilterMode>("none");
  const [isCheckerboard, setIsCheckerboard] = useState<boolean>(false);

  const [isDragging, setIsDragging] = useState<boolean>(false);
  const dragStartRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const posStartRef = useRef<ViewerPosition>({ x: 0, y: 0 });
  const clickStartRef = useRef<{ time: number; x: number; y: number }>({ time: 0, x: 0, y: 0 });

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
  const slideshowTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const isInitialLoadRef = useRef<boolean>(true);
  const isLoadedRef = useRef<boolean>(false);

  const triggerToast = useCallback((msg: string) => {
    setToastMessage(msg);
    if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    toastTimerRef.current = setTimeout(() => setToastMessage(""), 2000);
  }, []);

  const resetFit = useCallback(() => {
    setViewport({ scale: 1, position: { x: 0, y: 0 } });
  }, []);

  const stepScale = useCallback((factor: number) => {
    setViewport((prev) => ({
      ...prev,
      scale: clampScale(prev.scale * factor),
    }));
  }, []);

  const cycleFilter = useCallback(() => {
    setFilterMode((prev) => {
      const next = nextFilterMode(prev);
      triggerToast(next.toast);
      return next.mode;
    });
  }, [triggerToast]);

  const loadImage = useCallback(async (filePath: string, title?: string) => {
    if (!filePath) return;
    setCurrentPath(filePath);
    setLoadFallback(false);
    const name = filePath.split(/[\\/]/).pop() || filePath;
    setImageTitle(title || name);
    setRotation(0);
    setFlipH(false);
    setViewport({ scale: 1, position: { x: 0, y: 0 } });
    setNaturalSize(null);

    if (!isDesktop()) return;

    void api.imageViewerNotifyFileChanged(filePath, title || name);

    try {
      const info = await api.imageViewerGetInfo(filePath);
      setImageInfo(info);
    } catch {
      setImageInfo(null);
    }

    try {
      const items = await api.imageViewerGetFolderImages(filePath);
      setGallery(items);
      const idx = items.findIndex((it) => it.path === filePath);
      setCurrentIndex(idx >= 0 ? idx : 0);
    } catch (e) {
      console.error("扫描同目录图片失败:", e);
    }
  }, []);

  useEffect(() => {
    if (isLoadedRef.current) return;
    if (currentPath) {
      isLoadedRef.current = true;
      void loadImage(currentPath, imageTitle);
    } else if (isDesktop()) {
      void api.imageViewerGetCurrentFile().then((res) => {
        if (res && res[0]) {
          isLoadedRef.current = true;
          void loadImage(res[0], res[1] || undefined);
        }
      });
    }
  }, [currentPath, imageTitle, loadImage]);

  useEffect(() => {
    if (!isDesktop()) return;
    let unlistenFn: (() => void) | undefined;
    void listen<{ file: string; title?: string }>("image-viewer-load-file", (event) => {
      if (event.payload?.file) {
        void loadImage(event.payload.file, event.payload.title);
      }
    }).then((fn) => {
      unlistenFn = fn;
    });

    return () => {
      if (unlistenFn) unlistenFn();
    };
  }, [loadImage]);

  const handlePrev = useCallback(() => {
    if (gallery.length <= 1) return;
    const nextIdx = (currentIndex - 1 + gallery.length) % gallery.length;
    const target = gallery[nextIdx];
    if (target) void loadImage(target.path, target.name);
  }, [gallery, currentIndex, loadImage]);

  const handleNext = useCallback(() => {
    if (gallery.length <= 1) return;
    const nextIdx = (currentIndex + 1) % gallery.length;
    const target = gallery[nextIdx];
    if (target) void loadImage(target.path, target.name);
  }, [gallery, currentIndex, loadImage]);

  useEffect(() => {
    if (!isPlayingSlideshow) {
      if (slideshowTimerRef.current) clearInterval(slideshowTimerRef.current);
      return;
    }
    slideshowTimerRef.current = setInterval(handleNext, 3000);
    return () => {
      if (slideshowTimerRef.current) clearInterval(slideshowTimerRef.current);
    };
  }, [isPlayingSlideshow, handleNext]);

  const clearHideTimer = useCallback(() => {
    if (hideTimerRef.current) {
      clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
  }, []);

  const resetHideControlsTimer = useCallback(() => {
    if (isManualHidden) return;
    setShowControls(true);
    clearHideTimer();
    hideTimerRef.current = setTimeout(() => {
      if (!showThumbnailStrip && !showInfoModal && !showMoreMenu) {
        setShowControls(false);
      }
    }, 1800);
  }, [isManualHidden, showThumbnailStrip, showInfoModal, showMoreMenu, clearHideTimer]);

  const toggleControls = useCallback(() => {
    setIsManualHidden((prev) => {
      const nextManual = !prev;
      if (nextManual) {
        setShowControls(false);
        setShowMoreMenu(false);
        setShowThumbnailStrip(false);
        setShowInfoModal(false);
        clearHideTimer();
      } else {
        setShowControls(true);
        clearHideTimer();
        hideTimerRef.current = setTimeout(() => {
          if (!showThumbnailStrip && !showInfoModal && !showMoreMenu) {
            setShowControls(false);
          }
        }, 1800);
      }
      return nextManual;
    });
  }, [showThumbnailStrip, showInfoModal, showMoreMenu, clearHideTimer]);

  useEffect(() => {
    const stageEl = stageRef.current;
    if (!stageEl) return;

    const handleWheel = (e: WheelEvent) => {
      e.preventDefault();
      const factor = e.deltaY < 0 ? 1.15 : 0.87;
      const rect = stageEl.getBoundingClientRect();
      const pointerX = e.clientX;
      const pointerY = e.clientY;
      setViewport((prev) =>
        computeCursorAnchoredZoom({
          scale: prev.scale,
          position: prev.position,
          factor,
          pointerX,
          pointerY,
          stageRect: rect,
        })
      );
      resetHideControlsTimer();
    };

    stageEl.addEventListener("wheel", handleWheel, { passive: false });
    return () => stageEl.removeEventListener("wheel", handleWheel);
  }, [resetHideControlsTimer]);

  const handleMouseDown = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    setIsDragging(true);
    dragStartRef.current = { x: e.clientX, y: e.clientY };
    posStartRef.current = { ...viewport.position };
    clickStartRef.current = { time: Date.now(), x: e.clientX, y: e.clientY };
  };

  const handleMouseMove = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (isManualHidden) {
      if (e.clientY < 36 || e.clientY > window.innerHeight - 48) {
        setIsManualHidden(false);
        setShowControls(true);
      } else if (isDragging) {
        const dx = e.clientX - dragStartRef.current.x;
        const dy = e.clientY - dragStartRef.current.y;
        setViewport((prev) => ({
          ...prev,
          position: {
            x: posStartRef.current.x + dx,
            y: posStartRef.current.y + dy,
          },
        }));
        return;
      } else {
        return;
      }
    }

    resetHideControlsTimer();
    if (!isDragging) return;
    const dx = e.clientX - dragStartRef.current.x;
    const dy = e.clientY - dragStartRef.current.y;
    setViewport((prev) => ({
      ...prev,
      position: {
        x: posStartRef.current.x + dx,
        y: posStartRef.current.y + dy,
      },
    }));
  };

  const handleMouseUp = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (!isDragging) return;
    setIsDragging(false);
    const duration = Date.now() - clickStartRef.current.time;
    const dx = Math.abs(e.clientX - clickStartRef.current.x);
    const dy = Math.abs(e.clientY - clickStartRef.current.y);
    if (duration < 250 && dx < 5 && dy < 5) {
      if (showMoreMenu) {
        setShowMoreMenu(false);
      } else {
        toggleControls();
      }
    }
  };

  const copyPath = useCallback(async () => {
    if (!currentPath) return;
    try {
      await navigator.clipboard.writeText(currentPath);
      triggerToast("已复制文件路径");
    } catch {
      triggerToast("当前环境不支持剪贴板写入");
    }
  }, [currentPath, triggerToast]);

  const copyImage = useCallback(async () => {
    if (!currentPath) return;
    try {
      if (imgRef.current) {
        const canvas = document.createElement("canvas");
        canvas.width = imgRef.current.naturalWidth;
        canvas.height = imgRef.current.naturalHeight;
        const ctx = canvas.getContext("2d");
        if (ctx && canvas.width > 0 && canvas.height > 0) {
          ctx.drawImage(imgRef.current, 0, 0);
          canvas.toBlob(async (blob) => {
            if (blob) {
              try {
                await navigator.clipboard.write([
                  new ClipboardItem({ "image/png": blob }),
                ]);
                triggerToast("已复制图片到剪贴板");
                return;
              } catch {
                await copyPath();
                return;
              }
            }
            await copyPath();
          }, "image/png");
          return;
        }
      }
      await copyPath();
    } catch {
      await copyPath();
    }
  }, [currentPath, copyPath, triggerToast]);

  const handleMinimize = () => void api.imageViewerWindowMinimize();
  const handleToggleMaximize = () => void api.imageViewerWindowToggleMaximize();
  const handleClose = useCallback(() => void api.imageViewerWindowClose(), []);
  const handleToggleFullscreen = useCallback(async () => {
    const fs = await api.imageViewerWindowToggleFullscreen();
    setIsFullscreen(fs);
  }, []);
  const handleToggleAlwaysOnTop = useCallback(async () => {
    const top = await api.imageViewerWindowToggleAlwaysOnTop();
    setIsAlwaysOnTop(top);
    triggerToast(top ? "已置顶窗口" : "已取消置顶");
  }, [triggerToast]);

  const handleToggleCheckerboard = useCallback(() => {
    setIsCheckerboard((prev) => {
      const next = !prev;
      triggerToast(next ? "已开启透明棋盘格" : "已恢复中性暗底");
      return next;
    });
  }, [triggerToast]);

  const handleToggleSlideshow = useCallback(() => {
    setIsPlayingSlideshow((prev) => {
      const next = !prev;
      triggerToast(next ? "开始幻灯片轮播" : "已暂停幻灯片");
      return next;
    });
  }, [triggerToast]);

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
        case "+":
        case "=":
          e.preventDefault();
          stepScale(1.25);
          break;
        case "-":
        case "_":
          e.preventDefault();
          stepScale(0.8);
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
          handleToggleCheckerboard();
          break;
        case "1":
          e.preventDefault();
          resetFit();
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
    stepScale,
    resetFit,
    showMoreMenu,
    showInfoModal,
    showThumbnailStrip,
    isFullscreen,
    copyImage,
    copyPath,
    handleClose,
    handleToggleAlwaysOnTop,
    handleToggleCheckerboard,
    handleToggleFullscreen,
    triggerToast,
  ]);

  const resolveImageUrl = useCallback(
    (path: string, fallback = false) =>
      computeViewerImageUrl(path, fallback, isDesktop(), convertFileSrc),
    []
  );

  const imgUrl = resolveImageUrl(currentPath, loadFallback);
  const filterLabel = VIEWER_FILTER_LABELS[filterMode];
  const hasGalleryStrip = showThumbnailStrip && gallery.length > 0;

  return (
    <div
      className={`maobu-viewer-root ${isCheckerboard ? "checkerboard-bg" : ""}`}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={() => setIsDragging(false)}
    >
      {/* 顶部标题栏 */}
      <div
        data-tauri-drag-region
        className={`maobu-viewer-titlebar ${!showControls ? "hidden" : ""}`}
        onMouseEnter={clearHideTimer}
        onMouseLeave={resetHideControlsTimer}
      >
        <div className="maobu-viewer-title-info" data-tauri-drag-region>
          <span className="maobu-viewer-title-text" data-tauri-drag-region>
            {imageTitle || "猫步看图"}
          </span>
          {gallery.length > 0 && (
            <span className="maobu-viewer-counter-badge">
              {currentIndex + 1} / {gallery.length}
            </span>
          )}
          {filterLabel && (
            <span className="maobu-viewer-filter-badge">{filterLabel}</span>
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

      {/* 核心视口与画布 */}
      <div
        ref={stageRef}
        className={`maobu-viewer-stage ${isDragging ? "dragging" : ""}`}
        onMouseDown={handleMouseDown}
        onDoubleClick={resetFit}
      >
        {gallery.length > 1 && (
          <>
            <button
              type="button"
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                handlePrev();
              }}
              title="上一张 (← / A)"
              className={`maobu-viewer-nav-btn left ${!showControls ? "hidden-controls" : ""}`}
            >
              <ChevronLeft size={18} />
            </button>
            <button
              type="button"
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                handleNext();
              }}
              title="下一张 (→ / D / 空格)"
              className={`maobu-viewer-nav-btn right ${!showControls ? "hidden-controls" : ""}`}
            >
              <ChevronRight size={18} />
            </button>
          </>
        )}

        <div
          className="maobu-viewer-canvas-wrap"
          style={{
            transform: `translate(${viewport.position.x}px, ${viewport.position.y}px) scale(${viewport.scale}) rotate(${rotation}deg) scaleX(${flipH ? -1 : 1})`,
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

                if (isDesktop() && naturalW > 0 && naturalH > 0) {
                  if (isInitialLoadRef.current) {
                    isInitialLoadRef.current = false;
                  } else {
                    const optimal = calculateOptimalViewerSize(naturalW, naturalH);
                    void api.imageViewerWindowSetSize(optimal.width, optimal.height, false);
                  }
                }

                setViewport({ scale: 1, position: { x: 0, y: 0 } });
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

      {/* 底部更多工具浮层（胶卷开启时自动上抬避免重叠） */}
      {showControls && showMoreMenu && (
        <ViewerMoreMenu
          flipH={flipH}
          filterMode={filterMode}
          isCheckerboard={isCheckerboard}
          isPlayingSlideshow={isPlayingSlideshow}
          hasThumbnailStrip={hasGalleryStrip}
          onToggleFlip={() => setFlipH((prev) => !prev)}
          onCycleFilter={cycleFilter}
          onToggleCheckerboard={handleToggleCheckerboard}
          onToggleSlideshow={handleToggleSlideshow}
          onCopyImage={() => void copyImage()}
          onPointerEnter={clearHideTimer}
          onPointerLeave={resetHideControlsTimer}
        />
      )}

      {/* 底部精密控制浮岛 */}
      <div
        className={`maobu-viewer-bottom-bar ${!showControls ? "hidden" : ""}`}
        onClick={(e) => e.stopPropagation()}
        onMouseEnter={clearHideTimer}
        onMouseLeave={resetHideControlsTimer}
      >
        <button
          type="button"
          onClick={handlePrev}
          disabled={gallery.length <= 1}
          title="上一张 (←)"
          className="maobu-viewer-icon-btn"
        >
          <ChevronLeft size={14} />
        </button>
        <button
          type="button"
          onClick={handleNext}
          disabled={gallery.length <= 1}
          title="下一张 (→)"
          className="maobu-viewer-icon-btn"
        >
          <ChevronRight size={14} />
        </button>

        <div className="maobu-viewer-bar-divider" />

        <button
          type="button"
          onClick={() => stepScale(0.8)}
          title="缩小 (-)"
          className="maobu-viewer-icon-btn"
        >
          <Minus size={13} />
        </button>
        <button
          type="button"
          onClick={resetFit}
          title="点击重置为适应窗口 (0/F)"
          className="maobu-viewer-scale-text"
        >
          {Math.round(viewport.scale * 100)}%
        </button>
        <button
          type="button"
          onClick={() => stepScale(1.25)}
          title="放大 (+)"
          className="maobu-viewer-icon-btn"
        >
          <Plus size={13} />
        </button>
        <button
          type="button"
          onClick={() => {
            resetFit();
            triggerToast("100% 原始尺寸");
          }}
          title="1:1 实际像素 (1)"
          className={`maobu-viewer-icon-btn maobu-viewer-ratio-btn ${viewport.scale === 1 ? "active" : ""}`}
        >
          1:1
        </button>

        <div className="maobu-viewer-bar-divider" />

        <button
          type="button"
          onClick={() => setRotation((prev) => (prev + 90) % 360)}
          title="顺时针旋转 90° (R)"
          className={`maobu-viewer-icon-btn ${rotation !== 0 ? "active" : ""}`}
        >
          <RotateCw size={13} />
        </button>
        <button
          type="button"
          onClick={() => setShowThumbnailStrip((prev) => !prev)}
          title="缩略图胶卷 (T)"
          className={`maobu-viewer-icon-btn ${showThumbnailStrip ? "active" : ""}`}
        >
          <Film size={13} />
        </button>
        <button
          type="button"
          onClick={() => setShowInfoModal((prev) => !prev)}
          title="图片属性 (I)"
          className={`maobu-viewer-icon-btn ${showInfoModal ? "active" : ""}`}
        >
          <Info size={13} />
        </button>
        <button
          type="button"
          onClick={() => setShowMoreMenu((prev) => !prev)}
          title="更多工具"
          className={`maobu-viewer-icon-btn ${showMoreMenu ? "active" : ""}`}
        >
          <MoreHorizontal size={14} />
        </button>

        <div className="maobu-viewer-bar-divider" />

        <button
          type="button"
          onClick={() => void handleToggleFullscreen()}
          title="全屏 (F11)"
          className={`maobu-viewer-icon-btn ${isFullscreen ? "active" : ""}`}
        >
          {isFullscreen ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
        </button>
      </div>

      {/* 底部缩略图胶卷 */}
      {showThumbnailStrip && (
        <ViewerThumbnailStrip
          gallery={gallery}
          currentIndex={currentIndex}
          resolveUrl={(p) => resolveImageUrl(p, false)}
          onSelect={(item) => void loadImage(item.path, item.name)}
          onPointerEnter={clearHideTimer}
          onPointerLeave={resetHideControlsTimer}
        />
      )}

      {/* 右侧图片属性抽屉 */}
      {showInfoModal && (
        <ViewerInfoDrawer
          imageInfo={imageInfo}
          imageTitle={imageTitle}
          naturalSize={naturalSize}
          scale={viewport.scale}
          onClose={() => setShowInfoModal(false)}
          onCopyImage={() => void copyImage()}
          onCopyPath={() => void copyPath()}
          onPointerEnter={clearHideTimer}
          onPointerLeave={resetHideControlsTimer}
        />
      )}

      {/* 轻提示 Toast */}
      {toastMessage && <div className="maobu-viewer-toast">{toastMessage}</div>}
    </div>
  );
}
