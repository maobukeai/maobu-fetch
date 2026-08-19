import { useEffect, useMemo, useState, type MouseEvent } from "react";
import { getCurrentWindow, Effect } from "@tauri-apps/api/window";
import { isDesktop } from "../../api";
import type { AppSettings, ColorScheme } from "../../types";

export function usesDarkTheme(colorScheme: ColorScheme | AppSettings["theme"]) {
  return (
    colorScheme === "dark" ||
    (colorScheme === "system" &&
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches)
  );
}

export async function applyWindowAppearance(frostedGlass: boolean, dark: boolean) {
  if (typeof document !== "undefined") {
    document.documentElement.dataset.windowStyle = frostedGlass ? "frosted" : "solid";
  }
  if (!isDesktop()) return;

  const appWindow = getCurrentWindow();
  if (frostedGlass) {
    await appWindow.setEffects({
      effects: [Effect.Acrylic],
      color: dark ? [24, 24, 27, 72] : [246, 248, 252, 56],
    });
  } else {
    await appWindow.clearEffects();
  }
}

export function Titlebar() {
  const [isMaximized, setIsMaximized] = useState(false);
  const appWindow = useMemo(() => (isDesktop() ? getCurrentWindow() : null), []);

  useEffect(() => {
    if (!appWindow) return;
    void appWindow.isMaximized().then(setIsMaximized);
    let unlisten: (() => void) | undefined;
    appWindow
      .onResized(() => {
        void appWindow.isMaximized().then(setIsMaximized);
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      if (unlisten) unlisten();
    };
  }, [appWindow]);

  const handleMinimize = () => {
    void appWindow?.minimize();
  };
  const handleMaximize = () => {
    void appWindow?.toggleMaximize();
  };
  const handleClose = () => {
    void appWindow?.close();
  };

  return (
    <div className="window-titlebar" data-tauri-drag-region>
      <div className="window-titlebar-title" data-tauri-drag-region>
        猫步下载器 · Maobu Fetch
      </div>
      <div className="window-controls">
        <button
          className="window-control-btn min"
          onClick={handleMinimize}
          title="最小化"
        >
          <svg width="10" height="1" viewBox="0 0 10 1">
            <rect width="10" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          className="window-control-btn max"
          onClick={handleMaximize}
          title={isMaximized ? "向下还原" : "最大化"}
        >
          {isMaximized ? (
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path
                d="M1.5,3.5 L1.5,8.5 L6.5,8.5 L6.5,3.5 Z"
                fill="none"
                stroke="currentColor"
                strokeWidth="1"
              />
              <path
                d="M3.5,1.5 L8.5,1.5 L8.5,6.5"
                fill="none"
                stroke="currentColor"
                strokeWidth="1"
              />
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 10 10">
              <rect
                width="10"
                height="10"
                fill="none"
                stroke="currentColor"
                strokeWidth="1"
              />
            </svg>
          )}
        </button>
        <button
          className="window-control-btn close"
          onClick={handleClose}
          title="关闭"
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path
              d="M0,0 L10,10 M10,0 L0,10"
              stroke="currentColor"
              strokeWidth="1"
            />
          </svg>
        </button>
      </div>
    </div>
  );
}

export function WindowResizeHandles() {
  if (!isDesktop()) return null;

  const handleMouseDown = (direction: string, event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    try {
      const appWindow = getCurrentWindow();
      void appWindow.startResizeDragging(direction as any);
    } catch (err) {
      console.error("Failed to start resize dragging:", err);
    }
  };

  const directions = [
    { key: "top", dir: "North" },
    { key: "bottom", dir: "South" },
    { key: "left", dir: "West" },
    { key: "right", dir: "East" },
    { key: "top-left", dir: "NorthWest" },
    { key: "top-right", dir: "NorthEast" },
    { key: "bottom-left", dir: "SouthWest" },
    { key: "bottom-right", dir: "SouthEast" },
  ];

  return (
    <>
      {directions.map(({ key, dir }) => (
        <div
          key={key}
          className={`resize-handle ${key}`}
          onMouseDown={(e) => handleMouseDown(dir, e)}
        />
      ))}
    </>
  );
}
