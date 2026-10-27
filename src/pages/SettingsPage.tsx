import { useEffect, useMemo, useRef, useState } from "react";
import { brand } from "../assets/brand";
import { BackgroundPanel } from "../components/BackgroundPanel";
import { JavaPanel } from "../components/JavaPanel";
import { StoragePanel } from "../components/StoragePanel";
import { Modal } from "../components/Modal";
import { formatBytes } from "../lib/format";
import { DownloadCenter } from "../components/DownloadCenter";
import { useT } from "../lib/i18n";
import { ACCENT_PRESETS, DEFAULT_UI_FONT, fontStack, isFontAvailable, isPresetAccent, normalizeAccent } from "../lib/theme";
import { api } from "../lib/tauri-api";
import type {
  AppInfo,
  DownloadSourceId,
  FontChoice,
  DataDirInfo,
  ResourceSourceId,
  Settings,
  SourceProbe,
  SourceProbeTarget,
  TranslationSource,
  TranslationTestResult,
} from "../lib/types";

type SectionKey = "general" | "appearance" | "java" | "download" | "content" | "storage" | "about";

interface GroupDef {
  id: string;
  titleKey: string;
  /** 搜索时额外匹配的词（俗称 / 英文名），命中标题或关键词都算 */
  keywords: string[];
}

interface SectionDef {
  key: SectionKey;
  labelKey: string;
  groups: GroupDef[];
}

/** 左侧导航 + 每个分组的搜索元数据，照 QookiX 设置页的顺序。 */
const SECTIONS: SectionDef[] = [
  {
    key: "general",
    labelKey: "settings.nav.general",
    groups: [
      { id: "font", titleKey: "settings.font", keywords: ["字体", "font", "显示"] },
      { id: "behavior", titleKey: "settings.groupBehavior", keywords: ["行为", "关闭", "最小化", "托盘", "close", "behavior", "退出"] },
      { id: "dataDir", titleKey: "settings.groupDataDir", keywords: ["数据目录", "目录", "迁移", "data", "datadir", "存储位置"] },
    ],
  },
  {
    key: "appearance",
    labelKey: "settings.nav.appearance",
    groups: [
      { id: "theme", titleKey: "settings.groupTheme", keywords: ["主题", "theme", "深色", "浅色", "dark", "light"] },
      { id: "accent", titleKey: "settings.accent", keywords: ["主题色", "accent", "颜色", "配色"] },
      {
        id: "background",
        titleKey: "settings.groupBackground",
        keywords: ["背景", "壁纸", "background", "wallpaper", "steam", "壁纸引擎", "轮播", "渐变"],
      },
    ],
  },
  {
    key: "java",
    labelKey: "settings.nav.java",
    groups: [{ id: "java", titleKey: "settings.java", keywords: ["java", "运行时", "jdk", "jre"] }],
  },
  {
    key: "download",
    labelKey: "settings.nav.download",
    groups: [
      { id: "parallel", titleKey: "settings.parallel", keywords: ["并发", "线程", "下载数", "parallel", "thread", "同时下载"] },
      { id: "mirror", titleKey: "settings.mirror", keywords: ["镜像", "源", "测速", "mirror", "bmclapi", "官方", "下载源"] },
      { id: "center", titleKey: "settings.downloadCenter", keywords: ["下载中心", "任务", "进度", "速度"] },
      { id: "serverMirror", titleKey: "settings.serverMirror", keywords: ["服务端", "镜像", "server", "mirror", "核心"] },
    ],
  },
  {
    key: "content",
    labelKey: "settings.nav.content",
    groups: [
      {
        id: "translation",
        titleKey: "settings.translation",
        keywords: ["翻译", "translation", "ai", "openai", "接口", "api", "key", "模型", "model"],
      },
      {
        id: "sayingApi",
        titleKey: "settings.sayingApi",
        keywords: ["一言", "saying", "hitokoto", "接口", "api", "地址"],
      },
      {
        id: "contentSource",
        titleKey: "settings.contentSource",
        keywords: ["来源", "内容来源", "modrinth", "curseforge", "mcim", "模组", "整合包", "资源包", "content"],
      },
      { id: "curseforgeKey", titleKey: "settings.cfKeyTitle", keywords: ["curseforge", "key", "api", "镜像", "社区"] },
      {
        id: "msLogin",
        titleKey: "settings.msLogin",
        keywords: ["微软", "登录", "client id", "clientid", "azure", "oauth", "账号", "ms"],
      },
    ],
  },
  {
    key: "storage",
    labelKey: "settings.nav.storage",
    groups: [{ id: "storage", titleKey: "settings.storage", keywords: ["存储", "目录", "磁盘", "路径", "storage"] }],
  },
  {
    key: "about",
    labelKey: "settings.nav.about",
    groups: [{ id: "about", titleKey: "settings.aboutTitle", keywords: ["关于", "版本", "仓库", "about", "version"] }],
  },
];

/** 三个可选镜像源（自动档单独一行放在上面）。 */
const MIRRORS: Array<{ id: DownloadSourceId; titleKey: string; descKey: string | null }> = [
  { id: "official", titleKey: "settings.mirrorOfficial", descKey: "settings.mirrorOfficialDesc" },
  { id: "bmclapi", titleKey: "settings.mirrorBmclapi", descKey: "settings.mirrorBmclapiDesc" },
  { id: "custom", titleKey: "settings.mirrorCustom", descKey: null },
];

/**
 * 两个内容来源：点即选中并落盘（CurseForge 的 Key 单独保存）。
 * MCIM 是 CurseForge 的镜像通道，不再作为用户可选来源，这里只留一句说明。
 */
const CONTENT_SOURCES: Array<{ id: Extract<ResourceSourceId, "modrinth" | "curseforge">; titleKey: string; hintKey: string }> = [
  { id: "modrinth", titleKey: "settings.contentSourceModrinth", hintKey: "settings.contentSourceModrinthHint" },
  { id: "curseforge", titleKey: "settings.contentSourceCurseforge", hintKey: "settings.contentSourceCurseforgeHint" },
];

/** 翻译接口草稿：默认沿用默认值（0 表示不填，后端按 0.2 / 512 处理）。 */
interface TranslationDraft {
  source: TranslationSource;
  baseUrl: string;
  apiKey: string;
  model: string;
  temperature: number;
  maxTokens: number;
}

interface Props {
  settings: Settings;
  appInfo: AppInfo | null;
  dataRoot: string;
  fonts: FontChoice[];
  fontMissing: boolean;
  onChange: (patch: Partial<Settings>) => void;
  /** 保存成功等轻提示，来自 App 的 toast */
  onNotify: (message: string) => void;
  /** 从别的页面跳进来时指定落到哪一页（例如账号页的「去 设置 → 内容服务 填写」） */
  initialSection?: string | null;
}

/**
 * 设置页（照 QookiX）：左侧导航 + 搜索 + 版本卡，右侧按分组渲染。
 * 下载页三块：并行下载 / 下载镜像源 / 下载中心（复用 DownloadCenter inline）。
 * 所有写入都走 onChange → api.settingsSet，没有「保存」按钮。
 */
export function SettingsPage({ settings, appInfo, dataRoot, fonts, fontMissing, onChange, onNotify, initialSection }: Props) {
  const t = useT();
  const [section, setSection] = useState<SectionKey>("general");
  const [search, setSearch] = useState("");

  const [probes, setProbes] = useState<Partial<Record<DownloadSourceId, SourceProbe | "probing">>>({});

  // 内容服务：翻译接口与 CurseForge Key 都用草稿，点「保存」才落盘
  // 自动翻译开关：老设置没有这个字段时按「开启」处理
  const translateEnabled = settings.translateEnabled !== false;
  useEffect(() => {
    let alive = true;
    void api
      .settingsGet()
      .then((fresh) => {
        if (!alive) return;
        // 资源页工具行的开关也会写这份设置，进来时对齐一次，避免显示旧状态
        if (typeof fresh.translateEnabled === "boolean" && fresh.translateEnabled !== settings.translateEnabled) {
          onChange({ translateEnabled: fresh.translateEnabled });
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
    // 只在挂载时对齐
  }, []);

  const [translation, setTranslation] = useState<TranslationDraft>(() => ({
    source: settings.translationSource,
    baseUrl: settings.translationBaseUrl,
    apiKey: settings.translationApiKey,
    model: settings.translationModel,
    temperature: settings.translationTemperature,
    maxTokens: settings.translationMaxTokens,
  }));
  const [curseforgeKey, setCurseforgeKey] = useState(settings.curseforgeApiKey);
  const [sayingApiDraft, setSayingApiDraft] = useState(settings.sayingApiUrl);
  const [showTranslationKey, setShowTranslationKey] = useState(false);
  const [showCurseforgeKey, setShowCurseforgeKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TranslationTestResult | null>(null);

  // 微软登录的 Client ID：草稿 + 眼睛 + 格式校验；后端说旧值非法时额外提示
  const [msClientDraft, setMsClientDraft] = useState(settings.msClientId);
  const [showMsClient, setShowMsClient] = useState(false);
  const [msClientError, setMsClientError] = useState("");
  const [msClientStoredInvalid, setMsClientStoredInvalid] = useState(false);
  const [fontError, setFontError] = useState(false);

  // 数据目录：当前信息 / 目标目录 / 二次确认 / 迁移结果
  const [dataDir, setDataDir] = useState<DataDirInfo | null>(null);
  const [dataDirTarget, setDataDirTarget] = useState("");
  const [dataDirConfirm, setDataDirConfirm] = useState(false);
  const [dataDirMigrating, setDataDirMigrating] = useState(false);
  const [dataDirDone, setDataDirDone] = useState(false);

  useEffect(() => {
    let alive = true;
    void api
      .dataDirGet()
      .then((info) => {
        if (alive) setDataDir(info);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const openDataDir = async () => {
    if (!dataDir) return;
    try {
      await api.openExternal(dataDir.path);
    } catch (error) {
      onNotify(String(error));
    }
  };

  const pickDataDir = async () => {
    try {
      const target = await api.dataDirPick();
      if (!target) {
        onNotify(t("settings.dataDirPickCanceled"));
        return;
      }
      setDataDirTarget(target);
      if (dataDir && target === dataDir.path) {
        onNotify(t("settings.dataDirSame"));
        return;
      }
      setDataDirDone(false);
      setDataDirConfirm(true);
    } catch (error) {
      onNotify(String(error));
    }
  };

  const startMigrate = async () => {
    if (!dataDirTarget) return;
    setDataDirMigrating(true);
    try {
      const result = await api.dataDirMigrate({ target: dataDirTarget });
      setDataDirConfirm(false);
      setDataDirDone(true);
      onNotify(result.restartRequired ? t("settings.dataDirDone") : t("settings.dataDirMigrating"));
    } catch (error) {
      onNotify(String(error));
    } finally {
      setDataDirMigrating(false);
    }
  };

  useEffect(() => {
    if (!initialSection) return;
    setSection(initialSection as SectionKey);
  }, [initialSection]);

  useEffect(() => {
    let alive = true;
    void api
      .msAuthStatus()
      .then((status) => {
        if (alive) setMsClientStoredInvalid(Boolean(status.invalid));
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [settings.msClientId]);

  // 滑块拖动时先动本地值，停手 160ms 后再落盘，避免每次像素都写一次设置
  const [draft, setDraft] = useState({
    concurrent: settings.concurrentDownloads,
    perFile: settings.perFileConnections,
  });
  const pending = useRef<Partial<Settings> | null>(null);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    setDraft({ concurrent: settings.concurrentDownloads, perFile: settings.perFileConnections });
  }, [settings.concurrentDownloads, settings.perFileConnections]);

  // 卸载前把还在防抖窗口里的滑块值补写一次，避免丢设置
  useEffect(() => {
    return () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
      if (pending.current) onChange(pending.current);
    };
  }, [onChange]);

  const commitNumber = (key: "concurrentDownloads" | "perFileConnections", value: number) => {
    setDraft((prev) => (key === "concurrentDownloads" ? { ...prev, concurrent: value } : { ...prev, perFile: value }));
    pending.current = { ...(pending.current ?? {}), [key]: value };
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const patch = pending.current;
      pending.current = null;
      if (patch) onChange(patch);
    }, 160);
  };

  const query = search.trim().toLowerCase();
  const matches = (text: string) => text.toLowerCase().includes(query);

  const visibleGroups = (def: SectionDef): GroupDef[] =>
    query.length === 0
      ? def.groups
      : def.groups.filter(
          (group) => matches(t(group.titleKey)) || matches(t(def.labelKey)) || group.keywords.some((keyword) => matches(keyword)),
        );

  const visibleSections = SECTIONS.filter((def) => query.length === 0 || matches(t(def.labelKey)) || visibleGroups(def).length > 0);

  // 搜索打字时把当前分组切到第一个命中的分组，清空搜索后不再跳回去
  useEffect(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return;
    const hit = (value: string) => value.toLowerCase().includes(needle);
    const first = SECTIONS.find(
      (def) => hit(t(def.labelKey)) || def.groups.some((group) => hit(t(group.titleKey)) || group.keywords.some(hit)),
    );
    if (first) setSection(first.key);
  }, [search, t]);

  const activeSection: SectionKey =
    visibleSections.length === 0
      ? section
      : query.length > 0
        ? visibleSections[0].key
        : visibleSections.some((def) => def.key === section)
          ? section
          : visibleSections[0].key;
  const activeDef = SECTIONS.find((def) => def.key === activeSection) ?? SECTIONS[0];
  const shownGroups = new Set(visibleGroups(activeDef).map((group) => group.id));
  const showGroup = (id: string) => shownGroups.has(id);

  // ---- 镜像源 ----
  const probeSource = async (target: SourceProbeTarget) => {
    if (target.id === "custom" && (target.base ?? "").trim().length === 0) {
      setProbes((prev) => ({ ...prev, custom: { id: "custom", ok: false, latencyMs: null, message: t("settings.mirrorNeedUrl") } }));
      return;
    }
    setProbes((prev) => ({ ...prev, [target.id]: "probing" }));
    try {
      const result = await api.sourcesProbe(target);
      setProbes((prev) => ({ ...prev, [target.id]: result }));
    } catch (error) {
      setProbes((prev) => ({
        ...prev,
        [target.id]: { id: target.id, ok: false, latencyMs: null, message: String(error) },
      }));
    }
  };

  const fastest = useMemo(() => {
    let best: { id: DownloadSourceId; latencyMs: number } | null = null;
    for (const id of ["official", "bmclapi", "custom"] as DownloadSourceId[]) {
      const state = probes[id];
      if (!state || state === "probing" || !state.ok) continue;
      const latency = state.latencyMs ?? 0;
      if (!best || latency < best.latencyMs) best = { id, latencyMs: latency };
    }
    return best;
  }, [probes]);

  const mirrorLabel = (id: DownloadSourceId) => {
    const found = MIRRORS.find((item) => item.id === id);
    return found ? t(found.titleKey) : t("settings.mirrorAuto");
  };

  const probePill = (id: DownloadSourceId) => {
    const state = probes[id];
    if (!state) {
      return <span className="probe-pill" data-probe-pill={id}>{t("settings.probeUntested")}</span>;
    }
    if (state === "probing") {
      return <span className="probe-pill is-busy" data-probe-pill={id}>{t("settings.probing")}</span>;
    }
    if (state.ok) {
      return (
        <span className="probe-pill is-ok" data-probe-pill={id}>
          {t("settings.probeMs", { ms: state.latencyMs ?? 0 })}
        </span>
      );
    }
    return (
      <span className="probe-pill is-off" data-probe-pill={id} title={state.message}>
        {t("settings.probeUnavailable")}
      </span>
    );
  };

  const selectSource = (id: DownloadSourceId) => onChange({ downloadSource: id });

  // ---- 内容服务：翻译接口 / 内容来源 ----
  const canTest = translation.baseUrl.trim().length > 0 && translation.model.trim().length > 0;

  const saveTranslation = () => {
    onChange({
      translationSource: translation.source,
      translationBaseUrl: translation.baseUrl.trim(),
      translationApiKey: translation.apiKey.trim(),
      translationModel: translation.model.trim(),
      translationTemperature: translation.temperature,
      translationMaxTokens: translation.maxTokens,
    });
  };

  const saveTranslationWithToast = () => {
    saveTranslation();
    onNotify(t("settings.translationSaved"));
  };

  const runTranslationTest = async () => {
    setTesting(true);
    setTestResult(null);
    saveTranslation();
    try {
      // 契约：translation_test() 测的是已保存的配置，所以先等设置落盘再测
      await new Promise((resolve) => window.setTimeout(resolve, 90));
      setTestResult(await api.translationTest());
    } catch (error) {
      setTestResult({ ok: false, latencyMs: null, message: String(error) });
    } finally {
      setTesting(false);
    }
  };

  // Client ID：非空必须是 8-4-4-4-12 UUID，否则红字且不写盘；空串允许（等于关掉微软登录）
  const MS_CLIENT_ID_PATTERN = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

  const saveMsClient = () => {
    const value = msClientDraft.trim();
    if (value.length > 0 && !MS_CLIENT_ID_PATTERN.test(value)) {
      setMsClientError(t("settings.msClientIdInvalid"));
      return;
    }
    setMsClientError("");
    setMsClientStoredInvalid(false);
    onChange({ msClientId: value });
    onNotify(t("settings.msClientIdSaved"));
  };

  const saveCurseforge = () => {
    onChange({ curseforgeApiKey: curseforgeKey.trim() });
    onNotify(t("settings.curseforgeSaved"));
  };

  const contentSource: ResourceSourceId = CONTENT_SOURCES.some((item) => item.id === settings.resourceSource)
    ? settings.resourceSource
    : "modrinth";

  const eyeIcon = (visible: boolean) => (
    // TODO(M3-1): 图标素材里没有眼睛，先用内联 SVG
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      {visible ? (
        <>
          <path d="M2 12s3.6-6 10-6 10 6 10 6-3.6 6-10 6-10-6-10-6z" />
          <circle cx="12" cy="12" r="2.6" />
        </>
      ) : (
        <>
          <path d="M2 12s3.6-6 10-6 10 6 10 6-3.6 6-10 6-10-6-10-6z" />
          <path d="M4 20 20 4" />
        </>
      )}
    </svg>
  );

  // ---------------------------------------------------------------- 各分组

  const activeFont = fonts.find((font) => font.family === settings.uiFont);
  const previewStack = fontStack(settings.uiFont, activeFont?.aliases);

  /**
   * 换字体：后端列出来的字体视为可用（后端已经筛过能显示中文的系统字体）；
   * 只有列表里没有的名字才现场量一次，量不出宽度就当不可用：toast + 不写盘。
   */
  const pickFont = (family: string) => {
    const known = fonts.some((font) => font.family === family);
    if (!known && !isFontAvailable(family)) {
      setFontError(true);
      onNotify(t("settings.fontNotAvailable"));
      return;
    }
    setFontError(false);
    onChange({ uiFont: family });
  };

  const fontGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.font")}</div>
      <div className="row">
        <div className="row__hint">{fonts.length === 0 ? t("settings.fontUnavailable") : t("settings.fontHint")}</div>
        <div className="card-row">
          <select
            className="font-picker"
            value={settings.uiFont}
            data-font-picker
            disabled={fonts.length === 0}
            onChange={(event) => pickFont(event.target.value)}
            style={{ fontFamily: previewStack }}
          >
            {fonts.length === 0 ? <option value={settings.uiFont}>{settings.uiFont}</option> : null}
            {fonts.map((font) => (
              <option key={font.family} value={font.family} style={{ fontFamily: fontStack(font.family, font.aliases) }}>
                {font.displayName ? font.displayName + "（" + font.family + "）" : font.family}
              </option>
            ))}
          </select>
          <button
            onClick={() => {
              setFontError(false);
              onChange({ uiFont: DEFAULT_UI_FONT });
            }}
          >
            {t("settings.fontReset")}
          </button>
        </div>
      </div>

      <div
        className={"font-preview" + (fontError ? " is-error" : "")}
        data-font-preview
        style={{ fontFamily: previewStack }}
      >
        {t("settings.fontPreview")}
      </div>

      {fontMissing || fontError ? (
        <div className="chip" role="status" data-font-warning>
          {t("settings.fontMissing")}
        </div>
      ) : null}
    </section>
  );

  const themeGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.groupTheme")}</div>
      <div className="segmented" role="group" aria-label={t("settings.groupTheme")}>
        <button
          className={"segmented__item" + (settings.theme === "light" ? " is-active" : "")}
          aria-pressed={settings.theme === "light"}
          onClick={() => onChange({ theme: "light" })}
        >
          {t("settings.themeLight")}
        </button>
        <button
          className={"segmented__item" + (settings.theme === "dark" ? " is-active" : "")}
          aria-pressed={settings.theme === "dark"}
          onClick={() => onChange({ theme: "dark" })}
        >
          {t("settings.themeDark")}
        </button>
      </div>
    </section>
  );

  // 主题色：预设 + 自定义（自定义走系统取色器，两者共用同一套 --accent 变量）
  const accentValue = normalizeAccent(settings.themeColor ?? settings.accent);
  const customActive = !isPresetAccent(accentValue);
  const accentGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.accent")}</div>
      <div className="row__hint">{t("settings.accentHint")}</div>
      <div className="accent-swatches">
        {ACCENT_PRESETS.map((preset) => (
          <button
            key={preset.id}
            className={"accent-swatch" + (isPresetAccent(accentValue) && accentValue.toLowerCase() === preset.value.toLowerCase() ? " is-active" : "")}
            style={{ background: preset.value }}
            title={preset.label}
            aria-label={preset.label}
            data-accent-preset={preset.id}
            onClick={() => onChange({ accent: preset.value, themeColor: preset.value })}
          />
        ))}
        <label
          className={"accent-swatch accent-swatch--custom" + (customActive ? " is-active" : "")}
          title={t("settings.accentCustom")}
          aria-label={t("settings.accentCustom")}
          data-accent-custom
          style={customActive ? { background: accentValue } : undefined}
        >
          <input
            type="color"
            value={accentValue}
            data-accent-color
            aria-label={t("settings.accentCustom")}
            onChange={(event) => onChange({ accent: event.target.value, themeColor: event.target.value })}
          />
          <span className="accent-swatch__plus" aria-hidden="true" style={customActive ? { color: "#ffffff" } : undefined}>
            {customActive ? "" : "+"}
          </span>
        </label>
      </div>
      {customActive ? (
        <div className="row__hint" data-accent-custom-value>
          {t("settings.accentCustomValue", { value: accentValue })}
        </div>
      ) : null}
    </section>
  );

  const javaGroup = (
    <div className="settings-java">
      <div className="row__hint">{t("settings.javaHint")}</div>
      <JavaPanel settings={settings} onChange={onChange} notify={onNotify} />
    </div>
  );

  const parallelGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.parallel")}</div>

      <div className="range-field">
        <div className="range-field__head">
          <span>{t("settings.concurrent")}：</span>
          <strong data-draft="concurrent">{draft.concurrent}</strong>
        </div>
        <input
          type="range"
          min={1}
          max={32}
          step={1}
          aria-label={t("settings.concurrent")}
          data-setting="concurrentDownloads"
          value={draft.concurrent}
          onChange={(event) => commitNumber("concurrentDownloads", Number(event.target.value))}
        />
        <div className="row__hint">{t("settings.concurrentHint")}</div>
      </div>

      <div className="range-field">
        <div className="range-field__head">
          <span>{t("settings.perFile")}：</span>
          <strong data-draft="perFile">{draft.perFile}</strong>
        </div>
        <input
          type="range"
          min={1}
          max={16}
          step={1}
          aria-label={t("settings.perFile")}
          data-setting="perFileConnections"
          value={draft.perFile}
          onChange={(event) => commitNumber("perFileConnections", Number(event.target.value))}
        />
        <div className="row__hint">{t("settings.perFileHint")}</div>
      </div>
    </section>
  );

  const mirrorGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.mirror")}</div>

      <button
        type="button"
        className={"mirror-auto" + (settings.downloadSource === "auto" ? " is-active" : "")}
        data-source="auto"
        aria-pressed={settings.downloadSource === "auto"}
        onClick={() => selectSource("auto")}
      >
        <span className="mirror-auto__mark" aria-hidden="true" />
        <span className="mirror-auto__text">
          <span className="mirror-auto__title">{t("settings.mirrorAuto")}</span>
          <span className="row__hint">
            {fastest ? t("settings.mirrorAutoHit", { name: mirrorLabel(fastest.id) }) : t("settings.mirrorAutoPending")}
          </span>
        </span>
      </button>

      <div className="mirror-list" role="radiogroup" aria-label={t("settings.mirror")}>
        {MIRRORS.map((mirror) => (
          <div
            key={mirror.id}
            role="radio"
            tabIndex={0}
            aria-checked={settings.downloadSource === mirror.id}
            data-source={mirror.id}
            className={"mirror-card" + (settings.downloadSource === mirror.id ? " is-active" : "")}
            onClick={() => selectSource(mirror.id)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                selectSource(mirror.id);
              }
            }}
          >
            <span className="mirror-card__mark" aria-hidden="true" />
            <div className="mirror-card__body">
              <div className="mirror-card__title">{t(mirror.titleKey)}</div>
              {mirror.descKey ? (
                <div className="mirror-card__desc">{t(mirror.descKey)}</div>
              ) : (
                <input
                  type="text"
                  className="mirror-card__input"
                  data-mirror-input="custom"
                  value={settings.customMirror}
                  placeholder={t("settings.mirrorCustomPlaceholder")}
                  aria-label={t("settings.mirrorCustom")}
                  onClick={(event) => event.stopPropagation()}
                  onFocus={() => selectSource("custom")}
                  onChange={(event) => onChange({ customMirror: event.target.value, downloadSource: "custom" })}
                />
              )}
            </div>
            <button
              type="button"
              className="mirror-card__probe"
              data-probe={mirror.id}
              onClick={(event) => {
                event.stopPropagation();
                void probeSource({ id: mirror.id, base: mirror.id === "custom" ? settings.customMirror : undefined });
              }}
            >
              {t("settings.probe")}
            </button>
            {probePill(mirror.id)}
          </div>
        ))}
      </div>

      <div className="row__hint">{t("settings.mirrorNote")}</div>
    </section>
  );

  const downloadCenterGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.downloadCenter")}</div>
      <div className="row__hint">{t("settings.downloadCenterHint")}</div>
      <DownloadCenter variant="inline" />
    </section>
  );

  const translationGroup = (
    <section className="card settings-card" data-settings-translation>
      <div className="settings-card__title">{t("settings.translateEnabledTitle")}</div>
      <div className="settings-switch-row" style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <label className="switch">
          <input
            type="checkbox"
            checked={translateEnabled}
            data-translate-enabled
            aria-label={t("settings.translateEnabledTitle")}
            onChange={(event) => onChange({ translateEnabled: event.target.checked })}
          />
          <span className="switch__track" aria-hidden="true">
            <span className="switch__dot" />
          </span>
        </label>
        <span className="settings-switch-row__state" style={{ fontSize: 12, fontWeight: 600, color: "var(--accent-strong)" }} data-translate-enabled-state>
          {translateEnabled ? t("settings.translateEnabledOn") : t("settings.translateEnabledOff")}
        </span>
      </div>
      <div className="row__hint">{t("settings.translateEnabledHint")}</div>

      <div
        className={"translation-body" + (translateEnabled ? "" : " is-disabled")}
        aria-disabled={!translateEnabled}
        data-translation-body
        style={{
          display: "grid",
          gap: 10,
          opacity: translateEnabled ? 1 : 0.45,
          pointerEvents: translateEnabled ? "auto" : "none",
        }}
      >
      <div className="row__hint">{t("settings.translationHint")}</div>

      <div className="segmented" role="radiogroup" aria-label={t("settings.translation")}>
        <button
          type="button"
          role="radio"
          data-translation-source="builtin"
          aria-checked={translation.source === "builtin"}
          className={"segmented__item" + (translation.source === "builtin" ? " is-active" : "")}
          onClick={() => setTranslation((prev) => ({ ...prev, source: "builtin" }))}
        >
          {t("settings.translationBuiltin")}
        </button>
        <button
          type="button"
          role="radio"
          data-translation-source="custom"
          aria-checked={translation.source === "custom"}
          className={"segmented__item" + (translation.source === "custom" ? " is-active" : "")}
          onClick={() => setTranslation((prev) => ({ ...prev, source: "custom" }))}
        >
          {t("settings.translationCustom")}
        </button>
      </div>

      {translation.source === "custom" ? (
        <div className="translation-fields">
          <div className="field">
            <label htmlFor="translation-base">{t("settings.translationBaseUrl")}</label>
            <input
              id="translation-base"
              type="text"
              data-translation="baseUrl"
              value={translation.baseUrl}
              placeholder={t("settings.translationBaseUrlPlaceholder")}
              onChange={(event) => setTranslation((prev) => ({ ...prev, baseUrl: event.target.value }))}
            />
            <div className="row__hint">{t("settings.translationBaseUrlHint")}</div>
          </div>

          <div className="field">
            <label htmlFor="translation-key">{t("settings.translationApiKey")}</label>
            <div className="secret-field">
              <input
                id="translation-key"
                type={showTranslationKey ? "text" : "password"}
                data-translation="apiKey"
                autoComplete="off"
                spellCheck={false}
                value={translation.apiKey}
                onChange={(event) => setTranslation((prev) => ({ ...prev, apiKey: event.target.value }))}
              />
              <button
                type="button"
                className={"icon-button secret-field__toggle" + (showTranslationKey ? " is-active" : "")}
                aria-pressed={showTranslationKey}
                title={showTranslationKey ? t("settings.secretHide") : t("settings.secretShow")}
                aria-label={showTranslationKey ? t("settings.secretHide") : t("settings.secretShow")}
                onClick={() => setShowTranslationKey((value) => !value)}
              >
                {eyeIcon(showTranslationKey)}
              </button>
            </div>
            <div className="row__hint">{t("settings.translationApiKeyHint")}</div>
          </div>

          <div className="field">
            <label htmlFor="translation-model">{t("settings.translationModel")}</label>
            <input
              id="translation-model"
              type="text"
              data-translation="model"
              value={translation.model}
              placeholder={t("settings.translationModelPlaceholder")}
              onChange={(event) => setTranslation((prev) => ({ ...prev, model: event.target.value }))}
            />
          </div>

          <div className="field-pair">
            <div className="field">
              <label htmlFor="translation-temp">{t("settings.translationTemperature")}</label>
              <input
                id="translation-temp"
                type="number"
                min={0}
                max={2}
                step={0.1}
                data-translation="temperature"
                value={translation.temperature === 0 ? "" : translation.temperature}
                placeholder="0.2"
                onChange={(event) =>
                  setTranslation((prev) => ({ ...prev, temperature: Number(event.target.value) || 0 }))
                }
              />
            </div>
            <div className="field">
              <label htmlFor="translation-tokens">{t("settings.translationMaxTokens")}</label>
              <input
                id="translation-tokens"
                type="number"
                min={0}
                step={64}
                data-translation="maxTokens"
                value={translation.maxTokens === 0 ? "" : translation.maxTokens}
                placeholder="512"
                onChange={(event) =>
                  setTranslation((prev) => ({ ...prev, maxTokens: Number(event.target.value) || 0 }))
                }
              />
            </div>
          </div>
          <div className="row__hint">{t("settings.translationOptionalHint", { temperature: "0.2", tokens: "512" })}</div>

          <div className="field-actions">
            <button
              type="button"
              data-test="translation"
              disabled={!canTest || testing}
              onClick={() => void runTranslationTest()}
            >
              {testing ? t("settings.translationTesting") : t("settings.translationTest")}
            </button>
            <button type="button" className="pill-button" data-save="translation" onClick={saveTranslationWithToast}>
              {t("common.save")}
            </button>
          </div>

          {testResult ? (
            <div className={"test-result" + (testResult.ok ? " is-ok" : " is-off")} role="status" data-test-result>
              {testResult.ok
                ? t("settings.translationTestOk", { ms: testResult.latencyMs ?? 0, message: testResult.message })
                : testResult.message}
            </div>
          ) : null}
        </div>
      ) : (
        <div className="test-result is-plain" role="status">
          {t("settings.translationTestBuiltinHint")}
        </div>
      )}

      <div className="row__hint">{t("settings.translationNote")}</div>
      </div>
      {translateEnabled ? null : <div className="row__hint" data-translate-off-hint>{t("settings.translateEnabledOffHint")}</div>}
    </section>
  );

  const sayingApiGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.sayingApi")}</div>
      <div className="secret-field">
        <input
          type="text"
          data-saying-api="url"
          spellCheck={false}
          aria-label={t("settings.sayingApi")}
          value={sayingApiDraft}
          placeholder={t("settings.sayingApiPlaceholder")}
          onChange={(event) => setSayingApiDraft(event.target.value)}
        />
        <button
          type="button"
          className="pill-button"
          data-save="sayingApi"
          onClick={() => {
            onChange({ sayingApiUrl: sayingApiDraft.trim() });
            onNotify(t("settings.sayingApiSaved"));
          }}
        >
          {t("common.save")}
        </button>
      </div>
      <div className="row__hint">{t("settings.sayingApiHint")}</div>
    </section>
  );

  const contentSourceGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.contentSource")}</div>
      <div className="row__hint">{t("settings.contentSourceHint")}</div>

      <div className="mirror-list" role="radiogroup" aria-label={t("settings.contentSource")}>
        {CONTENT_SOURCES.map((item) => (
          <div
            key={item.id}
            role="radio"
            tabIndex={0}
            data-content-source={item.id}
            aria-checked={contentSource === item.id}
            className={"mirror-card" + (contentSource === item.id ? " is-active" : "")}
            onClick={() => onChange({ resourceSource: item.id })}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onChange({ resourceSource: item.id });
              }
            }}
          >
            <span className="mirror-card__mark" aria-hidden="true" />
            <div className="mirror-card__body">
              <div className="mirror-card__title">{t(item.titleKey)}</div>
              <div className="mirror-card__desc">{t(item.hintKey)}</div>
            </div>
          </div>
        ))}
      </div>

      <div className="row__hint" data-content-mirror-note>
        {t("settings.contentSourceMirrorNote")}
      </div>

      {contentSource === "curseforge" ? (
        <div className="row__hint" data-cf-key-hint>
          {t("settings.cfKeyInSource")}
        </div>
      ) : null}
    </section>
  );

  const serverMirrorGroup = (
    <section className="card settings-card" data-server-mirror-group>
      <div className="settings-card__title">{t("settings.serverMirror")}</div>
      <div className="field">
        <input
          type="text"
          data-server-mirror
          value={settings.serverMirrorUrl}
          placeholder="https://mirror.example.com/server"
          onChange={(event) => onChange({ serverMirrorUrl: event.target.value })}
        />
        <div className="row__hint">{t("settings.serverMirrorHint")}</div>
      </div>
    </section>
  );

  const behaviorGroup = (
    <section className="card settings-card" data-behavior-group>
      <div className="settings-card__title">{t("settings.groupBehavior")}</div>
      <div className="field">
        <label>{t("settings.closeAction")}</label>
        <div className="segmented segmented--sm">
          {(["ask", "minimize", "exit"] as const).map((item) => (
            <button
              key={item}
              type="button"
              className={"segmented__item" + (settings.closeAction === item ? " is-active" : "")}
              data-close-action={item}
              onClick={() => onChange({ closeAction: item })}
            >
              {t("settings.close." + item)}
            </button>
          ))}
        </div>
      </div>
      <div className="field">
        <label>{t("settings.updateWay")}</label>
        <div className="segmented segmented--sm">
          {(["auto", "manual", "off"] as const).map((item) => (
            <button
              key={item}
              type="button"
              className={"segmented__item" + (settings.updateMode === item ? " is-active" : "")}
              data-update-way={item}
              onClick={() => onChange({ updateMode: item })}
            >
              {t("settings.update" + item.charAt(0).toUpperCase() + item.slice(1))}
            </button>
          ))}
        </div>
      </div>
      <div className="row__hint">{t("settings.updateHint")}</div>
      <div className="field" data-force-chinese>
        <label htmlFor="setting-force-chinese">{t("settings.forceChinese")}</label>
        <div className="settings-switch-row" style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <label className="switch">
            <input
              id="setting-force-chinese"
              type="checkbox"
              checked={settings.forceChinese}
              data-setting="forceChinese"
              aria-label={t("settings.forceChinese")}
              onChange={(event) => onChange({ forceChinese: event.target.checked })}
            />
            <span className="switch__track" aria-hidden="true">
              <span className="switch__dot" />
            </span>
          </label>
          <span
            className="settings-switch-row__state"
            style={{ fontSize: 12, fontWeight: 600, color: "var(--accent-strong)" }}
            data-force-chinese-state
          >
            {settings.forceChinese ? t("settings.forceChineseOn") : t("settings.forceChineseOff")}
          </span>
        </div>
        <div className="row__hint">{t("settings.forceChineseHint")}</div>
      </div>
      <div className="row__hint">{t("settings.behaviorHint")}</div>
    </section>
  );

  const dataDirGroup = (
    <section className="card settings-card" data-datadir-group>
      <div className="settings-card__title">{t("settings.groupDataDir")}</div>
      <div className="field">
        <div className="data-dir__row">
          <span className="data-dir__path mono" data-data-dir title={dataDir?.path ?? ""}>
            {dataDir?.path ?? t("common.loading")}
          </span>
          <button type="button" className="pill-outline" data-action="data-dir-open" onClick={() => void openDataDir()}>
            {t("settings.dataDirOpen")}
          </button>
          <button
            type="button"
            className="pill-outline"
            data-action="data-dir-change"
            disabled={Boolean(dataDir && dataDirTarget && dataDirTarget === dataDir.path)}
            onClick={() => void pickDataDir()}
          >
            {t("settings.dataDirChange")}
          </button>
        </div>
        <div className="row__hint">{t("settings.dataDirHint")}</div>
        <div className="row__hint">{t("settings.dataDirMoveHint")}</div>
        {dataDir && dataDirTarget && dataDirTarget === dataDir.path ? (
          <div className="field-error" data-data-dir-same>
            {t("settings.dataDirSame")}
          </div>
        ) : null}
      </div>
    </section>
  );

  const msLoginGroup = (
    <section className="card settings-card" data-ms-login-group>
      <div className="settings-card__title">{t("settings.msLogin")}</div>
      <div className="row__hint">{t("settings.msLoginHint")}</div>
      {msClientStoredInvalid ? (
        <div className="notice" data-ms-client-invalid>
          {t("settings.msClientIdInvalidStored")}
        </div>
      ) : null}
      <div className="field">
        <label htmlFor="ms-client-id">{t("settings.msClientId")}</label>
        <div className="secret-field">
          <input
            id="ms-client-id"
            type={showMsClient ? "text" : "password"}
            data-ms-client-id
            autoComplete="off"
            spellCheck={false}
            placeholder={t("settings.msClientIdPlaceholder")}
            value={msClientDraft}
            onChange={(event) => {
              setMsClientDraft(event.target.value);
              setMsClientError("");
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") saveMsClient();
            }}
          />
          <button
            type="button"
            className={"icon-button secret-field__toggle" + (showMsClient ? " is-active" : "")}
            aria-pressed={showMsClient}
            title={showMsClient ? t("settings.secretHide") : t("settings.secretShow")}
            aria-label={showMsClient ? t("settings.secretHide") : t("settings.secretShow")}
            onClick={() => setShowMsClient((value) => !value)}
          >
            {eyeIcon(showMsClient)}
          </button>
          <button type="button" className="pill-button" data-save="ms-client-id" onClick={saveMsClient}>
            {t("common.save")}
          </button>
        </div>
        {msClientError ? (
          <div className="field-error" data-ms-client-error>
            {msClientError}
          </div>
        ) : (
          <div className="row__hint">{t("settings.msClientIdNote")}</div>
        )}
      </div>
    </section>
  );

  const storageGroup = (
    <div className="settings-storage">
      <section className="card settings-card">
        <div className="settings-card__title">{t("settings.storage")}</div>
        <div className="row__hint">{t("settings.storageHint")}</div>
        <div className="field">
          <label>{t("settings.dataRoot")}</label>
          <div className="mono">{dataRoot || "—"}</div>
        </div>
      </section>
      <StoragePanel notify={onNotify} />
    </div>
  );

  /** CurseForge API Key：常驻卡片，和「内容来源」选中什么无关。 */
  const curseforgeKeyGroup = (
    <section className="card settings-card" data-cf-key-card>
      <div className="cf-key__head">
        <span className="settings-card__title">{t("settings.cfKeyTitle")}</span>
        <span className="row__hint">{t("settings.cfKeyOptional")}</span>
      </div>
      <div className="field">
        <label htmlFor="cf-api-key">{t("settings.cfKeyLabel")}</label>
        <div className="secret-field">
          <input
            id="cf-api-key"
            type={showCurseforgeKey ? "text" : "password"}
            data-cf-api-key
            autoComplete="off"
            spellCheck={false}
            placeholder={t("settings.cfKeyPlaceholder")}
            value={curseforgeKey}
            onChange={(event) => setCurseforgeKey(event.target.value)}
          />
          <button
            type="button"
            className={"icon-button secret-field__toggle" + (showCurseforgeKey ? " is-active" : "")}
            aria-pressed={showCurseforgeKey}
            title={showCurseforgeKey ? t("settings.secretHide") : t("settings.secretShow")}
            aria-label={showCurseforgeKey ? t("settings.secretHide") : t("settings.secretShow")}
            data-action="cf-key-eye"
            onClick={() => setShowCurseforgeKey((value) => !value)}
          >
            {eyeIcon(showCurseforgeKey)}
          </button>
          <button type="button" className="pill-button" data-save="curseforge" onClick={saveCurseforge}>
            {t("common.save")}
          </button>
        </div>
        <div className="row__hint">{t("settings.cfKeyNote")}</div>
      </div>
    </section>
  );

  const aboutGroup = (
    <section className="card settings-card">
      <div className="settings-card__title">{t("settings.aboutTitle")}</div>
      <div className="row__hint">{t("settings.aboutHint")}</div>
      <div className="row">
        <span>{t("settings.aboutVersion")}</span>
        <span className="mono">{appInfo?.version ?? "1.0.0"}</span>
      </div>
      <div className="row">
        <span>{t("settings.aboutIdentifier")}</span>
        <span className="mono">{appInfo?.identifier ?? "—"}</span>
      </div>
    </section>
  );

  return (
    <div className="settings-layout">
      <aside className="settings-side">
        <input
          type="text"
          className="settings-search"
          data-settings-search="q"
          value={search}
          placeholder={t("settings.search")}
          aria-label={t("settings.search")}
          onChange={(event) => setSearch(event.target.value)}
        />
        <nav className="settings-nav" aria-label={t("settings.title")}>
          {visibleSections.map((def) => (
            <button
              key={def.key}
              type="button"
              data-section={def.key}
              aria-current={def.key === activeSection ? "page" : undefined}
              className={"settings-nav__item" + (def.key === activeSection ? " is-active" : "")}
              onClick={() => setSection(def.key)}
            >
              {t(def.labelKey)}
            </button>
          ))}
        </nav>
        <div className="settings-version">
          <img className="settings-version__logo" src={brand.logo} alt="" data-brand-logo />
          <div className="settings-version__text">
            <div className="settings-version__name">{t("app.name")}</div>
            <div className="settings-version__version">
              {t("settings.versionLine", { version: appInfo?.version ?? "1.0.0" })}
            </div>
          </div>
        </div>
      </aside>

      <div className="settings-main">
        {visibleSections.length === 0 ? (
          <div className="empty-hint">{t("settings.searchEmpty")}</div>
        ) : (
          <>
            <h2 className="settings-main__title">{t(activeDef.labelKey)}</h2>

            {activeSection === "general" ? (
              <>
                {showGroup("font") ? fontGroup : null}
                {showGroup("behavior") ? behaviorGroup : null}
                {showGroup("dataDir") ? dataDirGroup : null}
              </>
            ) : null}

            {activeSection === "appearance" ? (
              <>
                {showGroup("theme") ? themeGroup : null}
                {showGroup("accent") ? accentGroup : null}
                {showGroup("background") ? (
                  <BackgroundPanel settings={settings} onChange={onChange} notify={onNotify} />
                ) : null}
              </>
            ) : null}

            {activeSection === "java" ? showGroup("java") ? javaGroup : null : null}

            {activeSection === "download" ? (
              <>
                {showGroup("parallel") ? parallelGroup : null}
                {showGroup("mirror") ? mirrorGroup : null}
                {showGroup("center") ? downloadCenterGroup : null}
                {showGroup("serverMirror") ? serverMirrorGroup : null}
              </>
            ) : null}

            {activeSection === "content" ? (
              <>
                {showGroup("translation") ? translationGroup : null}
                {showGroup("sayingApi") ? sayingApiGroup : null}
                {showGroup("contentSource") ? contentSourceGroup : null}
                {showGroup("curseforgeKey") ? curseforgeKeyGroup : null}
                {showGroup("msLogin") ? msLoginGroup : null}
              </>
            ) : null}

            {activeSection === "storage" ? (showGroup("storage") ? storageGroup : null) : null}
            {activeSection === "about" ? (showGroup("about") ? aboutGroup : null) : null}
          </>
        )}
      </div>

      {dataDirDone ? (
        <Modal
          title={t("settings.dataDirDone")}
          onClose={() => setDataDirDone(false)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setDataDirDone(false)}>
                {t("common.close")}
              </button>
              <button
                type="button"
                className="pill-button"
                data-action="data-dir-restart"
                onClick={() => void api.appRestart().catch((error) => onNotify(String(error)))}
              >
                {t("settings.dataDirRestart")}
              </button>
            </>
          }
        >
          <div className="field">
            <span className="row__hint">{t("settings.dataDirFrom")}</span>
            <span className="mono">{dataDir?.path ?? "—"}</span>
            <span className="row__hint">{t("settings.dataDirTo")}</span>
            <span className="mono" data-data-dir-target>
              {dataDirTarget}
            </span>
          </div>
          <div className="row__hint">{t("settings.dataDirMigrating")}</div>
        </Modal>
      ) : null}

      {dataDirConfirm ? (
        <Modal
          title={t("settings.dataDirConfirmTitle")}
          onClose={() => setDataDirConfirm(false)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setDataDirConfirm(false)}>
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className="pill-button"
                data-action="data-dir-migrate"
                disabled={dataDirMigrating}
                onClick={() => void startMigrate()}
              >
                {t("settings.dataDirStart")}
              </button>
            </>
          }
        >
          <div className="field">
            <span className="row__hint">{t("settings.dataDirFrom")}</span>
            <span className="mono">{dataDir?.path ?? "—"}</span>
            <span className="row__hint">{t("settings.dataDirTo")}</span>
            <span className="mono" data-data-dir-target>
              {dataDirTarget}
            </span>
            <span className="row__hint" data-data-dir-size>
              {t("settings.dataDirSize")}：{formatBytes(dataDir?.sizeBytes ?? 0)}
              {dataDir ? " · " + t("settings.dataDirFiles", { count: dataDir.fileCount }) : ""}
            </span>
          </div>
          <div className="row__hint">{t("settings.dataDirMoveHint")}</div>
        </Modal>
      ) : null}
    </div>
  );
}
