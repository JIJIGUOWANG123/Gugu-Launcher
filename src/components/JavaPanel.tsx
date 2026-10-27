import { useCallback, useEffect, useState } from "react";
import { icons } from "../assets/icons";
import { formatBytes } from "../lib/format";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { JavaRuntime, JavaSource, JavaSourceProbe, JavaVersion, Settings } from "../lib/types";

interface Props {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  notify: (message: string) => void;
}

const MAJOR_FILTERS = [0, 25, 21, 17, 8];

const VENDOR_KEY: Record<string, string> = {
  System: "settings.java.vendor.system",
  "BellSoft Liberica": "settings.java.vendor.bellsoft",
  Adoptium: "settings.java.vendor.adoptium",
  Zulu: "settings.java.vendor.zulu",
  Oracle: "settings.java.vendor.oracle",
  Bundled: "settings.java.vendor.bundled",
};

interface InstallState {
  /** 0..1，null 表示还没开始 */
  progress: number | null;
  done: boolean;
}

/**
 * 设置 → Java：已装列表（后端已去重，一行一个 JDK）+ 可下载版本面板。
 * 安装进度走下载中心，这里只显示按钮状态。
 */
export function JavaPanel({ settings, onChange, notify }: Props) {
  const t = useT();
  const [installed, setInstalled] = useState<JavaRuntime[] | null>(null);
  const [sources, setSources] = useState<JavaSource[]>([]);
  const [probes, setProbes] = useState<JavaSourceProbe[]>([]);
  const [source, setSource] = useState("auto");
  const [major, setMajor] = useState(0);
  const [versions, setVersions] = useState<JavaVersion[] | null>(null);
  const [installs, setInstalls] = useState<Record<string, InstallState>>({});

  const loadInstalled = useCallback(async () => {
    try {
      const list = await api.javaList();
      const seen = new Set<string>();
      const dedup = list.filter((item) => {
        const key = item.javaExe.toLowerCase();
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
      });
      dedup.sort((a, b) => (b.major - a.major) || b.version.localeCompare(a.version));
      setInstalled(dedup);
    } catch (error) {
      notify(String(error));
    }
  }, [notify]);

  useEffect(() => {
    void loadInstalled();
    void api.javaSources().then(setSources).catch(() => setSources([]));
    void api.javaProbeSources().then(setProbes).catch(() => setProbes([]));
  }, [loadInstalled]);

  useEffect(() => {
    let alive = true;
    void api
      .javaVersions({ source: source === "auto" ? undefined : source, major: major || undefined })
      .then((list) => {
        if (alive) setVersions(list);
      })
      .catch(() => {
        if (alive) setVersions([]);
      });
    return () => {
      alive = false;
    };
  }, [source, major]);

  // mock 安装进度：真后端会由 downloads_list / install://progress 推
  useEffect(() => {
    const running = Object.keys(installs).filter((key) => installs[key].progress !== null && !installs[key].done);
    if (running.length === 0) return;
    const timer = window.setInterval(() => {
      setInstalls((prev) => {
        const next: Record<string, InstallState> = { ...prev };
        for (const key of running) {
          const state = next[key];
          if (!state || state.progress === null || state.done) continue;
          const progress = Math.min(1, state.progress + 0.18);
          next[key] = { progress: progress, done: progress >= 1 };
        }
        return next;
      });
    }, 700);
    return () => window.clearInterval(timer);
  }, [installs]);

  const bestProbe = probes
    .filter((probe) => probe.ok && probe.latencyMs !== null)
    .slice()
    .sort((a, b) => (a.latencyMs ?? 0) - (b.latencyMs ?? 0))[0];

  const probeLine = probes
    .map((probe) => (sources.find((item) => item.id === probe.sourceId)?.name ?? probe.sourceId) + " " + (probe.latencyMs ?? "-") + "ms")
    .join(" · ");

  const install = async (item: JavaVersion) => {
    const key = item.source + ":" + item.version;
    setInstalls((prev) => ({ ...prev, [key]: { progress: 0, done: false } }));
    try {
      await api.javaInstall({ source: item.source, major: item.major, version: item.version });
      notify(t("settings.java.installStarted", { version: item.version }));
      void loadInstalled();
    } catch (error) {
      setInstalls((prev) => ({ ...prev, [key]: { progress: null, done: false } }));
      notify(String(error));
    }
  };

  const selectedExe = settings.javaPath;

  return (
    <div className="java-panel">
      <section className="card settings-card" data-java-installed>
        <div className="java-panel__head">
          <span className="settings-card__title">
            {t("settings.java.installed")}
            <span className="chip chip--tiny" data-java-count>
              {t("settings.java.installedCount", { count: installed?.length ?? 0 })}
            </span>
          </span>
          <div className="java-panel__head-actions">
            <label className="java-toggle">
              <input
                type="checkbox"
                checked={settings.autoDownloadJava}
                data-java-auto
                onChange={(event) => onChange({ autoDownloadJava: event.target.checked })}
              />
              {t("settings.java.autoDownload")}
            </label>
            <button type="button" className="pill-outline" onClick={() => void loadInstalled()}>
              <img src={icons.refresh} alt="" />
              {t("settings.java.refresh")}
            </button>
          </div>
        </div>

        {installed === null ? (
          <div className="row__hint">{t("common.loading")}</div>
        ) : installed.length === 0 ? (
          <div className="empty-hint">{t("settings.java.empty")}</div>
        ) : (
          <div className="java-list">
            {installed.map((runtime) => {
              const active = selectedExe ? selectedExe === runtime.javaExe : runtime.isDefault;
              return (
                <div
                  className={"java-row" + (active ? " is-active" : "")}
                  key={runtime.javaExe}
                  data-java-exe={runtime.javaExe}
                  title={runtime.javaExe}
                >
                  <span className="java-row__icon" aria-hidden="true">
                    <img src={icons.sliders} alt="" />
                  </span>
                  <div className="java-row__meta">
                    <div className="java-row__title">
                      <span className="java-row__name">Java {runtime.major}</span>
                      <span className="row__hint">{runtime.version}</span>
                      <span className="chip chip--tiny" data-java-vendor={runtime.vendor}>
                        {t(VENDOR_KEY[runtime.vendor] ?? "settings.java.vendor.other")}
                      </span>
                    </div>
                    <div className="java-row__path mono">{runtime.javaExe}</div>
                  </div>
                  <span className="java-row__ok">✓ {t("settings.java.available")}</span>
                  <button
                    type="button"
                    className="pill-outline pill-outline--tiny"
                    onClick={() =>
                      void api.openExternal(runtime.path).catch((error) => notify(String(error)))
                    }
                  >
                    {t("settings.java.openFolder")}
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <section className="card settings-card" data-java-download>
        <div className="settings-card__title">{t("settings.java.downloadTitle")}</div>
        <div className="java-sources">
          {sources.map((item) => (
            <button
              key={item.id}
              type="button"
              className={"chip chip--button" + (source === item.id ? " is-active" : "")}
              data-java-source={item.id}
              title={item.description}
              onClick={() => setSource(item.id)}
            >
              {item.name}
            </button>
          ))}
        </div>
        <div className="row__hint" data-java-probe>
          {probeLine}
          {bestProbe ? " → " + t("settings.java.probePick", { name: sources.find((item) => item.id === bestProbe.sourceId)?.name ?? bestProbe.sourceId }) : ""}
        </div>

        <div className="java-majors">
          {MAJOR_FILTERS.map((item) => (
            <button
              key={item}
              type="button"
              className={"chip chip--button" + (major === item ? " is-active" : "")}
              data-java-major={item}
              onClick={() => setMajor(item)}
            >
              {item === 0 ? t("settings.java.majorAll") : "Java " + item}
            </button>
          ))}
        </div>

        {versions === null ? (
          <div className="row__hint">{t("common.loading")}</div>
        ) : versions.length === 0 ? (
          <div className="empty-hint">{t("settings.java.noVersions")}</div>
        ) : (
          <div className="java-list">
            {versions.map((item) => {
              const key = item.source + ":" + item.version;
              const state = installs[key];
              const sourceMeta = sources.find((entry) => entry.id === item.source);
              const installing = state && state.progress !== null && !state.done;
              const done = Boolean(state?.done);
              return (
                <div className="java-row" key={key} data-java-version={key}>
                  <div className="java-row__meta">
                    <div className="java-row__title">
                      <span className="java-row__name">Java {item.major}</span>
                      <span className="row__hint">{item.version}</span>
                      {item.lts ? <span className="chip chip--tiny">{t("settings.java.lts")}</span> : null}
                      <span className="chip chip--tiny">{sourceMeta?.name ?? item.source}</span>
                    </div>
                    <div className="java-row__path">{formatBytes(item.sizeBytes)}</div>
                  </div>
                  {sourceMeta?.requiresLicense ? (
                    <button
                      type="button"
                      className="pill-outline pill-outline--tiny"
                      data-java-license
                      title={t("settings.java.licenseTip")}
                      onClick={() => void api.openExternal(item.url).catch((error) => notify(String(error)))}
                    >
                      {t("settings.java.gotoDownload")}
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="pill-button java-row__install"
                      data-java-install={key}
                      disabled={installing || done}
                      onClick={() => void install(item)}
                    >
                      {done
                        ? t("settings.java.installedBadge")
                        : installing
                          ? t("settings.java.installing", { percent: Math.round((state?.progress ?? 0) * 100) })
                          : t("settings.java.install")}
                    </button>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
