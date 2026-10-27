import { useCallback, useEffect, useState } from "react";
import { icons } from "../assets/icons";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { AppearanceSettings, BackgroundMode, BackgroundPage, BackgroundPlay, Settings } from "../lib/types";
import { formatBytes } from "../lib/format";

interface Props {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  notify: (message: string) => void;
}

/** 枚举值与后端一致：gradient / local / api / wallpaper。 */
const MODES: Array<{ id: BackgroundMode; labelKey: string; descKey: string }> = [
  { id: "gradient", labelKey: "settings.bg.mode.gradient", descKey: "settings.bg.desc.gradient" },
  { id: "local", labelKey: "settings.bg.mode.local", descKey: "settings.bg.desc.local" },
  { id: "api", labelKey: "settings.bg.mode.url", descKey: "settings.bg.desc.url" },
  { id: "wallpaper", labelKey: "settings.bg.mode.steam", descKey: "settings.bg.desc.steam" },
];

const PLAYS: Array<{ id: BackgroundPlay; labelKey: string }> = [
  { id: "single", labelKey: "settings.bg.play.single" },
  { id: "shuffle", labelKey: "settings.bg.play.shuffle" },
  { id: "random", labelKey: "settings.bg.play.random" },
];

const PAGE_SIZE = 20;

/** 预览对象的稳定标识：文件名 + 地址（换文件或换方案都要重置错误态）。 */
function previewItemKey(page: BackgroundPage | null, activeName: string): string {
  const items = page?.items ?? [];
  const item = items.find((entry) => entry.file === activeName) ?? items[0] ?? null;
  return item ? item.file + "|" + item.url : "";
}

/**
 * 设置 → 外观 → 背景：四方案互斥，选中哪个就只有哪个生效（其余三个自动关闭），
 * 改动立刻写进 settings.appearance，背景层马上跟着换。
 * 切换方案时先探一次 background_list，失败就把 mode 回滚，避免停在后端不认的枚举上。
 */
export function BackgroundPanel({ settings, onChange, notify }: Props) {
  const t = useT();
  const appearance = settings.appearance;
  const [page, setPage] = useState(0);
  const [browsed, setBrowsed] = useState<BackgroundPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [defaultDir, setDefaultDir] = useState("");
  const [urlTick, setUrlTick] = useState(0);
  const [previewError, setPreviewError] = useState("");

  const patch = useCallback(
    (next: Partial<AppearanceSettings>) => {
      onChange({ appearance: { ...appearance, ...next } });
    },
    [appearance, onChange],
  );

  const listKind: "local" | "wallpaper" = appearance.mode === "wallpaper" ? "wallpaper" : "local";
  const browsing = appearance.mode === "local" || appearance.mode === "wallpaper";

  useEffect(() => {
    void api
      .wallpaperDirDefault()
      .then(setDefaultDir)
      .catch(() => setDefaultDir(""));
  }, []);

  useEffect(() => {
    setPage(0);
  }, [appearance.mode, appearance.wallpaperDir]);

  const loadPage = useCallback(
    async (nextPage: number) => {
      if (!browsing) return;
      setLoading(true);
      try {
        const result = await api.backgroundList(listKind, nextPage, PAGE_SIZE);
        setBrowsed(result);
        setPage(nextPage);
      } catch (error) {
        notify(String(error));
      } finally {
        setLoading(false);
      }
    },
    [browsing, listKind, notify],
  );

  useEffect(() => {
    if (!browsing) {
      setBrowsed(null);
      return;
    }
    void loadPage(0);
  }, [browsing, loadPage]);

  /** 切方案：先跟后端确认这个 kind 能列，出错就提示后端原文并保持原方案。 */
  const selectMode = async (next: BackgroundMode) => {
    if (next === appearance.mode) return;
    const previous = appearance.mode;
    // 直接切过去；列表加载失败时再回滚（少一次阻塞式请求，切方案不会卡住界面）
    patch({ mode: next });
    if (next !== "local" && next !== "wallpaper") return;
    try {
      const result = await api.backgroundList(next, 0, PAGE_SIZE);
      setBrowsed(result);
      setPage(0);
    } catch (error) {
      patch({ mode: previous });
      notify(String(error));
    }
  };

  const pick = async (file: string) => {
    const next: Partial<AppearanceSettings> = appearance.mode === "wallpaper" ? { wallpaperFile: file } : { localFile: file };
    const previous = appearance.mode;
    try {
      // 键名必须是 file（真机报过 missing required key file）
      await api.backgroundApply({ kind: listKind, file });
      patch(next);
      notify(t("settings.bg.applied", { name: file }));
    } catch (error) {
      patch({ mode: previous });
      notify(String(error));
    }
  };

  // 换预览对象（换文件 / 换方案）时清掉上一条的错误，否则会一直挂着旧原因
  const previewKey = previewItemKey(browsed, appearance.mode === "wallpaper" ? appearance.wallpaperFile : appearance.localFile);
  useEffect(() => {
    setPreviewError("");
  }, [previewKey]);

  const pages = browsed ? Math.max(1, Math.ceil(browsed.total / browsed.pageSize)) : 1;
  const activeName = appearance.mode === "wallpaper" ? appearance.wallpaperFile : appearance.localFile;
  // 预览框：优先显示正在用的那张，没有就用列表第一张；视频走 <video muted loop>
  const previewItem = browsed?.items.find((item) => item.file === activeName) ?? browsed?.items[0] ?? null;

  return (
    <div className="settings-card bg-panel" data-bg-panel>
      <div className="settings-card__title">{t("settings.groupBackground")}</div>

      <div className="bg-schemes">
        {MODES.map((mode) => {
          const active = appearance.mode === mode.id;
          return (
            <button
              key={mode.id}
              type="button"
              className={"bg-scheme" + (active ? " is-active" : "")}
              data-bg-mode={mode.id}
              data-bg-state={active ? "on" : "off"}
              aria-pressed={active}
              onClick={() => void selectMode(mode.id)}
            >
              <span className="bg-scheme__radio" aria-hidden="true" />
              <span className="bg-scheme__label">{t(mode.labelKey)}</span>
              <span className="bg-scheme__state">{t(active ? "settings.bg.on" : "settings.bg.off")}</span>
              <span className="bg-scheme__desc">{t(mode.descKey)}</span>
            </button>
          );
        })}
      </div>

      {appearance.mode === "gradient" ? (
        <div className="bg-config" data-bg-config="gradient">
          <div className="bg-gradient-row">
            <label className="bg-color">
              <span>{t("settings.bg.gradientTop")}</span>
              <input
                type="color"
                value={appearance.gradientFrom}
                data-bg-color="top"
                onChange={(event) => patch({ gradientFrom: event.target.value })}
              />
            </label>
            <label className="bg-color">
              <span>{t("settings.bg.gradientBottom")}</span>
              <input
                type="color"
                value={appearance.gradientTo}
                data-bg-color="bottom"
                onChange={(event) => patch({ gradientTo: event.target.value })}
              />
            </label>
          </div>
          <div
            className="bg-gradient-preview"
            data-bg-gradient-preview
            style={{
              background:
                "linear-gradient(180deg, " +
                appearance.gradientFrom +
                " 0%, " +
                appearance.gradientFrom +
                " 34%, " +
                appearance.gradientTo +
                " 100%)",
            }}
          />
          <div className="row__hint">{t("settings.bg.gradientHint")}</div>
        </div>
      ) : null}

      {browsing ? (
        <div className="bg-config" data-bg-config={listKind}>
          {appearance.mode === "wallpaper" ? (
            <div className="field">
              <label htmlFor="bg-wallpaper-dir">{t("settings.bg.dir")}</label>
              <div className="field-pair">
                <input
                  id="bg-wallpaper-dir"
                  type="text"
                  value={appearance.wallpaperDir || defaultDir}
                  data-bg-dir
                  onChange={(event) => patch({ wallpaperDir: event.target.value })}
                />
                <button
                  type="button"
                  className="pill-outline"
                  onClick={() =>
                    void api
                      .pickDirectory()
                      .then((dir) => {
                        if (dir) patch({ wallpaperDir: dir });
                      })
                      .catch((error) => notify(String(error)))
                  }
                >
                  {t("settings.bg.dirPick")}
                </button>
                <button type="button" className="pill-outline" onClick={() => patch({ wallpaperDir: "" })}>
                  {t("settings.bg.dirReset")}
                </button>
              </div>
              <div className="row__hint">{t("settings.bg.dirHint")}</div>
            </div>
          ) : null}

          {previewItem ? (
            <div className="bg-preview" data-bg-preview={previewItem.isVideo ? "video" : "image"}>
              {previewItem.isVideo ? (
                <>
                  {/* 视频项始终保留 <video>：加载/解码失败只在上面叠一层错误提示，不把元素换掉 */}
                  <video
                    className="bg-preview__media"
                    data-bg-preview-video
                    key={previewItem.url}
                    src={previewItem.url}
                    poster={previewItem.previewUrl}
                    autoPlay
                    muted
                    loop
                    playsInline
                    onError={() => setPreviewError("无法播放该视频（编码不支持或文件损坏）")}
                  />
                  {previewError ? (
                    <div className="bg-preview__error" data-bg-preview-error>
                      {t("settings.bg.previewFailed", { reason: previewError })}
                    </div>
                  ) : null}
                </>
              ) : previewError ? (
                <div className="bg-preview__error" data-bg-preview-error>
                  {t("settings.bg.previewFailed", { reason: previewError })}
                </div>
              ) : (
                <img
                  className="bg-preview__media"
                  data-bg-preview-img
                  key={previewItem.url}
                  src={previewItem.previewUrl || previewItem.url}
                  alt=""
                  onError={() => setPreviewError("无法加载该图片")}
                />
              )}
              <span className="bg-preview__name">{previewItem.file}</span>
            </div>
          ) : null}

          {loading ? (
            <div className="row__hint">{t("settings.bg.loading")}</div>
          ) : browsed === null || browsed.items.length === 0 ? (
            <div className="empty-hint" data-bg-empty>
              {t("settings.bg.empty")}
              {browsed?.dir ? (
                <div className="row__hint" data-bg-empty-dir>
                  {t("settings.bg.emptyDir", { dir: browsed.dir })}
                </div>
              ) : null}
            </div>
          ) : (
            <div className="bg-grid" data-bg-grid>
              {browsed.items.map((item) => (
                <button
                  key={item.file}
                  type="button"
                  className={"bg-tile" + (activeName === item.file ? " is-active" : "")}
                  data-bg-file={item.file}
                  data-bg-video={item.isVideo ? "1" : "0"}
                  onClick={() => void pick(item.file)}
                >
                  <img className="bg-tile__img" src={item.previewUrl || item.url} alt={item.file} />
                  {item.isVideo ? <span className="bg-tile__badge">{t("settings.bg.video")}</span> : null}
                  <span className="bg-tile__name">{item.file}</span>
                  <span className="bg-tile__meta">{formatBytes(item.size)}</span>
                </button>
              ))}
            </div>
          )}

          {browsed ? (
            <div className="bg-pager">
              <button type="button" className="pill-outline" data-bg-prev disabled={page <= 0 || loading} onClick={() => void loadPage(page - 1)}>
                {t("settings.bg.prev")}
              </button>
              <span className="row__hint" data-bg-page>
                {t("settings.bg.page", { page: page + 1, pages, total: browsed.total })}
              </span>
              <button
                type="button"
                className="pill-outline"
                data-bg-next
                disabled={page + 1 >= pages || loading}
                onClick={() => void loadPage(page + 1)}
              >
                {t("settings.bg.nextPage")}
              </button>
            </div>
          ) : null}

          <div className="bg-play">
            <span className="bg-color__label">{t("settings.bg.play")}</span>
            <div className="segmented segmented--sm">
              {PLAYS.map((play) => {
                const current = appearance.mode === "wallpaper" ? appearance.wallpaperMode : appearance.localMode;
                return (
                  <button
                    key={play.id}
                    type="button"
                    className={"segmented__item" + (current === play.id ? " is-active" : "")}
                    data-bg-play={play.id}
                    onClick={() =>
                      patch(appearance.mode === "wallpaper" ? { wallpaperMode: play.id } : { localMode: play.id })
                    }
                  >
                    {t(play.labelKey)}
                  </button>
                );
              })}
            </div>
            <label className="bg-interval">
              <span>{t("settings.bg.interval")}</span>
              <input
                type="number"
                min={5}
                max={3600}
                value={appearance.mode === "wallpaper" ? appearance.wallpaperIntervalSec : appearance.localIntervalSec}
                data-bg-interval
                onChange={(event) => {
                  const value = Math.max(5, Number(event.target.value) || 5);
                  patch(appearance.mode === "wallpaper" ? { wallpaperIntervalSec: value } : { localIntervalSec: value });
                }}
              />
            </label>
          </div>
        </div>
      ) : null}

      {appearance.mode === "api" ? (
        <div className="bg-config" data-bg-config="api">
          <div className="field">
            <label htmlFor="bg-api-url">{t("settings.bg.url")}</label>
            <input
              id="bg-api-url"
              type="text"
              value={appearance.apiUrl}
              data-bg-url
              placeholder={t("settings.bg.urlPlaceholder")}
              onChange={(event) => patch({ apiUrl: event.target.value })}
            />
          </div>
          <div className="bg-play">
            <span className="bg-color__label">{t("settings.bg.play")}</span>
            <div className="segmented segmented--sm">
              {PLAYS.map((play) => (
                <button
                  key={play.id}
                  type="button"
                  className={"segmented__item" + (appearance.apiMode === play.id ? " is-active" : "")}
                  data-bg-url-play={play.id}
                  onClick={() => patch({ apiMode: play.id })}
                >
                  {t(play.labelKey)}
                </button>
              ))}
            </div>
            <label className="bg-interval">
              <span>{t("settings.bg.interval")}</span>
              <input
                type="number"
                min={5}
                max={3600}
                value={appearance.apiIntervalSec}
                data-bg-url-interval
                onChange={(event) => patch({ apiIntervalSec: Math.max(5, Number(event.target.value) || 5) })}
              />
            </label>
            <button
              type="button"
              className="pill-outline"
              data-bg-url-next
              onClick={() => {
                if (!appearance.apiUrl.trim()) {
                  notify(t("settings.bg.needUrl"));
                  return;
                }
                setUrlTick((value) => value + 1);
                notify(t("settings.bg.applied", { name: appearance.apiUrl }));
              }}
            >
              <img src={icons.refresh} alt="" />
              {t("settings.bg.next")}
            </button>
          </div>
          {appearance.apiUrl ? (
            <img
              className="bg-url-preview"
              key={urlTick}
              src={appearance.apiUrl + (appearance.apiUrl.includes("?") ? "&" : "?") + "t=" + urlTick}
              alt=""
              onError={() => undefined}
            />
          ) : null}
        </div>
      ) : null}

      {activeName && browsing ? (
        <div className="row__hint">{t("settings.bg.active", { name: activeName })}</div>
      ) : null}
      <div className="row__hint">{t("settings.bg.footer")}</div>
    </div>
  );
}
