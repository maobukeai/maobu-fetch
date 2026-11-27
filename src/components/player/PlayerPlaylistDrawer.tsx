import type { MouseEvent, ReactNode, RefObject } from "react";
import {
  ArrowUpDown,
  Check,
  Copy,
  FolderOpen,
  ListVideo,
  LocateFixed,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { formatBytes } from "../../formatters";
import type { PlaylistItem } from "../../types";
import type { PlayMode, PlaylistSortType } from "./player-ui-model";

const SORT_LABELS: Record<PlaylistSortType, string> = {
  default: "默认",
  "name-asc": "名称 ↑",
  "name-desc": "名称 ↓",
  "size-desc": "大小 ↓",
  "size-asc": "大小 ↑",
};

interface PlayerPlaylistDrawerProps {
  drawerRef: RefObject<HTMLDivElement | null>;
  activeItemRef: RefObject<HTMLDivElement | null>;
  open: boolean;
  playlist: PlaylistItem[];
  displayPlaylist: PlaylistItem[];
  filePath: string;
  searchKeyword: string;
  playMode: PlayMode;
  sortType: PlaylistSortType;
  showSortMenu: boolean;
  modeLabel: Record<PlayMode, string>;
  renderModeIcon: () => ReactNode;
  onClose: () => void;
  onLocateActive: () => void;
  onAddClick: () => void;
  onClearPlaylist: () => void;
  onSearchChange: (val: string) => void;
  onCyclePlayMode: () => void;
  onToggleSortMenu: () => void;
  onSelectSort: (nextSort: PlaylistSortType) => void;
  onPlayItem: (path: string, name: string) => void;
  onCopyItemPath: (e: MouseEvent, path: string) => void;
  onOpenItemFolder: (e: MouseEvent, path: string) => void;
  onRemoveItem: (e: MouseEvent, path: string) => void;
}

export function PlayerPlaylistDrawer({
  drawerRef,
  activeItemRef,
  open,
  playlist,
  displayPlaylist,
  filePath,
  searchKeyword,
  playMode,
  sortType,
  showSortMenu,
  modeLabel,
  renderModeIcon,
  onClose,
  onLocateActive,
  onAddClick,
  onClearPlaylist,
  onSearchChange,
  onCyclePlayMode,
  onToggleSortMenu,
  onSelectSort,
  onPlayItem,
  onCopyItemPath,
  onOpenItemFolder,
  onRemoveItem,
}: PlayerPlaylistDrawerProps) {
  return (
    <div
      ref={drawerRef}
      className={`maobu-player-playlist-drawer ${open ? "open" : ""}`}
      onClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className="maobu-player-playlist-header">
        <div className="maobu-player-playlist-title">
          <ListVideo size={15} className="maobu-player-accent-icon" />
          <span>播放列表</span>
          <span className="maobu-player-playlist-count">{playlist.length}</span>
        </div>

        <div className="maobu-player-playlist-header-actions">
          <button
            type="button"
            onClick={onLocateActive}
            title="定位当前视频"
            className="maobu-player-icon-btn"
          >
            <LocateFixed size={14} />
          </button>
          <button
            type="button"
            onClick={onAddClick}
            title="添加视频"
            className="maobu-player-icon-btn"
          >
            <Plus size={14} />
          </button>
          <button
            type="button"
            onClick={onClearPlaylist}
            title="清空列表"
            className="maobu-player-icon-btn"
          >
            <Trash2 size={14} />
          </button>
          <button
            type="button"
            onClick={onClose}
            title="关闭列表 (L)"
            className="maobu-player-icon-btn"
          >
            <X size={14} />
          </button>
        </div>
      </div>

      <div className="maobu-player-playlist-search-wrap">
        <Search size={13} className="maobu-player-playlist-search-icon" />
        <input
          type="text"
          value={searchKeyword}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder="搜索视频..."
          className="maobu-player-playlist-search-input"
        />
        {searchKeyword && (
          <button
            type="button"
            onClick={() => onSearchChange("")}
            className="maobu-player-playlist-search-clear"
            title="清除搜索"
          >
            <X size={12} />
          </button>
        )}
      </div>

      <div className="maobu-player-playlist-toolbar">
        <button
          type="button"
          onClick={onCyclePlayMode}
          className="maobu-player-playlist-mode-btn"
          title="切换播放模式"
        >
          {renderModeIcon()}
          <span>{modeLabel[playMode]}</span>
        </button>

        <div className="maobu-player-rel">
          <button
            type="button"
            onClick={onToggleSortMenu}
            className={`maobu-player-playlist-mode-btn ${sortType !== "default" ? "active" : ""}`}
            title="列表排序"
          >
            <ArrowUpDown size={12} />
            <span>{SORT_LABELS[sortType]}</span>
          </button>

          {showSortMenu && (
            <div className="maobu-player-sort-menu">
              {(
                [
                  ["default", "默认排序"],
                  ["name-asc", "名称升序 (A → Z)"],
                  ["name-desc", "名称降序 (Z → A)"],
                  ["size-desc", "大小降序"],
                  ["size-asc", "大小升序"],
                ] as const
              ).map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  onClick={() => onSelectSort(key)}
                  className={`maobu-player-menu-item ${sortType === key ? "active" : ""}`}
                >
                  <span>{label}</span>
                  {sortType === key && <Check size={12} />}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="maobu-player-playlist-items">
        {displayPlaylist.length > 0 ? (
          displayPlaylist.map((item, idx) => {
            const isActive = item.path === filePath;
            return (
              <div
                key={item.path}
                ref={isActive ? activeItemRef : undefined}
                className={`maobu-player-playlist-item ${isActive ? "active" : ""}`}
                onClick={() => onPlayItem(item.path, item.name)}
              >
                <div className="maobu-player-item-left">
                  {isActive ? (
                    <div className="maobu-wave-icon">
                      <div className="maobu-wave-bar" />
                      <div className="maobu-wave-bar" />
                      <div className="maobu-wave-bar" />
                    </div>
                  ) : (
                    <span className="maobu-player-item-index">{idx + 1}</span>
                  )}

                  <div className="maobu-player-item-details">
                    <span className="maobu-player-item-name" title={item.name}>
                      {item.name}
                    </span>
                    {item.size_bytes > 0 && (
                      <span className="maobu-player-item-meta">
                        {formatBytes(item.size_bytes)}
                      </span>
                    )}
                  </div>
                </div>

                <div className="maobu-player-item-actions">
                  <button
                    type="button"
                    onClick={(e) => onCopyItemPath(e, item.path)}
                    title="复制路径"
                    className="maobu-player-icon-btn maobu-player-item-btn"
                  >
                    <Copy size={13} />
                  </button>
                  <button
                    type="button"
                    onClick={(e) => onOpenItemFolder(e, item.path)}
                    title="打开所在目录"
                    className="maobu-player-icon-btn maobu-player-item-btn"
                  >
                    <FolderOpen size={13} />
                  </button>
                  <button
                    type="button"
                    onClick={(e) => onRemoveItem(e, item.path)}
                    title="移除"
                    className="maobu-player-icon-btn maobu-player-item-btn"
                  >
                    <X size={13} />
                  </button>
                </div>
              </div>
            );
          })
        ) : (
          <div className="maobu-player-playlist-empty">
            {searchKeyword ? "未找到匹配视频" : "暂无视频，拖入或点击 + 添加"}
          </div>
        )}
      </div>

      <div className="maobu-player-playlist-footer">
        <span className="tabular">
          共 {playlist.length} 项
          {searchKeyword ? ` · 匹配 ${displayPlaylist.length} 项` : ""}
        </span>
        <span>支持拖入文件</span>
      </div>
    </div>
  );
}
