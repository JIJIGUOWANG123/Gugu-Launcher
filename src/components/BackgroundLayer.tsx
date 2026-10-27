import { useEffect, useMemo, useState } from "react";
import { api } from "../lib/tauri-api";
import type { AppearanceSettings, BackgroundItem } from "../lib/types";

interface Props {
  appearance: AppearanceSettings;
}

const MIN_INTERVAL = 5;

/**
 * 背景层：四种方案互斥渲染，铺在 app 底下、内容层保持毛玻璃可读。
 * - 渐变：上白下主题色的线性渐变；
 * - 本地图 / Steam 壁纸：按播放方式轮播（单张 / 顺序 / 随机），视频用 <video> 播；
 * - URL：一张网络图，可定时换（加时间戳避免缓存）。
 */
export function BackgroundLayer({ appearance }: Props) {
  const [items, setItems] = useState<BackgroundItem[]>([]);
  const [cursor, setCursor] = useState(0);
  const [urlTick, setUrlTick] = useState(0);

  const kind = appearance.mode === "wallpaper" ? "wallpaper" : "local";

  useEffect(() => {
    if (appearance.mode !== "local" && appearance.mode !== "wallpaper") {
      setItems([]);
      return;
    }
    let alive = true;
    void api
      .backgroundList(kind, 0, 20)
      .then((page) => {
        if (alive) setItems(page.items);
      })
      .catch(() => {
        if (alive) setItems([]);
      });
    return () => {
      alive = false;
    };
    // 只在「方案 / 壁纸目录」变化时拉一次；选文件不重新扫目录（后端扫描很贵，会卡住窗口）
  }, [appearance.mode, kind, appearance.wallpaperDir]);

  const playMode = appearance.mode === "wallpaper" ? appearance.wallpaperMode : appearance.localMode;
  const interval = Math.max(
    MIN_INTERVAL,
    appearance.mode === "api"
      ? appearance.apiIntervalSec
      : appearance.mode === "wallpaper"
        ? appearance.wallpaperIntervalSec
        : appearance.localIntervalSec,
  );

  // 轮播：顺序就是往后走一格，随机就随便挑一张
  useEffect(() => {
    if (playMode === "single" || items.length < 2) return;
    const timer = window.setInterval(() => {
      setCursor((prev) => (playMode === "random" ? Math.floor(Math.random() * items.length) : (prev + 1) % items.length));
    }, interval * 1000);
    return () => window.clearInterval(timer);
  }, [playMode, interval, items.length]);

  useEffect(() => {
    if (appearance.mode !== "api" || appearance.apiMode === "single") return;
    const timer = window.setInterval(() => setUrlTick((value) => value + 1), Math.max(MIN_INTERVAL, appearance.apiIntervalSec) * 1000);
    return () => window.clearInterval(timer);
  }, [appearance.mode, appearance.apiMode, appearance.apiIntervalSec]);

  const selectedFile = appearance.mode === "wallpaper" ? appearance.wallpaperFile : appearance.localFile;
  const selectedIndex = useMemo(() => {
    const hit = items.findIndex((item) => item.file === selectedFile);
    return hit >= 0 ? hit : 0;
  }, [items, selectedFile]);

  if (appearance.mode === "gradient") {
    return (
      <div className="app-bg" data-bg-mode="gradient" aria-hidden="true">
        <div
          className="app-bg__gradient"
          style={{ background: "linear-gradient(180deg, " + appearance.gradientFrom + " 0%, " + appearance.gradientFrom + " 34%, " + appearance.gradientTo + " 100%)" }}
        />
      </div>
    );
  }

  if (appearance.mode === "api") {
    return (
      <div className="app-bg" data-bg-mode="api" aria-hidden="true">
        {appearance.apiUrl ? (
          <img className="app-bg__media" key={urlTick} src={appearance.apiUrl + (appearance.apiUrl.includes("?") ? "&" : "?") + "t=" + urlTick} alt="" />
        ) : (
          <div className="app-bg__gradient" style={{ background: "linear-gradient(160deg, var(--surface-muted), var(--bg))" }} />
        )}
        <div className="app-bg__veil" />
      </div>
    );
  }

  const current = items.length > 0 ? items[playMode === "single" ? selectedIndex : cursor % items.length] : null;
  if (!current) {
    return (
      <div className="app-bg" data-bg-mode={kind} aria-hidden="true">
        <div className="app-bg__gradient" style={{ background: "linear-gradient(160deg, var(--surface-muted), var(--bg))" }} />
      </div>
    );
  }

  return (
    <div className="app-bg" data-bg-mode={kind} aria-hidden="true">
      {current.isVideo && current.url ? (
        <video className="app-bg__media" src={current.url} poster={current.previewUrl} autoPlay muted loop playsInline />
      ) : (
        <img className="app-bg__media" src={current.previewUrl || current.url} alt="" />
      )}
      <div className="app-bg__veil" />
    </div>
  );
}
