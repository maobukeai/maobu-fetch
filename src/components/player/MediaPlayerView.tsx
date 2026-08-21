import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowUpDown,
  Camera,
  Check,
  Copy,
  ExternalLink,
  FastForward,
  Film,
  FolderOpen,
  ListVideo,
  LocateFixed,
  Maximize2,
  Minimize2,
  Minus,
  Pause,
  PictureInPicture,
  Pin,
  PinOff,
  Play,
  Plus,
  Repeat,
  Repeat1,
  Rewind,
  Search,
  Shuffle,
  SkipBack,
  SkipForward,
  Sliders,
  Subtitles,
  Trash2,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api, isDesktop } from "../../api";
import { formatBytes, formatDuration } from "../../formatters";
import type { PlaylistItem, SubtitleItem, SubtitleStyleConfig } from "../../types";
import { getActiveCueText, parseAnySubtitles, type SubtitleCue } from "./SubtitleParser";
import "./MediaPlayer.css";

interface MediaPlayerProps {
  initialFile?: string;
  initialTitle?: string;
}

type PlayMode = "sequence" | "loop" | "single" | "shuffle";

function formatPlayerTime(seconds: number, forceHours = false): string {
  if (!seconds || isNaN(seconds) || seconds < 0) {
    return forceHours ? "00:00:00" : "00:00";
  }
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;

  const mm = m.toString().padStart(2, "0");
  const ss = s.toString().padStart(2, "0");

  if (h > 0 || forceHours) {
    const hh = h.toString().padStart(2, "0");
    return `${hh}:${mm}:${ss}`;
  }
  return `${mm}:${ss}`;
}

export function MediaPlayerView({ initialFile, initialTitle }: MediaPlayerProps) {
  const [filePath, setFilePath] = useState<string>(initialFile || "");
  const [videoTitle, setVideoTitle] = useState<string>(initialTitle || "");
  const [isPlaying, setIsPlaying] = useState<boolean>(false);
  const [currentTime, setCurrentTime] = useState<number>(0);
  const [duration, setDuration] = useState<number>(0);
  const [bufferedEnd, setBufferedEnd] = useState<number>(0);
  const [volume, setVolume] = useState<number>(1.0); // 0.0 ~ 2.0 (支持 200% 软音量增益)
  const [isMuted, setIsMuted] = useState<boolean>(false);
  const [playbackRate, setPlaybackRate] = useState<number>(1.0);
  const [isFullscreen, setIsFullscreen] = useState<boolean>(false);
  const [isAlwaysOnTop, setIsAlwaysOnTop] = useState<boolean>(false);
  const [isMiniMode, setIsMiniMode] = useState<boolean>(false);
  const [showControls, setShowControls] = useState<boolean>(true);
  const [toastMessage, setToastMessage] = useState<string>("");
  const [overlayFeedback, setOverlayFeedback] = useState<{ icon: string; text: string } | null>(null);

  // 播放列表与模式
  const [playlist, setPlaylist] = useState<PlaylistItem[]>([]);
  const [showPlaylist, setShowPlaylist] = useState<boolean>(false);
  const [playMode, setPlayMode] = useState<PlayMode>("loop");
  const [searchKeyword, setSearchKeyword] = useState<string>("");
  const [sortType, setSortType] = useState<"default" | "name-asc" | "name-desc" | "size-desc" | "size-asc">("default");
  const [showSortMenu, setShowSortMenu] = useState<boolean>(false);
  const activeItemRef = useRef<HTMLDivElement | null>(null);

  // 字幕状态与样式偏好 (持久化到 localStorage)
  const [subtitles, setSubtitles] = useState<SubtitleCue[]>([]);
  const [subtitleOffset, setSubtitleOffset] = useState<number>(0);
  const [subtitleName, setSubtitleName] = useState<string>("");
  const [matchedSubtitles, setMatchedSubtitles] = useState<SubtitleItem[]>([]);
  const [activeSubtitlePath, setActiveSubtitlePath] = useState<string>("");
  const [showSpeedMenu, setShowSpeedMenu] = useState<boolean>(false);
  const [showSubtitleMenu, setShowSubtitleMenu] = useState<boolean>(false);

  const [subStyle, setSubStyle] = useState<SubtitleStyleConfig>(() => {
    try {
      const saved = localStorage.getItem("maobu_player_sub_style");
      if (saved) {
        return JSON.parse(saved);
      }
    } catch {}
    return {
      fontSize: 20,
      color: "#ffffff",
      bgOpacity: 0.45,
      bottomOffset: 7,
    };
  });

  const updateSubStyle = useCallback((updater: (prev: SubtitleStyleConfig) => SubtitleStyleConfig) => {
    setSubStyle((prev) => {
      const next = updater(prev);
      try {
        localStorage.setItem("maobu_player_sub_style", JSON.stringify(next));
      } catch {}
      return next;
    });
  }, []);

  // 进度条 Hover 预览
  const [hoverProgress, setHoverProgress] = useState<{ x: number; time: number } | null>(null);

  const videoRef = useRef<HTMLVideoElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const subtitleInputRef = useRef<HTMLInputElement | null>(null);
  const hideControlsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const overlayTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const triggerToast = useCallback((msg: string) => {
    setToastMessage(msg);
    if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    toastTimerRef.current = setTimeout(() => setToastMessage(""), 2500);
  }, []);

  // 手动选择本地字幕文件导入
  const handleSelectSubtitleFile = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;

      const reader = new FileReader();
      reader.onload = (event) => {
        const content = event.target?.result as string;
        if (content) {
          const cues = parseAnySubtitles(content, file.name);
          setSubtitles(cues);
          setSubtitleName(file.name);
          setActiveSubtitlePath(file.name);

          // 加入已匹配字幕源列表
          setMatchedSubtitles((prev) => {
            const ext = file.name.split(".").pop() || "srt";
            const exists = prev.some((it) => it.name === file.name);
            if (!exists) {
              return [{ path: file.name, name: file.name, ext }, ...prev];
            }
            return prev;
          });

          triggerToast(`已导入外挂字幕：${file.name} (${cues.length} 条)`);
        }
      };
      reader.readAsText(file);
      e.target.value = "";
    },
    [triggerToast]
  );

  // 音频增益节点 (支持 200% 软音量)
  const audioCtxRef = useRef<AudioContext | null>(null);
  const gainNodeRef = useRef<GainNode | null>(null);

  // 自然数排序辅助
  const compareNatural = useCallback((a: string, b: string) => {
    return a.localeCompare(b, "zh-CN", { numeric: true, sensitivity: "base" });
  }, []);

  // 经过搜索与排序过滤后的展示列表
  const displayPlaylist = useMemo(() => {
    let list = [...playlist];
    if (searchKeyword.trim()) {
      const q = searchKeyword.toLowerCase().trim();
      list = list.filter((item) => item.name.toLowerCase().includes(q));
    }

    switch (sortType) {
      case "name-asc":
        list.sort((a, b) => compareNatural(a.name, b.name));
        break;
      case "name-desc":
        list.sort((a, b) => compareNatural(b.name, a.name));
        break;
      case "size-desc":
        list.sort((a, b) => (b.size_bytes || 0) - (a.size_bytes || 0));
        break;
      case "size-asc":
        list.sort((a, b) => (a.size_bytes || 0) - (b.size_bytes || 0));
        break;
      default:
        break;
    }
    return list;
  }, [playlist, searchKeyword, sortType, compareNatural]);

  // 定位到当前播放项
  const scrollToActiveItem = useCallback(() => {
    if (activeItemRef.current) {
      activeItemRef.current.scrollIntoView({ behavior: "smooth", block: "center" });
      triggerToast("已定位到当前播放视频");
    } else {
      triggerToast("当前视频不在列表中");
    }
  }, [triggerToast]);

  // 复制路径
  const copyItemPath = useCallback((e: React.MouseEvent, path: string) => {
    e.stopPropagation();
    navigator.clipboard.writeText(path).then(() => {
      triggerToast("已复制视频完整路径");
    }).catch(() => {
      triggerToast("复制失败");
    });
  }, [triggerToast]);

  const triggerOverlay = useCallback((icon: string, text: string) => {
    setOverlayFeedback({ icon, text });
    if (overlayTimerRef.current) clearTimeout(overlayTimerRef.current);
    overlayTimerRef.current = setTimeout(() => setOverlayFeedback(null), 850);
  }, []);

  // 自动载入视频所在目录的所有视频构建播放列表
  const refreshFolderPlaylist = useCallback(async (targetPath: string) => {
    if (!targetPath) return;
    if (isDesktop()) {
      try {
        const items = await api.playerGetFolderVideos(targetPath);
        if (items && items.length > 0) {
          setPlaylist(items);
        }
      } catch (e) {
        console.error("Scan folder videos error:", e);
      }
    } else {
      setPlaylist((prev) => {
        if (!prev.some((it) => it.path === targetPath)) {
          return [...prev, { path: targetPath, name: targetPath.split(/[\\/]/).pop() || "视频", size_bytes: 0 }];
        }
        return prev;
      });
    }
  }, []);

  // 载入特定字幕文件内容并解析
  const loadSubtitleFile = useCallback(async (subItem: SubtitleItem, notify = true) => {
    try {
      const text = await api.playerReadSubtitleContent(subItem.path);
      if (text) {
        const cues = parseAnySubtitles(text, subItem.ext);
        setSubtitles(cues);
        setSubtitleName(subItem.name);
        setActiveSubtitlePath(subItem.path);
        if (notify) {
          triggerToast(`已加载外挂字幕: ${subItem.name} (${cues.length} 条)`);
        }
      }
    } catch (err) {
      if (notify) {
        triggerToast(`读取字幕失败: ${err}`);
      }
    }
  }, [triggerToast]);

  // 刷新同目录匹配字幕
  const refreshMatchedSubtitles = useCallback(async (videoPath: string) => {
    if (!isDesktop() || !videoPath) return;
    try {
      const list = await api.playerGetMatchedSubtitles(videoPath);
      setMatchedSubtitles(list);
      if (list.length > 0) {
        // 自动挂载第一个最佳匹配字幕
        await loadSubtitleFile(list[0], false);
      }
    } catch (err) {
      console.error("Failed to scan subtitles:", err);
    }
  }, [loadSubtitleFile]);

  // 加载指定视频并播放
  const loadAndPlayVideo = useCallback((path: string, title?: string) => {
    setFilePath(path);
    setVideoTitle(title || path.split(/[\\/]/).pop() || "视频播放");
    setSubtitles([]);
    setSubtitleName("");
    setActiveSubtitlePath("");
    setCurrentTime(0);
    setBufferedEnd(0);
    refreshFolderPlaylist(path);
    refreshMatchedSubtitles(path);
  }, [refreshFolderPlaylist, refreshMatchedSubtitles]);

  // 初始加载：综合读取 URL Query 与 Rust 后端 PlayerState
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const urlFile = params.get("file");
    const urlTitle = params.get("title");
    if (urlFile) {
      loadAndPlayVideo(urlFile, urlTitle || undefined);
    }

    if (isDesktop()) {
      api.playerGetCurrentFile().then((res) => {
        if (res && res[0]) {
          loadAndPlayVideo(res[0], res[1] || undefined);
        }
      }).catch(console.error);
    }
  }, [loadAndPlayVideo]);

  // 监听来自外部或主窗口的播放事件
  useEffect(() => {
    if (!isDesktop()) return;
    const unlistenPromise = listen<{ file: string; title?: string }>("player-load-file", (event) => {
      if (event.payload?.file) {
        loadAndPlayVideo(event.payload.file, event.payload.title);
        triggerToast("已加载新视频");
      }
    });
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, [loadAndPlayVideo, triggerToast]);

  // 初始化 Web Audio API 增益节点
  const initAudioGain = useCallback(() => {
    if (audioCtxRef.current || !videoRef.current) return;
    try {
      const AudioCtx = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      const ctx = new AudioCtx();
      const source = ctx.createMediaElementSource(videoRef.current);
      const gainNode = ctx.createGain();
      gainNode.gain.value = volume;
      source.connect(gainNode);
      gainNode.connect(ctx.destination);
      audioCtxRef.current = ctx;
      gainNodeRef.current = gainNode;
    } catch {
      // 忽略部分策略报错
    }
  }, [volume]);

  // 更新音量与增益
  useEffect(() => {
    if (gainNodeRef.current) {
      gainNodeRef.current.gain.value = isMuted ? 0 : volume;
    } else if (videoRef.current) {
      videoRef.current.volume = isMuted ? 0 : Math.min(1.0, volume);
    }
  }, [volume, isMuted]);

  // 记忆播放与自动续播
  const handleLoadedMetadata = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    const dur = video.duration;
    setDuration(dur);

    if (filePath) {
      const resumeKey = `maobu_resume_${filePath}`;
      const savedTime = parseFloat(localStorage.getItem(resumeKey) || "0");
      if (savedTime > 5 && savedTime < dur - 5) {
        video.currentTime = savedTime;
        triggerToast(`已从上次记忆位置 ${formatDuration(Math.floor(savedTime))} 继续`);
      }
    }

    video.play().then(() => setIsPlaying(true)).catch(() => setIsPlaying(false));
  }, [filePath, triggerToast]);

  // 定时记录播放进度
  const handleTimeUpdate = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    const ct = video.currentTime;
    setCurrentTime(ct);

    if (video.buffered.length > 0) {
      setBufferedEnd(video.buffered.end(video.buffered.length - 1));
    }

    if (filePath && ct > 3) {
      localStorage.setItem(`maobu_resume_${filePath}`, ct.toString());
    }
  }, [filePath]);

  // 控制栏自动隐藏
  const resetHideControlsTimer = useCallback(() => {
    setShowControls(true);
    if (hideControlsTimerRef.current) clearTimeout(hideControlsTimerRef.current);
    hideControlsTimerRef.current = setTimeout(() => {
      if (isPlaying && !showPlaylist) {
        setShowControls(false);
        setShowSpeedMenu(false);
        setShowSubtitleMenu(false);
      }
    }, 2500);
  }, [isPlaying, showPlaylist]);

  // 播放/暂停切换
  const togglePlay = useCallback(() => {
    initAudioGain();
    const video = videoRef.current;
    if (!video) return;
    if (video.paused) {
      video.play().then(() => {
        setIsPlaying(true);
        triggerOverlay("play", "播放");
      }).catch((e) => {
        triggerToast(`无法播放: ${e.message || '格式不支持'}`);
      });
    } else {
      video.pause();
      setIsPlaying(false);
      triggerOverlay("pause", "暂停");
    }
    resetHideControlsTimer();
  }, [initAudioGain, resetHideControlsTimer, triggerOverlay, triggerToast]);

  // 快进/快退 (方向键保留 5s)
  const seekRelative = useCallback((seconds: number) => {
    const video = videoRef.current;
    if (!video) return;
    const target = Math.max(0, Math.min(video.duration || 0, video.currentTime + seconds));
    video.currentTime = target;
    triggerOverlay(seconds > 0 ? "forward" : "rewind", `${seconds > 0 ? "+" : ""}${seconds}s`);
    resetHideControlsTimer();
  }, [resetHideControlsTimer, triggerOverlay]);

  // 切换上一视频
  const playPrevVideo = useCallback(() => {
    if (playlist.length === 0) {
      if (videoRef.current) {
        videoRef.current.currentTime = 0;
        videoRef.current.play();
      }
      return;
    }

    const currentIndex = playlist.findIndex((it) => it.path === filePath);
    let targetIndex = currentIndex - 1;

    if (targetIndex < 0) {
      if (playMode === "loop") {
        targetIndex = playlist.length - 1;
      } else {
        triggerToast("已是播放列表第一个视频");
        return;
      }
    }

    const targetItem = playlist[targetIndex];
    if (targetItem) {
      loadAndPlayVideo(targetItem.path, targetItem.name);
      triggerToast(`上一视频: ${targetItem.name}`);
      triggerOverlay("prev", "上一视频");
    }
  }, [filePath, loadAndPlayVideo, playMode, playlist, triggerOverlay, triggerToast]);

  // 切换下一视频
  const playNextVideo = useCallback((autoTrigger = false) => {
    if (playlist.length === 0) {
      if (videoRef.current) {
        videoRef.current.currentTime = 0;
        videoRef.current.play();
      }
      return;
    }

    if (autoTrigger && playMode === "single") {
      if (videoRef.current) {
        videoRef.current.currentTime = 0;
        videoRef.current.play();
      }
      return;
    }

    const currentIndex = playlist.findIndex((it) => it.path === filePath);
    let targetIndex = 0;

    if (playMode === "shuffle") {
      if (playlist.length > 1) {
        do {
          targetIndex = Math.floor(Math.random() * playlist.length);
        } while (targetIndex === currentIndex);
      }
    } else {
      targetIndex = currentIndex + 1;
      if (targetIndex >= playlist.length) {
        if (playMode === "loop") {
          targetIndex = 0;
        } else {
          if (autoTrigger) {
            setIsPlaying(false);
            triggerToast("已播放完列表全部视频");
          } else {
            triggerToast("已是播放列表最后一个视频");
          }
          return;
        }
      }
    }

    const targetItem = playlist[targetIndex];
    if (targetItem) {
      loadAndPlayVideo(targetItem.path, targetItem.name);
      triggerToast(`下一视频: ${targetItem.name}`);
      triggerOverlay("next", "下一视频");
    }
  }, [filePath, loadAndPlayVideo, playMode, playlist, triggerOverlay, triggerToast]);

  // 循环切换播放模式
  const cyclePlayMode = useCallback(() => {
    const modes: PlayMode[] = ["loop", "sequence", "single", "shuffle"];
    const currentIdx = modes.indexOf(playMode);
    const nextMode = modes[(currentIdx + 1) % modes.length];
    setPlayMode(nextMode);

    const labels: Record<PlayMode, string> = {
      loop: "列表循环",
      sequence: "顺序播放",
      single: "单曲循环",
      shuffle: "随机播放",
    };
    triggerOverlay(nextMode, labels[nextMode]);
    triggerToast(`播放模式: ${labels[nextMode]}`);
  }, [playMode, triggerOverlay, triggerToast]);

  // 调整音量
  const adjustVolume = useCallback((delta: number) => {
    initAudioGain();
    setVolume((prev) => {
      const next = Math.max(0, Math.min(2.0, parseFloat((prev + delta).toFixed(2))));
      triggerOverlay("volume", `音量 ${Math.round(next * 100)}%`);
      return next;
    });
    setIsMuted(false);
    resetHideControlsTimer();
  }, [initAudioGain, resetHideControlsTimer, triggerOverlay]);

  // 调整倍速
  const changePlaybackRate = useCallback((rate: number) => {
    const video = videoRef.current;
    if (!video) return;
    video.playbackRate = rate;
    setPlaybackRate(rate);
    setShowSpeedMenu(false);
    triggerOverlay("speed", `${rate}x`);
    resetHideControlsTimer();
  }, [resetHideControlsTimer, triggerOverlay]);

  // 切换全屏
  const toggleFullscreen = useCallback(async () => {
    if (isDesktop()) {
      try {
        const next = await api.playerWindowToggleFullscreen();
        setIsFullscreen(next);
      } catch (err) {
        console.error("Fullscreen error:", err);
      }
    } else if (containerRef.current) {
      if (!document.fullscreenElement) {
        await containerRef.current.requestFullscreen();
        setIsFullscreen(true);
      } else {
        await document.exitFullscreen();
        setIsFullscreen(false);
      }
    }
    resetHideControlsTimer();
  }, [resetHideControlsTimer]);

  const clickTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const drawerRef = useRef<HTMLDivElement | null>(null);
  const playlistBtnRef = useRef<HTMLButtonElement | null>(null);
  const justClosedDrawerRef = useRef<number>(0);

  // 点击外部自动关闭播放列表抽屉与排序菜单
  useEffect(() => {
    if (!showPlaylist) return;

    const handlePointerDownOutside = (e: MouseEvent | TouchEvent) => {
      const target = e.target as Node | null;
      if (!target) return;

      // 如果点击在抽屉内部或打开播放列表的按钮上，不关闭
      if (drawerRef.current && drawerRef.current.contains(target)) {
        return;
      }
      if (playlistBtnRef.current && playlistBtnRef.current.contains(target)) {
        return;
      }

      justClosedDrawerRef.current = Date.now();
      setShowPlaylist(false);
      setShowSortMenu(false);
    };

    document.addEventListener("mousedown", handlePointerDownOutside, true);
    return () => {
      document.removeEventListener("mousedown", handlePointerDownOutside, true);
    };
  }, [showPlaylist]);

  // 单击画面播放/暂停 (防抖区分单击与双击，若播放列表刚关闭或正打开则仅收起，坚决不暂停视频)
  const handleStageClick = useCallback(() => {
    const isJustClosed = Date.now() - justClosedDrawerRef.current < 400;
    if (showPlaylist || isJustClosed) {
      setShowPlaylist(false);
      setShowSortMenu(false);
      return;
    }
    if (clickTimeoutRef.current) {
      clearTimeout(clickTimeoutRef.current);
      clickTimeoutRef.current = null;
    }
    clickTimeoutRef.current = setTimeout(() => {
      togglePlay();
      clickTimeoutRef.current = null;
    }, 220);
  }, [showPlaylist, togglePlay]);

  // 双击画面全屏/最大化 (立即取消单击避免误触播放与HUD)
  const handleStageDoubleClick = useCallback(() => {
    if (clickTimeoutRef.current) {
      clearTimeout(clickTimeoutRef.current);
      clickTimeoutRef.current = null;
    }
    toggleFullscreen();
  }, [toggleFullscreen]);

  // 切换窗口置顶
  const toggleAlwaysOnTop = useCallback(async () => {
    const next = !isAlwaysOnTop;
    try {
      await api.playerWindowSetAlwaysOnTop(next);
      setIsAlwaysOnTop(next);
      triggerToast(next ? "窗口已置顶" : "已取消置顶");
    } catch (err) {
      triggerToast(`置顶设置失败: ${err}`);
    }
  }, [isAlwaysOnTop, triggerToast]);

  // 最小化窗口
  const handleMinimize = useCallback(() => {
    api.playerWindowMinimize().catch(console.error);
  }, []);

  // 最大化/还原窗口
  const handleToggleMaximize = useCallback(() => {
    api.playerWindowToggleMaximize().then(setIsFullscreen).catch(console.error);
  }, []);

  // 关闭窗口
  const handleClose = useCallback(() => {
    api.playerWindowClose().catch(console.error);
  }, []);

  // 画中画 (桌面端切换原生无边框迷你置顶浮窗，Web端回退标准PiP)
  const togglePiP = useCallback(async () => {
    if (isDesktop()) {
      try {
        const next = !isMiniMode;
        await api.playerWindowToggleMiniMode(next);
        setIsMiniMode(next);
        setIsAlwaysOnTop(next);
        triggerToast(next ? "已切换为桌面画中画迷你置顶窗口 (按 P 或右上角还原)" : "已还原正常播放窗口");
      } catch (err) {
        console.error("Mini mode error:", err);
      }
    } else {
      const video = videoRef.current;
      if (!video) return;
      try {
        if (document.pictureInPictureElement) {
          await document.exitPictureInPicture();
        } else if (document.pictureInPictureEnabled) {
          await video.requestPictureInPicture();
        }
      } catch {
        triggerToast("画中画不可用");
      }
    }
  }, [isMiniMode, triggerToast]);

  // 视频截屏 (默认保存至下载目录并复制到剪贴板)
  const captureScreenshot = useCallback(async () => {
    const video = videoRef.current;
    if (!video) return;
    try {
      const canvas = document.createElement("canvas");
      canvas.width = video.videoWidth || 1920;
      canvas.height = video.videoHeight || 1080;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.drawImage(video, 0, 0, canvas.width, canvas.height);

      const dataUrl = canvas.toDataURL("image/png");

      if (isDesktop()) {
        try {
          const savedPath = await api.playerSaveScreenshot(dataUrl, videoTitle, currentTime, filePath);
          const fname = savedPath.split(/[\\/]/).pop() || "截图.png";
          triggerToast(`截图已保存: ${fname} (已复制到剪贴板)`);
        } catch (err) {
          triggerToast(`截图文件写入失败: ${err}`);
        }
      } else {
        const a = document.createElement("a");
        a.download = `screenshot_${Date.now()}.png`;
        a.href = dataUrl;
        a.click();
        triggerToast("已保存截图");
      }

      canvas.toBlob(async (blob) => {
        if (!blob) return;
        try {
          const item = new ClipboardItem({ "image/png": blob });
          await navigator.clipboard.write([item]);
        } catch {
          // ignore fallback
        }
      });
    } catch (err) {
      console.error("Screenshot error:", err);
      triggerToast(`截图失败: ${err instanceof Error ? err.message : String(err)}`);
    }
  }, [currentTime, filePath, triggerToast, videoTitle]);

  // 外部播放器打开
  const openWithExternalPlayer = useCallback(() => {
    if (filePath) {
      api.openPathDirect(filePath).catch((err) => triggerToast(`调用外部播放器失败: ${err}`));
    }
  }, [filePath, triggerToast]);

  // 手动添加视频到播放列表
  const handleAddFiles = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files || files.length === 0) return;
    const newItems: PlaylistItem[] = [];
    for (let i = 0; i < files.length; i++) {
      const f = files[i];
      const p = (f as unknown as { path?: string }).path || f.name;
      newItems.push({
        path: p,
        name: f.name,
        size_bytes: f.size,
      });
    }
    setPlaylist((prev) => {
      const map = new Map<string, PlaylistItem>();
      prev.forEach((it) => map.set(it.path, it));
      newItems.forEach((it) => map.set(it.path, it));
      return Array.from(map.values());
    });
    triggerToast(`已添加 ${newItems.length} 个视频到播放列表`);
  }, [triggerToast]);

  // 移除列表单项
  const handleRemovePlaylistItem = useCallback((e: React.MouseEvent, itemPath: string) => {
    e.stopPropagation();
    setPlaylist((prev) => prev.filter((it) => it.path !== itemPath));
    triggerToast("已从列表移除");
  }, [triggerToast]);

  // 键盘快捷键响应
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;

      switch (e.key) {
        case " ":
        case "k":
        case "K":
          e.preventDefault();
          togglePlay();
          break;
        case "ArrowLeft":
          e.preventDefault();
          if (e.shiftKey) {
            playPrevVideo();
          } else {
            seekRelative(e.ctrlKey ? -1 : -5);
          }
          break;
        case "ArrowRight":
          e.preventDefault();
          if (e.shiftKey) {
            playNextVideo();
          } else {
            seekRelative(e.ctrlKey ? 1 : 5);
          }
          break;
        case "PageUp":
          e.preventDefault();
          playPrevVideo();
          break;
        case "PageDown":
          e.preventDefault();
          playNextVideo();
          break;
        case "ArrowUp":
          e.preventDefault();
          adjustVolume(0.05);
          break;
        case "ArrowDown":
          e.preventDefault();
          adjustVolume(-0.05);
          break;
        case "m":
        case "M":
          e.preventDefault();
          setIsMuted((prev) => !prev);
          triggerOverlay("mute", !isMuted ? "已静音" : "已恢复音量");
          break;
        case "f":
        case "F":
        case "Enter":
          e.preventDefault();
          toggleFullscreen();
          break;
        case "p":
        case "P":
          e.preventDefault();
          togglePiP();
          break;
        case "s":
        case "S":
          e.preventDefault();
          captureScreenshot();
          break;
        case "l":
        case "L":
          e.preventDefault();
          setShowPlaylist((prev) => !prev);
          break;
        case "[":
          e.preventDefault();
          changePlaybackRate(Math.max(0.5, parseFloat((playbackRate - 0.25).toFixed(2))));
          break;
        case "]":
          e.preventDefault();
          changePlaybackRate(Math.min(3.0, parseFloat((playbackRate + 0.25).toFixed(2))));
          break;
        case "Escape":
          if (showPlaylist) {
            e.preventDefault();
            setShowPlaylist(false);
            setShowSortMenu(false);
          } else if (showSpeedMenu || showSubtitleMenu) {
            e.preventDefault();
            setShowSpeedMenu(false);
            setShowSubtitleMenu(false);
          } else if (isFullscreen) {
            e.preventDefault();
            toggleFullscreen();
          }
          break;
        default:
          if (e.key >= "0" && e.key <= "9") {
            const pct = parseInt(e.key, 10) / 10;
            if (videoRef.current && duration > 0) {
              videoRef.current.currentTime = duration * pct;
              triggerOverlay("seek", `${pct * 100}%`);
            }
          }
          break;
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    adjustVolume,
    captureScreenshot,
    changePlaybackRate,
    duration,
    isMuted,
    playNextVideo,
    playPrevVideo,
    playbackRate,
    seekRelative,
    toggleFullscreen,
    togglePiP,
    togglePlay,
    triggerOverlay,
  ]);

  // 拖拽视频文件或字幕载入
  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const files = Array.from(e.dataTransfer.files);
      if (files.length === 0) return;

      const videoExts = [".mp4", ".mkv", ".webm", ".avi", ".mov", ".flv", ".wmv", ".ts", ".m4v", ".rmvb"];
      const droppedVideos: PlaylistItem[] = [];

      for (const file of files) {
        const name = file.name.toLowerCase();
        if (name.endsWith(".srt") || name.endsWith(".vtt") || name.endsWith(".ass") || name.endsWith(".ssa") || name.endsWith(".txt")) {
          const reader = new FileReader();
          reader.onload = (event) => {
            const content = event.target?.result as string;
            if (content) {
              const cues = parseAnySubtitles(content, file.name);
              setSubtitles(cues);
              setSubtitleName(file.name);
              triggerToast(`已加载外挂字幕：${file.name} (${cues.length} 条)`);
            }
          };
          reader.readAsText(file);
        } else if (videoExts.some((ext) => name.endsWith(ext))) {
          const p = (file as unknown as { path?: string }).path || file.name;
          droppedVideos.push({
            path: p,
            name: file.name,
            size_bytes: file.size,
          });
        }
      }

      if (droppedVideos.length > 0) {
        setPlaylist((prev) => {
          const map = new Map<string, PlaylistItem>();
          prev.forEach((it) => map.set(it.path, it));
          droppedVideos.forEach((it) => map.set(it.path, it));
          return Array.from(map.values());
        });
        loadAndPlayVideo(droppedVideos[0].path, droppedVideos[0].name);
        triggerToast(`已添加并播放：${droppedVideos[0].name}`);
      }
    },
    [loadAndPlayVideo, triggerToast]
  );

  // 计算当前字幕文本
  const currentSubtitleText = getActiveCueText(subtitles, currentTime, subtitleOffset);

  // 构造流播放 URL
  const videoSrc = filePath
    ? (isDesktop() ? convertFileSrc(filePath) : filePath)
    : "";

  const renderModeIcon = () => {
    switch (playMode) {
      case "loop":
        return <Repeat size={12} />;
      case "single":
        return <Repeat1 size={12} />;
      case "shuffle":
        return <Shuffle size={12} />;
      case "sequence":
      default:
        return <Play size={12} />;
    }
  };

  const modeLabel: Record<PlayMode, string> = {
    loop: "列表循环",
    sequence: "顺序播放",
    single: "单曲循环",
    shuffle: "随机播放",
  };

  return (
    <div
      ref={containerRef}
      className={`maobu-player-root ${isMiniMode ? "mini-mode" : ""} ${!showControls && isPlaying && !showPlaylist ? "hide-cursor" : ""}`}
      onMouseMove={resetHideControlsTimer}
      onClick={resetHideControlsTimer}
      onDragOver={(e) => e.preventDefault()}
      onDrop={handleDrop}
    >
      {/* 隐藏的本地视频导入 input */}
      <input
        ref={fileInputRef}
        type="file"
        multiple
        accept="video/*"
        style={{ display: "none" }}
        onChange={handleAddFiles}
      />

      {/* 隐藏的本地字幕导入 input */}
      <input
        ref={subtitleInputRef}
        type="file"
        accept=".srt,.vtt,.ass,.ssa,.txt"
        style={{ display: "none" }}
        onChange={handleSelectSubtitleFile}
      />

      {/* 顶部自定义精简微晶标题栏 */}
      <div
        data-tauri-drag-region
        className={`maobu-player-titlebar ${!showControls && !showPlaylist ? "hidden" : ""}`}
      >
        <div className="maobu-player-title-info" data-tauri-drag-region>
          <Film className="maobu-player-title-icon" />
          <span className="maobu-player-title-text" data-tauri-drag-region>
            {videoTitle || "猫步播放器 · Maobu Player"}
          </span>
        </div>

        <div className="maobu-player-window-controls" data-tauri-drag-region="false">
          <button
            type="button"
            data-tauri-drag-region="false"
            onClick={toggleAlwaysOnTop}
            title={isAlwaysOnTop ? "取消置顶" : "置顶窗口"}
            className={`maobu-player-icon-btn ${isAlwaysOnTop ? "active" : ""}`}
          >
            {isAlwaysOnTop ? <Pin size={14} /> : <PinOff size={14} />}
          </button>
          {isDesktop() && (
            <>
              <button
                type="button"
                data-tauri-drag-region="false"
                onClick={handleMinimize}
                title="最小化"
                className="maobu-player-icon-btn"
              >
                <Minus size={14} />
              </button>
              <button
                type="button"
                data-tauri-drag-region="false"
                onClick={handleToggleMaximize}
                title={isFullscreen ? "还原" : "最大化"}
                className="maobu-player-icon-btn"
              >
                {isFullscreen ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
              </button>
              <button
                type="button"
                data-tauri-drag-region="false"
                onClick={handleClose}
                title="关闭"
                className="maobu-player-icon-btn close-btn"
              >
                <X size={14} />
              </button>
            </>
          )}
        </div>
      </div>

      {/* 核心视频播放区域 */}
      <div
        className="maobu-player-stage"
        onClick={handleStageClick}
        onDoubleClick={handleStageDoubleClick}
      >
        {videoSrc ? (
          <video
            ref={videoRef}
            src={videoSrc}
            crossOrigin="anonymous"
            className="maobu-player-video"
            onLoadedMetadata={handleLoadedMetadata}
            onTimeUpdate={handleTimeUpdate}
            onEnded={() => playNextVideo(true)}
            onPlay={() => setIsPlaying(true)}
            onPause={() => setIsPlaying(false)}
            onError={(e) => {
              const err = (e.currentTarget as HTMLVideoElement).error;
              console.error("Video load error:", err, "src:", videoSrc);
              triggerToast(err ? `视频加载异常 (代码 ${err.code}): 格式或编解码器限制` : "视频加载失败");
            }}
            preload="auto"
            playsInline
          />
        ) : (
          <div className="maobu-player-empty-state">
            <Film className="maobu-player-empty-icon" />
            <p style={{ fontSize: "14px", margin: 0 }}>暂无可播放的视频源</p>
          </div>
        )}

        {/* 动态字幕渲染层 (应用字号、色彩、底衬与垂直位置) */}
        {currentSubtitleText && (
          <div
            className={`maobu-player-subtitles ${subStyle.bgOpacity > 0 ? "has-backdrop" : ""}`}
            style={{
              fontSize: `${subStyle.fontSize}px`,
              color: subStyle.color,
              bottom: `${subStyle.bottomOffset}%`,
              backgroundColor: subStyle.bgOpacity > 0 ? `rgba(0, 0, 0, ${subStyle.bgOpacity})` : "transparent",
            }}
          >
            {currentSubtitleText}
          </div>
        )}

        {/* 快捷键操作中央 HUD 反馈 (极简通透 Apple 胶囊) */}
        {overlayFeedback && (
          <div className="maobu-player-hud">
            {overlayFeedback.icon === "play" && <Play className="maobu-player-hud-icon" style={{ fill: "#ffffff" }} />}
            {overlayFeedback.icon === "pause" && <Pause className="maobu-player-hud-icon" style={{ fill: "#ffffff" }} />}
            {overlayFeedback.icon === "prev" && <SkipBack className="maobu-player-hud-icon" />}
            {overlayFeedback.icon === "next" && <SkipForward className="maobu-player-hud-icon" />}
            {overlayFeedback.icon === "forward" && <FastForward className="maobu-player-hud-icon" />}
            {overlayFeedback.icon === "rewind" && <Rewind className="maobu-player-hud-icon" />}
            {overlayFeedback.icon === "volume" && <Volume2 className="maobu-player-hud-icon" />}
            {overlayFeedback.icon === "mute" && <VolumeX className="maobu-player-hud-icon" style={{ color: "#f87171" }} />}
            {overlayFeedback.icon === "speed" && <Sliders className="maobu-player-hud-icon" />}
            {overlayFeedback.icon === "loop" && <Repeat className="maobu-player-hud-icon" />}
            {overlayFeedback.icon === "single" && <Repeat1 className="maobu-player-hud-icon" />}
            {overlayFeedback.icon === "shuffle" && <Shuffle className="maobu-player-hud-icon" />}
            <span>{overlayFeedback.text}</span>
          </div>
        )}
      </div>

      {/* 底部浮动控制栏面板 */}
      <div
        className={`maobu-player-controls-panel ${!showControls && !showPlaylist ? "hidden" : ""}`}
        onClick={(e) => e.stopPropagation()}
      >
        {/* 进度条与 Hover 预览 */}
        <div
          className="maobu-player-progress-wrap"
          onClick={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            const pos = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
            if (videoRef.current && duration > 0) {
              videoRef.current.currentTime = duration * pos;
            }
          }}
          onMouseMove={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            const pos = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
            setHoverProgress({ x: e.clientX, time: pos * duration });
          }}
          onMouseLeave={() => setHoverProgress(null)}
        >
          <div className="maobu-player-progress-rail">
            <div
              className="maobu-player-progress-buffered"
              style={{ width: `${(bufferedEnd / (duration || 1)) * 100}%` }}
            />
            <div
              className="maobu-player-progress-played"
              style={{ width: `${(currentTime / (duration || 1)) * 100}%` }}
            />
          </div>

          <div
            className="maobu-player-progress-thumb"
            style={{ left: `calc(${(currentTime / (duration || 1)) * 100}% - 6px)` }}
          />

          {hoverProgress && (
            <div
              className="maobu-player-progress-tooltip"
              style={{ left: hoverProgress.x }}
            >
              {formatPlayerTime(hoverProgress.time, duration >= 3600)}
            </div>
          )}
        </div>

        {/* 底部按钮栏两端布局 */}
        <div className="maobu-player-buttons-row">
          {/* 左侧控制：上一视频、播放/暂停、下一视频、时间 */}
          <div className="maobu-player-left-group">
            <button
              type="button"
              onClick={playPrevVideo}
              title="上一个视频 (PageUp / Shift+←)"
              className="maobu-player-icon-btn"
            >
              <SkipBack size={16} />
            </button>

            <button
              type="button"
              onClick={togglePlay}
              title={isPlaying ? "暂停 (Space)" : "播放 (Space)"}
              className="maobu-player-main-play-btn"
            >
              {isPlaying ? (
                <Pause size={16} style={{ fill: "#ffffff" }} />
              ) : (
                <Play size={16} style={{ fill: "#ffffff", marginLeft: "2px" }} />
              )}
            </button>

            <button
              type="button"
              onClick={() => playNextVideo(false)}
              title="下一个视频 (PageDown / Shift+→)"
              className="maobu-player-icon-btn"
            >
              <SkipForward size={16} />
            </button>

            <div className="maobu-player-time-display">
              <span className="maobu-player-time-current">
                {formatPlayerTime(currentTime, duration >= 3600)}
              </span>
              <span className="maobu-player-time-divider">/</span>
              <span className="maobu-player-time-total">
                {formatPlayerTime(duration, duration >= 3600)}
              </span>
            </div>
          </div>

          {/* 右侧控制：音量、倍速、字幕、截图、画中画、播放列表、外部打开、全屏 */}
          <div className="maobu-player-right-group" style={{ position: "relative" }}>
            {/* 音量控制组 */}
            <div className="maobu-player-volume-group">
              <button
                type="button"
                onClick={() => setIsMuted((prev) => !prev)}
                title={isMuted ? "恢复音量 (M)" : "静音 (M)"}
                className="maobu-player-icon-btn"
              >
                {isMuted || volume === 0 ? <VolumeX size={16} style={{ color: "#ef4444" }} /> : <Volume2 size={16} />}
              </button>
              <input
                type="range"
                min="0"
                max="2"
                step="0.05"
                value={isMuted ? 0 : volume}
                onChange={(e) => {
                  initAudioGain();
                  setVolume(parseFloat(e.target.value));
                  setIsMuted(false);
                }}
                className="maobu-player-volume-slider"
                title={`音量: ${Math.round((isMuted ? 0 : volume) * 100)}%`}
              />
              <span className="maobu-player-volume-label">
                {Math.round((isMuted ? 0 : volume) * 100)}%
              </span>
            </div>

            {/* 倍速选择器 */}
            <div style={{ position: "relative" }}>
              <button
                type="button"
                onClick={() => {
                  setShowSpeedMenu((prev) => !prev);
                  setShowSubtitleMenu(false);
                }}
                className="maobu-player-icon-btn"
                style={{ fontSize: "12px", fontFamily: "monospace", padding: "4px 8px" }}
                title="播放速度 ([ / ])"
              >
                {playbackRate === 1.0 ? "倍速" : `${playbackRate}x`}
              </button>
              {showSpeedMenu && (
                <div className="maobu-player-popup-menu">
                  {[0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0].map((rate) => (
                    <button
                      key={rate}
                      type="button"
                      onClick={() => changePlaybackRate(rate)}
                      className={`maobu-player-menu-item ${playbackRate === rate ? "active" : ""}`}
                    >
                      <span>{rate}x</span>
                      {playbackRate === rate && <Check size={12} />}
                    </button>
                  ))}
                </div>
              )}
            </div>

            {/* 字幕菜单 */}
            <div style={{ position: "relative" }}>
              <button
                type="button"
                onClick={() => {
                  setShowSubtitleMenu((prev) => !prev);
                  setShowSpeedMenu(false);
                }}
                title="字幕设置"
                className={`maobu-player-icon-btn ${subtitles.length > 0 ? "active" : ""}`}
              >
                <Subtitles size={16} />
              </button>
              {showSubtitleMenu && (
                <div
                  className="maobu-player-popup-menu maobu-player-subtitle-panel"
                  onClick={(e) => e.stopPropagation()}
                  onMouseDown={(e) => e.stopPropagation()}
                >
                  {/* 头部标题与操作 */}
                  <div className="maobu-player-subtitle-header">
                    <div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
                      <Subtitles size={14} style={{ color: "#38bdf8" }} />
                      <span style={{ fontWeight: 600, color: "#f1f5f9" }}>字幕设置</span>
                      {subtitles.length > 0 && (
                        <span className="maobu-player-sub-badge">{subtitles.length} 条</span>
                      )}
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
                      <button
                        type="button"
                        onClick={() => subtitleInputRef.current?.click()}
                        className="maobu-player-sub-import-btn"
                        title="选择本地字幕文件导入 (.srt / .vtt / .ass / .ssa / .txt)"
                      >
                        <Plus size={11} style={{ marginRight: "2px" }} />
                        导入字幕
                      </button>
                      {subtitleName && (
                        <button
                          type="button"
                          onClick={() => {
                            setSubtitles([]);
                            setSubtitleName("");
                            setActiveSubtitlePath("");
                            triggerToast("已关闭外挂字幕");
                          }}
                          className="maobu-player-sub-remove-btn"
                          title="关闭外挂字幕"
                        >
                          关闭
                        </button>
                      )}
                    </div>
                  </div>

                  {/* 发现的同名/匹配字幕列表 */}
                  {matchedSubtitles.length > 0 ? (
                    <div className="maobu-player-subtitle-section">
                      <div className="maobu-player-subtitle-section-title">
                        <span>字幕源选择 ({matchedSubtitles.length})</span>
                      </div>
                      <div className="maobu-player-subtitle-list">
                        {matchedSubtitles.map((sub) => {
                          const isActive = activeSubtitlePath === sub.path || subtitleName === sub.name;
                          return (
                            <button
                              key={sub.path}
                              type="button"
                              onClick={() => loadSubtitleFile(sub)}
                              className={`maobu-player-subtitle-item ${isActive ? "active" : ""}`}
                              title={sub.path}
                            >
                              <span className="maobu-player-subtitle-item-ext">{sub.ext.toUpperCase()}</span>
                              <span className="maobu-player-subtitle-item-name">{sub.name}</span>
                              {isActive && <Check size={12} style={{ color: "#38bdf8", flexShrink: 0 }} />}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  ) : (
                    <div
                      className="maobu-player-subtitle-empty-box"
                      onClick={() => subtitleInputRef.current?.click()}
                      title="点击选择本地字幕文件"
                    >
                      <FolderOpen size={14} style={{ color: "#38bdf8", marginBottom: "4px" }} />
                      <span style={{ fontSize: "11px", color: "#cbd5e1" }}>点击选择本地字幕文件</span>
                      <span style={{ fontSize: "10px", color: "#64748b" }}>支持 .srt / .vtt / .ass / .txt 或直接拖入</span>
                    </div>
                  )}

                  {/* 字号大小调节 */}
                  <div className="maobu-player-subtitle-section">
                    <div className="maobu-player-subtitle-section-title">
                      <span>字号大小</span>
                      <span style={{ fontFamily: "monospace", color: "#38bdf8" }}>{subStyle.fontSize}px</span>
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                      <span style={{ fontSize: "11px", color: "#64748b" }}>小</span>
                      <input
                        type="range"
                        min="14"
                        max="36"
                        step="1"
                        value={subStyle.fontSize}
                        onChange={(e) => {
                          const v = parseInt(e.target.value, 10) || 20;
                          updateSubStyle((prev) => ({ ...prev, fontSize: v }));
                        }}
                        className="maobu-player-sub-slider"
                      />
                      <span style={{ fontSize: "11px", color: "#64748b" }}>大</span>
                    </div>
                  </div>

                  {/* 字体颜色选择器 */}
                  <div className="maobu-player-subtitle-section">
                    <div className="maobu-player-subtitle-section-title">
                      <span>字体颜色</span>
                    </div>
                    <div className="maobu-player-color-palette">
                      {[
                        { color: "#ffffff", label: "纯白" },
                        { color: "#fde047", label: "暖黄" },
                        { color: "#38bdf8", label: "晴空" },
                        { color: "#4ade80", label: "翠绿" },
                        { color: "#f472b6", label: "珊瑚" },
                      ].map((item) => (
                        <button
                          key={item.color}
                          type="button"
                          onClick={() => updateSubStyle((prev) => ({ ...prev, color: item.color }))}
                          className={`maobu-player-color-dot ${subStyle.color === item.color ? "active" : ""}`}
                          style={{ backgroundColor: item.color }}
                          title={item.label}
                        >
                          {subStyle.color === item.color && (
                            <span className="maobu-player-color-check" />
                          )}
                        </button>
                      ))}
                    </div>
                  </div>

                  {/* 微晶底衬与垂直高度 */}
                  <div className="maobu-player-subtitle-section">
                    <div className="maobu-player-subtitle-section-title">
                      <span>微晶底衬</span>
                      <button
                        type="button"
                        onClick={() =>
                          updateSubStyle((prev) => ({
                            ...prev,
                            bgOpacity: prev.bgOpacity > 0 ? 0 : 0.5,
                          }))
                        }
                        className={`maobu-player-chip-btn ${subStyle.bgOpacity > 0 ? "active" : ""}`}
                      >
                        {subStyle.bgOpacity > 0 ? "已开启" : "已关闭"}
                      </button>
                    </div>
                  </div>

                  <div className="maobu-player-subtitle-section">
                    <div className="maobu-player-subtitle-section-title">
                      <span>垂直位置 (距底)</span>
                      <span style={{ fontFamily: "monospace", color: "#38bdf8" }}>{subStyle.bottomOffset}%</span>
                    </div>
                    <input
                      type="range"
                      min="3"
                      max="28"
                      step="1"
                      value={subStyle.bottomOffset}
                      onChange={(e) => {
                        const v = parseInt(e.target.value, 10) || 7;
                        updateSubStyle((prev) => ({ ...prev, bottomOffset: v }));
                      }}
                      className="maobu-player-sub-slider"
                    />
                  </div>

                  {/* 时轴微调 */}
                  {subtitles.length > 0 && (
                    <div className="maobu-player-subtitle-section" style={{ borderBottom: "none", marginBottom: 0, paddingBottom: 0 }}>
                      <div className="maobu-player-subtitle-section-title">
                        <span>时轴微调</span>
                        <span style={{ fontSize: "11px", fontFamily: "monospace", color: subtitleOffset !== 0 ? "#38bdf8" : "#94a3b8" }}>
                          {subtitleOffset > 0 ? `+${subtitleOffset}` : subtitleOffset}s
                        </span>
                      </div>
                      <div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
                        <button
                          type="button"
                          onClick={() => setSubtitleOffset((prev) => parseFloat((prev - 0.5).toFixed(1)))}
                          className="maobu-player-offset-btn"
                          title="字幕提前 0.5s"
                        >
                          -0.5s
                        </button>
                        {subtitleOffset !== 0 && (
                          <button
                            type="button"
                            onClick={() => setSubtitleOffset(0)}
                            className="maobu-player-offset-btn"
                            title="重置时轴偏移"
                          >
                            重置
                          </button>
                        )}
                        <button
                          type="button"
                          onClick={() => setSubtitleOffset((prev) => parseFloat((prev + 0.5).toFixed(1)))}
                          className="maobu-player-offset-btn"
                          title="字幕延后 0.5s"
                        >
                          +0.5s
                        </button>
                      </div>
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* 单帧截图 */}
            <button
              type="button"
              onClick={captureScreenshot}
              title="截图当前帧 (S)"
              className="maobu-player-icon-btn maobu-player-btn-screenshot"
            >
              <Camera size={16} />
            </button>

            {/* 画中画 */}
            <button
              type="button"
              onClick={togglePiP}
              title={isMiniMode ? "还原窗口 (P)" : "画中画迷你置顶窗口 (P)"}
              className={`maobu-player-icon-btn maobu-player-btn-pip ${isMiniMode ? "active" : ""}`}
            >
              <PictureInPicture size={16} />
            </button>

            {/* 播放列表抽屉开关 */}
            <button
              ref={playlistBtnRef}
              type="button"
              onClick={() => setShowPlaylist((prev) => !prev)}
              title="播放列表 (L)"
              className={`maobu-player-icon-btn ${showPlaylist ? "active" : ""}`}
              style={{ position: "relative" }}
            >
              <ListVideo size={16} />
              {playlist.length > 0 && (
                <span className="maobu-player-btn-badge">{playlist.length}</span>
              )}
            </button>

            {/* 系统默认播放器打开 */}
            <button
              type="button"
              onClick={openWithExternalPlayer}
              title="使用系统默认播放器打开"
              className="maobu-player-icon-btn maobu-player-btn-external"
            >
              <ExternalLink size={16} />
            </button>

            {/* 全屏 */}
            <button
              type="button"
              onClick={toggleFullscreen}
              title={isFullscreen ? "退出全屏 (F)" : "全屏 (F)"}
              className="maobu-player-icon-btn"
            >
              {isFullscreen ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
            </button>
          </div>
        </div>
      </div>

      {/* 播放列表右侧极简超晶抽屉 */}
      <div
        ref={drawerRef}
        className={`maobu-player-playlist-drawer ${showPlaylist ? "open" : ""}`}
        onClick={(e) => e.stopPropagation()}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* 头部标题与操作 */}
        <div className="maobu-player-playlist-header">
          <div className="maobu-player-playlist-title">
            <ListVideo size={16} style={{ color: "#38bdf8" }} />
            <span>播放列表</span>
            <span className="maobu-player-playlist-count">{playlist.length}</span>
          </div>

          <div style={{ display: "flex", alignItems: "center", gap: "4px" }}>
            <button
              type="button"
              onClick={scrollToActiveItem}
              title="定位当前播放视频"
              className="maobu-player-icon-btn"
            >
              <LocateFixed size={14} />
            </button>
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              title="添加视频文件"
              className="maobu-player-icon-btn"
            >
              <Plus size={14} />
            </button>
            <button
              type="button"
              onClick={() => {
                setPlaylist([]);
                triggerToast("已清空播放列表");
              }}
              title="清空列表"
              className="maobu-player-icon-btn"
            >
              <Trash2 size={14} />
            </button>
            <button
              type="button"
              onClick={() => setShowPlaylist(false)}
              title="关闭列表 (L)"
              className="maobu-player-icon-btn"
            >
              <X size={14} />
            </button>
          </div>
        </div>

        {/* 搜索过滤栏 */}
        <div className="maobu-player-playlist-search-wrap">
          <Search size={13} className="maobu-player-playlist-search-icon" />
          <input
            type="text"
            value={searchKeyword}
            onChange={(e) => setSearchKeyword(e.target.value)}
            placeholder="搜索当前列表中的视频..."
            className="maobu-player-playlist-search-input"
          />
          {searchKeyword && (
            <button
              type="button"
              onClick={() => setSearchKeyword("")}
              className="maobu-player-playlist-search-clear"
              title="清除搜索"
            >
              <X size={12} />
            </button>
          )}
        </div>

        {/* 播放模式与排序工具栏 */}
        <div className="maobu-player-playlist-toolbar">
          <button
            type="button"
            onClick={cyclePlayMode}
            className="maobu-player-playlist-mode-btn"
            title="点击切换播放模式"
          >
            {renderModeIcon()}
            <span>{modeLabel[playMode]}</span>
          </button>

          <div style={{ position: "relative" }}>
            <button
              type="button"
              onClick={() => setShowSortMenu((prev) => !prev)}
              className={`maobu-player-playlist-mode-btn ${sortType !== "default" ? "active" : ""}`}
              title="列表排序"
            >
              <ArrowUpDown size={12} />
              <span>
                {sortType === "name-asc" && "名称 ↑"}
                {sortType === "name-desc" && "名称 ↓"}
                {sortType === "size-desc" && "大小 ↓"}
                {sortType === "size-asc" && "大小 ↑"}
                {sortType === "default" && "智能默认"}
              </span>
            </button>

            {showSortMenu && (
              <div className="maobu-player-sort-menu">
                <button
                  type="button"
                  onClick={() => {
                    setSortType("default");
                    setShowSortMenu(false);
                  }}
                  className={`maobu-player-menu-item ${sortType === "default" ? "active" : ""}`}
                >
                  <span>智能默认排序</span>
                  {sortType === "default" && <Check size={12} />}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setSortType("name-asc");
                    setShowSortMenu(false);
                  }}
                  className={`maobu-player-menu-item ${sortType === "name-asc" ? "active" : ""}`}
                >
                  <span>文件名升序 (A ➔ Z)</span>
                  {sortType === "name-asc" && <Check size={12} />}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setSortType("name-desc");
                    setShowSortMenu(false);
                  }}
                  className={`maobu-player-menu-item ${sortType === "name-desc" ? "active" : ""}`}
                >
                  <span>文件名降序 (Z ➔ A)</span>
                  {sortType === "name-desc" && <Check size={12} />}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setSortType("size-desc");
                    setShowSortMenu(false);
                  }}
                  className={`maobu-player-menu-item ${sortType === "size-desc" ? "active" : ""}`}
                >
                  <span>按文件大小 (从大到小)</span>
                  {sortType === "size-desc" && <Check size={12} />}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setSortType("size-asc");
                    setShowSortMenu(false);
                  }}
                  className={`maobu-player-menu-item ${sortType === "size-asc" ? "active" : ""}`}
                >
                  <span>按文件大小 (从小到大)</span>
                  {sortType === "size-asc" && <Check size={12} />}
                </button>
              </div>
            )}
          </div>
        </div>

        {/* 视频条目列表 */}
        <div className="maobu-player-playlist-items">
          {displayPlaylist.length > 0 ? (
            displayPlaylist.map((item, idx) => {
              const isActive = item.path === filePath;
              return (
                <div
                  key={item.path}
                  ref={isActive ? activeItemRef : undefined}
                  className={`maobu-player-playlist-item ${isActive ? "active" : ""}`}
                  onClick={() => loadAndPlayVideo(item.path, item.name)}
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
                      onClick={(e) => copyItemPath(e, item.path)}
                      title="复制完整文件路径"
                      className="maobu-player-icon-btn"
                      style={{ padding: "4px" }}
                    >
                      <Copy size={13} />
                    </button>
                    <button
                      type="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        api.openPathDirect(item.path).catch(console.error);
                      }}
                      title="打开所在文件夹"
                      className="maobu-player-icon-btn"
                      style={{ padding: "4px" }}
                    >
                      <FolderOpen size={13} />
                    </button>
                    <button
                      type="button"
                      onClick={(e) => handleRemovePlaylistItem(e, item.path)}
                      title="从列表移除"
                      className="maobu-player-icon-btn"
                      style={{ padding: "4px" }}
                    >
                      <X size={13} />
                    </button>
                  </div>
                </div>
              );
            })
          ) : (
            <div style={{ textAlign: "center", padding: "40px 20px", color: "#64748b", fontSize: "12px" }}>
              {searchKeyword ? "未找到匹配的视频" : "暂无视频，点击右上角 + 或拖入视频文件"}
            </div>
          )}
        </div>

        {/* 底部统计栏 */}
        <div className="maobu-player-playlist-footer">
          <span>共 {playlist.length} 个视频{searchKeyword ? ` (已过滤出 ${displayPlaylist.length} 个)` : ""}</span>
          <span>支持拖拽文件加入</span>
        </div>
      </div>

      {/* 轻量浮动 Toast 提示 */}
      {toastMessage && (
        <div className="maobu-player-toast">
          {toastMessage}
        </div>
      )}
    </div>
  );
}
