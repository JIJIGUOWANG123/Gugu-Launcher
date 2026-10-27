/**
 * 所有后端调用的唯一出口：页面只 import 这里的函数，不直接 invoke。
 * 浏览器里（没有 Tauri 运行时）自动回落到 mock，保证 `vite dev` 与 `vite build` 都能跑。
 */
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { builtinSkins } from "../assets/skins";
import { normalizeManifest } from "./versions";
import type {
  Account,
  AppInfo,
  AuthlibStatus,
  BackgroundItem,
  BackgroundPage,
  CapesListResult,
  CapeState,
  ContentFileKind,
  ContentDependency,
  ExternalImportResult,
  ExternalScanResult,
  ContentItem,
  ContentKindTab,
  ContentProvider,
  ContentSearchQuery,
  ContentSearchResult,
  ContentType,
  DownloadSource,
  DownloadTask,
  FontChoice,
  GithubDownloadRequest,
  InstallProgress,
  Instance,
  InstanceCreate,
  InstanceSettings,
  JavaRuntime,
  JavaSource,
  JavaSourceProbe,
  JavaVersion,
  LoaderVersion,
  DataDirInfo,
  LauncherEvent,
  LogFile,
  ModApplyResult,
  ModCheckResult,
  ModFileItem,
  ModUpdateItem,
  ModpackInstallRequest,
  MsAuthStatus,
  MsLoginPoll,
  MsProfile,
  ThirdpartySkin,
  NewsItem,
  MsLoginStart,
  PathsLayout,
  ProjectDetail,
  SaveItem,
  Settings,
  SayingResult,
  ServerConfigResult,
  ServerContentItem,
  ServerCore,
  ServerCreateRequest,
  ServerConsoleState,
  ServerInstance,
  ServerSaveItem,
  ServerState,
  ServerVersion,
  StorageKey,
  StorageStatItem,
  StorageStats,
  SkinItem,
  SkinLookupResult,
  SourceProbe,
  TranslationTestResult,
  SourceProbeTarget,
  TeraState,
  TeraStatus,
  TranslateResult,
  UpdateCheckResult,
  UpdateStatus,
  VersionChannel,
  VersionItem,
  VersionManifest,
} from "./types";

const VERSION = "1.0.0";
const PRODUCT = "咕咕启动器";
const PRODUCT_EN = "Gugu Launcher";
const IDENTIFIER = "com.gugu.launcher";

/** Tauri 2 在 window 上注入 __TAURI_INTERNALS__；浏览器预览时没有。 */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export const RUNTIME_MODE: "tauri" | "browser" = isTauri() ? "tauri" : "browser";

// --------------------------------------------------------- invoke 计数 / 缓存

/** 每个命令被调用了多少次：只给自检用（window.__guguInvokeStats），界面上不显示。 */
const invokeCounts = new Map<string, number>();

export function invokeStats(): Record<string, number> {
  const out: Record<string, number> = {};
  invokeCounts.forEach((value, key) => {
    out[key] = value;
  });
  return out;
}

export function resetInvokeStats(): void {
  invokeCounts.clear();
}

if (typeof window !== "undefined") {
  const host = window as unknown as Record<string, unknown>;
  host.__guguInvokeStats = invokeStats;
  host.__guguResetInvokeStats = resetInvokeStats;
}

/**
 * 重命令的会话级缓存：java_list / fonts_list / background_list / wallpaper_dir_default
 * 这类「一个页面挂载只该问一次」的命令走这里，避免切子页、改设置就重发请求（后端会拉起子进程）。
 */
const responseCache = new Map<string, Promise<unknown>>();

function cached<T>(key: string, loader: () => Promise<T>): Promise<T> {
  const hit = responseCache.get(key);
  if (hit) return hit as Promise<T>;
  const pending = loader().catch((error: unknown) => {
    responseCache.delete(key);
    throw error;
  });
  responseCache.set(key, pending);
  return pending;
}

/** 设置保存后调用：清掉依赖设置的缓存（背景列表跟着壁纸目录走）。 */
export function invalidateApiCache(): void {
  responseCache.clear();
}

/** 统一 invoke：Tauri 里走真后端，浏览器里走 mock。 */
async function call<T>(cmd: string, args: Record<string, unknown> | undefined, mock: () => T | Promise<T>): Promise<T> {
  invokeCounts.set(cmd, (invokeCounts.get(cmd) ?? 0) + 1);
  if (!isTauri()) return mock();
  return (await invoke(cmd, args)) as T;
}

// ---------------------------------------------------------------- mock 层

const MOCK_SETTINGS_KEY = "gugu.mock.settings";

const MOCK_SETTINGS: Settings = {
  language: "zh-CN",
  theme: "light",
  accent: "#f5c845",
  updateMode: "manual",
  downloadSource: "auto",
  concurrentDownloads: 8,
  perFileConnections: 4,
  customMirror: "",
  resourceSource: "modrinth",
  translateEnabled: true,
  translationSource: "builtin",
  translationBaseUrl: "",
  translationApiKey: "",
  translationModel: "",
  translationTemperature: 0,
  translationMaxTokens: 0,
  curseforgeApiKey: "",
  msClientId: "",
  thirdpartyAuthServer: "",
  closeAction: "ask",
  autoDownloadJava: true,
  forceChinese: true,
  serverMirrorUrl: "",
  sayingApiUrl: "",
  githubDownloadDir: "",
  javaPath: "",
  memoryMb: 4096,
  githubAccelerator: "auto",
  uiFont: "Microsoft YaHei",
  appearance: {
    mode: "gradient",
    gradientFrom: "#ffffff",
    gradientTo: "#f5c845",
    localFile: "",
    localMode: "single",
    localIntervalSec: 30,
    apiUrl: "",
    apiMode: "single",
    apiIntervalSec: 60,
    wallpaperFile: "",
    wallpaperDir: "",
    wallpaperMode: "single",
    wallpaperIntervalSec: 60,
  },
};

const MOCK_FONTS: FontChoice[] = [
  { family: "Microsoft YaHei", displayName: "微软雅黑", aliases: ["Microsoft YaHei UI", "微软雅黑"], source: "System" },
  { family: "Microsoft YaHei UI", displayName: "微软雅黑 UI", aliases: ["Microsoft YaHei", "微软雅黑"], source: "System" },
  { family: "SimSun", displayName: "宋体", aliases: ["宋体", "NSimSun"], source: "System" },
  { family: "KaiTi", displayName: "楷体", aliases: ["楷体", "楷体_GB2312"], source: "System" },
  { family: "DengXian", displayName: "等线", aliases: ["等线 Light"], source: "System" },
  { family: "SimHei", displayName: "黑体", aliases: ["黑体"], source: "System" },
];

const mockRoot = "D:\\minecraft\\.mc";

const MOCK_PATHS: PathsLayout = {
  root: mockRoot,
  assets: mockRoot + "\\assets",
  libraries: mockRoot + "\\libraries",
  versions: mockRoot + "\\versions",
  instances: mockRoot + "\\instances",
  runtimes: mockRoot + "\\runtimes",
  skins: mockRoot + "\\skins",
  gameIcons: mockRoot + "\\game-icons",
  cache: mockRoot + "\\cache",
  logs: mockRoot + "\\logs",
  diagnostics: mockRoot + "\\diagnostics",
  settingsFile: mockRoot + "\\settings.json",
  dbFile: mockRoot + "\\gugu.db",
  versionManifestFile: mockRoot + "\\version_manifest.json",
};

const MOCK_VERSIONS: VersionManifest = {
  latestRelease: "1.21.4",
  latestSnapshot: "25w02a",
  fromCache: false,
  fetchedAt: null,
  versions: [
    { id: "1.21.4", versionType: "release", url: "", releaseTime: "2025-01-08T10:00:00+00:00" },
    { id: "1.21.3", versionType: "release", url: "", releaseTime: "2024-11-20T10:00:00+00:00" },
    { id: "1.20.6", versionType: "release", url: "", releaseTime: "2024-06-13T10:00:00+00:00" },
    { id: "1.19.4", versionType: "release", url: "", releaseTime: "2023-03-14T10:00:00+00:00" },
    { id: "1.19.2", versionType: "release", url: "", releaseTime: "2022-08-05T10:00:00+00:00" },
    { id: "1.20.1", versionType: "release", url: "", releaseTime: "2023-06-12T10:00:00+00:00" },
    { id: "25w02a", versionType: "snapshot", url: "", releaseTime: "2025-01-15T10:00:00+00:00" },
    { id: "24w14potato", versionType: "snapshot", url: "", releaseTime: "2024-04-01T10:00:00+00:00" },
    { id: "23w13a_or_b", versionType: "snapshot", url: "", releaseTime: "2023-04-01T10:00:00+00:00" },
    { id: "22w13oneblockatatime", versionType: "snapshot", url: "", releaseTime: "2022-04-01T10:00:00+00:00" },
    { id: "20w14infinite", versionType: "snapshot", url: "", releaseTime: "2020-04-01T10:00:00+00:00" },
    { id: "3D Shareware v1.34", versionType: "snapshot", url: "", releaseTime: "2019-04-01T10:00:00+00:00" },
    { id: "1.RV-Pre1", versionType: "snapshot", url: "", releaseTime: "2016-03-31T10:00:00+00:00" },
    { id: "15w14a", versionType: "snapshot", url: "", releaseTime: "2015-04-01T10:00:00+00:00" },
    { id: "b1.7.3", versionType: "old_beta", url: "", releaseTime: "2011-07-08T10:00:00+00:00" },
    { id: "a1.2.6", versionType: "old_alpha", url: "", releaseTime: "2010-12-02T10:00:00+00:00" },
  ],
};

let mockInstances: Instance[] = [
  {
    id: 1,
    name: "26.3 Fabric 0.26.1",
    mcVersion: "1.21.4",
    loader: "Fabric",
    loaderVersion: "0.26.1",
    icon: "",
    pinnedHome: true,
    playTimeSecs: 500,
    createdAt: "2025-01-01T00:00:00Z",
    lastPlayedAt: "2025-02-01T00:00:00Z",
  },
  // 1.20 及以上才支持「进入该存档」，这两个实例就是新旧版本两侧的对照
  {
    id: 2,
    name: "1.19.4 原版",
    mcVersion: "1.19.4",
    loader: "Vanilla",
    loaderVersion: "",
    icon: "",
    pinnedHome: false,
    playTimeSecs: 0,
    createdAt: "2025-01-12T00:00:00Z",
    lastPlayedAt: null,
  },
  {
    id: 3,
    name: "1.20.1 生存",
    mcVersion: "1.20.1",
    loader: "Fabric",
    loaderVersion: "0.15.11",
    icon: "",
    pinnedHome: false,
    playTimeSecs: 1200,
    createdAt: "2025-02-03T00:00:00Z",
    lastPlayedAt: "2025-02-10T00:00:00Z",
  },
];

const MOCK_UPDATE_STATUS: UpdateStatus = {
  phase: "uptodate",
  currentVersion: VERSION,
  remoteVersion: null,
  message: "已是最新（1.0.0）",
  checkedAt: null,
  canInstall: false,
};

const MOCK_SOURCES: DownloadSource[] = [
  { id: "official", label: "官方源", description: "直接使用各官方地址", base: "https://launchermeta.mojang.com" },
  { id: "bmclapi", label: "BMCLAPI", description: "https://bmclapi2.bangbang93.com", base: "https://bmclapi2.bangbang93.com" },
  { id: "custom", label: "自定义镜像", description: "需兼容 BMCLAPI 接口", base: "" },
  { id: "auto", label: "自动", description: "按测速选最快", base: "" },
];

/** 浏览器里的假测速：官方最慢、BMCLAPI 最快；example.com / 空地址判为不可用。 */
function mockSourceProbe(target: SourceProbeTarget): Promise<SourceProbe> {
  const base = (target.base ?? "").trim();
  return new Promise((resolve) => {
    window.setTimeout(() => {
      if (target.id === "custom" && base.length === 0) {
        resolve({ id: target.id, ok: false, latencyMs: null, message: "请先填写自定义镜像地址" });
        return;
      }
      // 自检用：打开这个开关后每个源都返回不可用，文案带真后端那种原因（状态码 / 超时）
      if (window.localStorage.getItem("gugu.mock.probe") === "fail") {
        resolve({
          id: target.id,
          ok: false,
          latencyMs: null,
          message: target.id === "official" ? "HTTP 502（Bad Gateway）" : "连接超时（3000ms）",
        });
        return;
      }
      if (base.includes("example.com") || base.includes("invalid")) {
        resolve({ id: target.id, ok: false, latencyMs: null, message: "HTTP 404（Not Found）：镜像地址不存在" });
        return;
      }
      const baseMs = target.id === "bmclapi" ? 34 : target.id === "custom" ? 76 : 148;
      resolve({ id: target.id, ok: true, latencyMs: baseMs + Math.floor(Math.random() * 30), message: "" });
    }, 320);
  });
}

const MOCK_SAYINGS = [
  "愿你挖到的每一块石头，都是意外之喜。",
  "今天的红石信号不会乱跳，放心连。",
  "别急着下矿，先把背包理一理。",
  "方块世界很大，慢慢走也来得及。",
  "手气正好，想开新档就现在。",
];

let mockSayingIndex = 0;

const GITHUB_URL = /^https?:\/\/(www\.)?(github\.com|raw\.githubusercontent\.com|codeload\.github\.com|objects\.githubusercontent\.com)\//i;

/** 从地址末段推文件名；推不出来返回空串（交给后端兜底）。 */
function mockNameFromUrl(raw: string): string {
  try {
    const { pathname } = new URL(raw.trim());
    if (pathname.endsWith("/")) return "";
    return decodeURIComponent(pathname.split("/").filter(Boolean).pop() ?? "");
  } catch {
    return "";
  }
}

/** 浏览器里的假翻译接口测试：地址/模型必填，Key 用 401/长度 模拟各种失败。 */
function mockTranslationTest(settings: Settings): Promise<TranslationTestResult> {
  const base = settings.translationBaseUrl.trim();
  const model = settings.translationModel.trim();
  const key = settings.translationApiKey.trim();
  return new Promise((resolve) => {
    window.setTimeout(() => {
      if (settings.translationSource === "builtin") {
        resolve({ ok: true, latencyMs: 24, message: "使用内置翻译接口" });
        return;
      }
      if (!base) {
        resolve({ ok: false, latencyMs: null, message: "请先填写接口地址" });
        return;
      }
      if (!/^https?:\/\//i.test(base)) {
        resolve({ ok: false, latencyMs: null, message: "接口地址需以 http:// 或 https:// 开头" });
        return;
      }
      if (!model) {
        resolve({ ok: false, latencyMs: null, message: "请先填写模型名" });
        return;
      }
      if (base.includes("timeout")) {
        resolve({ ok: false, latencyMs: null, message: "请求超时（15s）" });
        return;
      }
      if (base.includes("html")) {
        resolve({ ok: false, latencyMs: null, message: "JSON 不符：返回内容不是 OpenAI 兼容格式" });
        return;
      }
      const local = base.includes("localhost") || base.includes("127.0.0.1");
      if (!local && (key.length === 0 || key.includes("401"))) {
        resolve({ ok: false, latencyMs: null, message: "401 未授权：请检查 API Key" });
        return;
      }
      resolve({ ok: true, latencyMs: 8 + Math.floor(Math.random() * 30), message: "ok" });
    }, 360);
  });
}

/**
 * 兼容老 settings.json：mode 若还是旧枚举（url/steam）或未知值，就折成新枚举，
 * 避免界面「四个方案都没选中」又去调后端不认识的 kind。
 */
export function normalizeAppearance(raw: Settings): Settings {
  const appearance = raw.appearance;
  if (!appearance) return raw;
  const legacy: Record<string, string> = { url: "api", steam: "wallpaper" };
  const allowed = ["gradient", "local", "api", "wallpaper"];
  const mode = legacy[appearance.mode] ?? (allowed.indexOf(appearance.mode) >= 0 ? appearance.mode : "gradient");
  const legacyFields = appearance as unknown as Record<string, unknown>;
  const hasLegacy = "steamFile" in legacyFields || "gradientTop" in legacyFields || "url" in legacyFields;
  if (mode === appearance.mode && !hasLegacy && appearance.gradientFrom && appearance.localMode) {
    return raw;
  }
  return {
    ...raw,
    appearance: {
      ...appearance,
      mode: mode as Settings["appearance"]["mode"],
      gradientFrom: appearance.gradientFrom ?? (appearance as unknown as Record<string, string>).gradientTop ?? "#ffffff",
      gradientTo: appearance.gradientTo ?? (appearance as unknown as Record<string, string>).gradientBottom ?? raw.accent ?? "#f5c845",
      localMode: appearance.localMode ?? (appearance as unknown as Record<string, string>).localPlay ?? "single",
      localIntervalSec: appearance.localIntervalSec ?? (appearance as unknown as Record<string, number>).localInterval ?? 30,
      apiUrl: appearance.apiUrl ?? (appearance as unknown as Record<string, string>).url ?? "",
      apiMode: appearance.apiMode ?? (appearance as unknown as Record<string, string>).urlPlay ?? "single",
      apiIntervalSec: appearance.apiIntervalSec ?? (appearance as unknown as Record<string, number>).urlInterval ?? 60,
      wallpaperDir: appearance.wallpaperDir ?? (appearance as unknown as Record<string, string>).steamDir ?? "",
      wallpaperFile: appearance.wallpaperFile ?? (appearance as unknown as Record<string, string>).steamFile ?? "",
      wallpaperMode: appearance.wallpaperMode ?? "single",
      wallpaperIntervalSec: appearance.wallpaperIntervalSec ?? 60,
    },
  };
}

function readMockSettings(): Settings {
  try {
    const raw = window.localStorage.getItem(MOCK_SETTINGS_KEY);
    if (raw) return { ...MOCK_SETTINGS, ...(JSON.parse(raw) as Partial<Settings>) };
  } catch {
    /* 忽略损坏的 mock 设置 */
  }
  return { ...MOCK_SETTINGS };
}

// ------------------------------------------- mock 层：下载任务 / 事件 / 实例设置

const secondsAgo = (secs: number): string => new Date(Date.now() - secs * 1000).toISOString();

/** 浏览器预览用的下载任务：一进入就能同时看到「进行中」和「已完成」两种形态。 */
let mockTasks: DownloadTask[] = [
  {
    id: "mock-task-1",
    title: "1.21.4 资源补全",
    stage: "正在下载 libraries/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar",
    phase: "downloading",
    progress: 0.42,
    bytesPerSec: 3_355_443,
    receivedBytes: 41_943_040,
    totalBytes: 99_864_381,
    targetDir: mockRoot + "\\versions\\1.21.4",
    createdAt: secondsAgo(92),
    finishedAt: null,
    error: null,
  },
  {
    id: "mock-task-2",
    title: "Fabric 加载器 0.26.1",
    stage: "",
    phase: "finished",
    progress: 1,
    bytesPerSec: 0,
    receivedBytes: 8_388_608,
    totalBytes: 8_388_608,
    targetDir: mockRoot + "\\versions\\fabric-loader-0.26.1-1.21.4",
    createdAt: secondsAgo(3600),
    finishedAt: secondsAgo(3520),
    error: null,
  },
  {
    id: "mock-task-3",
    title: "1.20.1 客户端",
    stage: "",
    phase: "finished",
    progress: 1,
    bytesPerSec: 0,
    receivedBytes: 251_658_240,
    totalBytes: 251_658_240,
    targetDir: mockRoot + "\\versions\\1.20.1",
    createdAt: secondsAgo(86_400),
    finishedAt: secondsAgo(86_300),
    error: null,
  },
];

const MOCK_STAGES = [
  "正在下载 assets/objects/1a/1a2b3c4d5e",
  "正在下载 libraries/com/mojang/brigadier",
  "正在下载 客户端 jar 分片",
  "正在校验 sha1",
];

const mockHandlers = new Set<(event: LauncherEvent) => void>();
let mockTicker: number | null = null;

function emitMock(event: LauncherEvent): void {
  mockHandlers.forEach((handler) => {
    try {
      handler(event);
    } catch {
      /* mock 里的订阅者异常不该影响其它订阅者 */
    }
  });
}

function emitMockProgress(task: DownloadTask): void {
  const payload: InstallProgress = {
    taskId: task.id,
    title: task.title,
    phase: task.phase,
    stage: task.stage,
    progress: task.progress,
    bytesPerSec: task.bytesPerSec,
    receivedBytes: task.receivedBytes,
    totalBytes: task.totalBytes,
  };
  emitMock({ kind: "install-progress", payload });
}

/** 浏览器预览里让「进行中」的任务真的会走：每 900ms 推一次 install://progress。 */
function ensureMockTicker(): void {
  if (mockTicker !== null || typeof window === "undefined") return;
  mockTicker = window.setInterval(() => {
    let touched = false;
    for (const task of mockTasks) {
      if (task.phase !== "downloading" && task.phase !== "queued" && task.phase !== "verifying") continue;
      touched = true;
      const step = 0.006 + Math.random() * 0.008;
      task.progress = Math.min(1, task.progress + step);
      task.stage = MOCK_STAGES[Math.floor(task.progress * MOCK_STAGES.length) % MOCK_STAGES.length];
      task.bytesPerSec = 2_200_000 + Math.floor(Math.random() * 2_400_000);
      if (task.totalBytes !== null) task.receivedBytes = Math.floor(task.totalBytes * task.progress);
      if (task.progress >= 1) {
        task.phase = "finished";
        task.stage = "";
        task.bytesPerSec = 0;
        task.finishedAt = new Date().toISOString();
        task.receivedBytes = task.totalBytes;
      }
      emitMockProgress(task);
    }
    if (!touched && mockHandlers.size === 0 && mockTicker !== null) {
      window.clearInterval(mockTicker);
      mockTicker = null;
    }
  }, 900);
}

/** 新建实例后由 instance_create + version_install 拉起的假安装任务（导出实例包 / 内容安装也走这条）。 */
function startMockInstall(title: string, targetDir: string, kind: DownloadTask["kind"] = "install"): void {
  const task: DownloadTask = {
    id: "mock-task-" + Date.now().toString(36),
    kind,
    title,
    stage: "正在获取版本清单",
    phase: "downloading",
    progress: 0.02,
    bytesPerSec: 2_400_000,
    receivedBytes: 1_048_576,
    totalBytes: 209_715_200,
    targetDir,
    createdAt: new Date().toISOString(),
    finishedAt: null,
    error: null,
  };
  mockTasks = [task, ...mockTasks];
  ensureMockTicker();
  emitMockProgress(task);
}

const MOCK_INSTANCE_SETTINGS = new Map<number, InstanceSettings>();

function mockInstanceSettings(id: number): InstanceSettings {
  const cached = MOCK_INSTANCE_SETTINGS.get(id);
  if (cached) return cached;
  const instance = mockInstances.find((item) => item.id === id);
  const fresh: InstanceSettings = {
    name: instance?.name ?? "实例",
    note: "",
    memoryMb: 2048,
    windowWidth: 1920,
    windowHeight: 1080,
    javaPath: "",
    iconPath: "",
  };
  MOCK_INSTANCE_SETTINGS.set(id, fresh);
  return fresh;
}

const MOCK_JAVA: JavaRuntime[] = [
  {
    path: "C:\\Program Files\\BellSoft\\LibericaJDK-8",
    javaExe: "C:\\Program Files\\BellSoft\\LibericaJDK-8\\bin\\javaw.exe",
    version: "1.8.0_482",
    major: 8,
    arch: "x64",
    vendor: "BellSoft Liberica",
    isDefault: false,
  },
  {
    path: "C:\\Program Files\\BellSoft\\LibericaJDK-17",
    javaExe: "C:\\Program Files\\BellSoft\\LibericaJDK-17\\bin\\javaw.exe",
    version: "17.0.18",
    major: 17,
    arch: "x64",
    vendor: "BellSoft Liberica",
    isDefault: true,
  },
  {
    path: "C:\\Program Files\\Eclipse Adoptium\\jdk-21.0.10",
    javaExe: "C:\\Program Files\\Eclipse Adoptium\\jdk-21.0.10\\bin\\javaw.exe",
    version: "21.0.10",
    major: 21,
    arch: "x64",
    vendor: "Adoptium",
    isDefault: false,
  },
  {
    path: "C:\\Program Files\\Zulu\\zulu-25",
    javaExe: "C:\\Program Files\\Zulu\\zulu-25\\bin\\javaw.exe",
    version: "25.0.2",
    major: 25,
    arch: "x64",
    vendor: "Zulu",
    isDefault: false,
  },
  {
    path: mockRoot + "\\runtimes\\jre-legacy",
    javaExe: mockRoot + "\\runtimes\\jre-legacy\\bin\\javaw.exe",
    version: "1.8.0_202",
    major: 8,
    arch: "x64",
    vendor: "Bundled",
    isDefault: false,
  },
];

const MOCK_JAVA_SOURCES: JavaSource[] = [
  { id: "auto", name: "全部（合并全部源）", description: "并发查所有源，同版本保留延迟最低的那条" },
  { id: "bellsoft", name: "BellSoft Liberica", description: "覆盖面广，含 JRE 与全量 JDK" },
  { id: "adoptium", name: "Adoptium (Temurin)", description: "社区维护的 OpenJDK 构建" },
  { id: "tuna", name: "清华 TUNA 镜像", description: "国内镜像，下载稳定" },
  { id: "zulu", name: "Azul Zulu", description: "Azul 提供的 OpenJDK 构建" },
  { id: "oracle", name: "Oracle JDK", description: "官网下载，需要接受 Oracle 许可", requiresLicense: true },
];

/** 各源的实测延迟（和 java_probe_sources 一致）：合并列表按它挑同版本里最快的那个。 */
const MOCK_JAVA_LATENCY: Record<string, number> = { oracle: 72, bellsoft: 171, tuna: 177, zulu: 300, adoptium: 355 };

const MOCK_JAVA_SOURCE_LABEL: Record<string, string> = {
  bellsoft: "BellSoft Liberica",
  adoptium: "Adoptium · Temurin",
  tuna: "清华 TUNA 镜像 · Temurin",
  zulu: "Azul Zulu",
  oracle: "Oracle JDK",
};

/** 每个源 × 5 个 major（25/21/17/11/8）；tuna 是 Adoptium 镜像，版本号与 adoptium 相同，用来验证去重。 */
const MOCK_JAVA_MATRIX: Array<{ source: string; versions: Array<[string, number]> }> = [
  {
    source: "adoptium",
    versions: [["25.0.2+10", 25], ["21.0.10+7", 21], ["17.0.18+8", 17], ["11.0.29+8", 11], ["8.0.482+8", 8]],
  },
  {
    source: "tuna",
    versions: [["25.0.2+10", 25], ["21.0.10+7", 21], ["17.0.18+8", 17], ["11.0.29+8", 11], ["8.0.482+8", 8]],
  },
  {
    source: "bellsoft",
    versions: [["25.0.2", 25], ["21.0.10", 21], ["17.0.18", 17], ["11.0.29", 11], ["8.0.482", 8]],
  },
  {
    source: "zulu",
    versions: [["25.0.2", 25], ["21.0.10", 21], ["17.0.18", 17], ["11.0.29", 11], ["8.0.482", 8]],
  },
  {
    source: "oracle",
    versions: [["25.0.2", 25], ["17.0.18", 17], ["8.0.482", 8]],
  },
];

const MOCK_JAVA_URL: Record<string, string> = {
  bellsoft: "https://bell-sw.com/pages/downloads/",
  adoptium: "https://adoptium.net/",
  tuna: "https://mirrors.tuna.tsinghua.edu.cn/Adoptium/",
  zulu: "https://www.azul.com/downloads/",
  oracle: "https://www.oracle.com/java/technologies/downloads/",
};

const MOCK_JAVA_VERSIONS: JavaVersion[] = MOCK_JAVA_MATRIX.flatMap((row) =>
  row.versions.map(([version, major], index) => ({
    source: row.source,
    version,
    major,
    lts: major !== 26,
    sizeBytes: 90_000_000 + major * 4_317_000 + index * 1_900_000,
    url: MOCK_JAVA_URL[row.source],
    sourceLabel: MOCK_JAVA_SOURCE_LABEL[row.source],
  })),
);

const MOCK_DATA_DIR = { path: mockRoot, sizeBytes: 12_884_901_888, fileCount: 48_213 };

/** 存储分项的 mock 体积（storage_clean 会就地减少）。 */
const mockStorageBytes: Record<StorageKey, number> = {
  instances: 2_147_483_648,
  versions: 6_442_450_944,
  libraries: 2_288_000_000,
  assets: 1_073_741_824,
  cache: 536_870_912,
  logs: 12_582_912,
  runtimes: 402_653_184,
};

/** 已安装的 java 版本（java_install 之后加进来）。 */
const mockInstalledJava: JavaVersion[] = [];

/** Forge / NeoForge 只在这些 MC 版本上有构建，其余版本后端会回「没有构建」。 */
const MOCK_FORGE_SUPPORT = ["1.21.4", "1.20.1", "1.19.2", "1.18.2", "1.16.5", "1.12.2"];
const MOCK_NEOFORGE_SUPPORT = ["1.21.4", "1.21.1", "1.20.4", "1.20.2"];

/** loader_versions 的假实现：不支持的组合直接抛后端原文，界面把原因显示出来。 */
function mockLoaderVersions(loader: string, mcVersion: string): LoaderVersion[] {
  const mc = mcVersion.trim();
  if (!mc) throw new Error("先选一个 Minecraft 版本");
  if (loader === "forge") {
    if (!MOCK_FORGE_SUPPORT.includes(mc)) throw new Error("该版本没有 Forge 构建（" + mc + "）");
    const builds: Record<string, string> = {
      "1.21.4": "52.0.4",
      "1.20.1": "47.3.0",
      "1.19.2": "43.4.0",
      "1.18.2": "40.2.10",
      "1.16.5": "36.2.42",
      "1.12.2": "14.23.5.2860",
    };
    return [
      { id: "forge-" + mc + "-" + builds[mc], version: builds[mc], recommended: true },
      { id: "forge-" + mc + "-legacy", version: builds[mc] + "-beta" },
    ];
  }
  if (loader === "neoforge") {
    if (!MOCK_NEOFORGE_SUPPORT.includes(mc)) throw new Error("NeoForge 只支持 1.20.2 及以后（当前 " + mc + "）");
    const builds: Record<string, string> = { "1.21.4": "21.4.56", "1.21.1": "21.1.80", "1.20.4": "20.4.237", "1.20.2": "20.2.88" };
    return [
      { id: "neoforge-" + mc + "-" + builds[mc], version: builds[mc], recommended: true },
      { id: "neoforge-" + mc + "-old", version: builds[mc].split(".")[0] + ".0.1" },
    ];
  }
  // Fabric：正式版都有，快照/preview 没有
  if (/-(pre|rc)\d*/i.test(mc)) throw new Error("Fabric 还没有适配 " + mc + " 的构建");
  return [
    { id: "fabric-" + mc + "-0.16.9", version: "0.16.9", recommended: true },
    { id: "fabric-" + mc + "-0.16.5", version: "0.16.5" },
    { id: "fabric-" + mc + "-0.15.11", version: "0.15.11" },
  ];
}

const MOCK_LOG_FILES: LogFile[] = [
  { name: "latest.log", path: mockRoot + "\\logs\\latest.log", size: 48_213, modifiedAt: secondsAgo(600) },
  { name: "2025-02-01-1.log", path: mockRoot + "\\logs\\2025-02-01-1.log", size: 210_554, modifiedAt: secondsAgo(86_400) },
  { name: "debug.log", path: mockRoot + "\\logs\\debug.log", size: 9_120, modifiedAt: secondsAgo(7200) },
];

const MOCK_LOG_BODY = [
  "[10:02:31] [main/INFO]: Loading Minecraft 1.21.4 with Fabric Loader 0.26.1",
  "[10:02:31] [main/INFO]: Loading 56 mods:",
  "\t- fabric-api 0.115.0+1.21.4",
  "\t- fabric-loader 0.26.1",
  "\t- fabric-lifecycle-events-v1 2.5.4",
  "\t- fabric-resource-loader-v0 1.10.5",
  "[10:02:32] [main/WARN]: Configuration conflict for 'default' at 'config/x.json'",
  "[10:02:33] [main/INFO]: Environment: Environment[sessionHost=https://sessionserver.mojang.com, servicesHost=https://api.minecraftservices.com, name=PROD]",
  "[10:02:34] [main/INFO]: Found new data pack file/fabric-api, loading it automatically",
  "[10:02:35] [Render thread/INFO]: Backend library: LWJGL version 3.3.3+5",
  "[10:02:36] [Render thread/INFO]: OpenGL debug message: id=1282, source=API, type=ERROR, severity=HIGH",
  "[10:02:37] [Render thread/INFO]: Narrator library for x64 successfully loaded",
  "[10:02:38] [Render thread/WARN]: Failed to load pack 'vanilla-extras.zip' - not found",
  "[10:02:39] [Render thread/INFO]: Loaded 1284 recipes, 92 advancements",
  "[10:02:41] [Render thread/INFO]: Sound engine started",
  "[10:02:42] [Render thread/INFO]: Created: 1024x512x0 minecraft:textures/atlas/blocks.png-atlas",
  "[10:02:43] [Render thread/INFO]: OpenAL initialized on device Speakers",
  "[10:03:02] [Server thread/INFO]: Saving chunks for level 'ServerLevel[新的世界]'/minecraft:overworld",
  "[10:03:05] [Render thread/INFO]: Stopping worker threads",
  "[10:03:06] [main/INFO]: Game closed with exit code 0",
];

function mockLogContent(name: string): string {
  return MOCK_LOG_BODY.map((line, index) => (index === 0 ? "===== " + name + " =====" : line)).join("\n");
}

// ------------------------------------- mock 层：M3-2 皮肤 / 披风 / 账号 / 内容

/** 画一张 64×64 的假皮肤：模型能渲染出人形就够，不追求像真的。 */
function mockSkinDataUrl(variant: number): string {
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 64;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  const hair = ["#6b4a2f", "#33261a", "#c9a227", "#26418f"][variant % 4];
  const shirt = ["#3f6fd1", "#2f9e6f", "#9c4dcc", "#c1554d"][variant % 4];
  ctx.clearRect(0, 0, 64, 64);
  // 先把所有面铺成皮肤色，再把「外层」清掉，免得外层把本体糊住
  ctx.fillStyle = "#e0ac69";
  ctx.fillRect(0, 0, 64, 64);
  ctx.clearRect(32, 0, 32, 16);
  ctx.clearRect(0, 32, 56, 16);
  ctx.clearRect(48, 48, 16, 16);
  // 头发（头顶 + 后脑）
  ctx.fillStyle = hair;
  ctx.fillRect(8, 0, 8, 8);
  ctx.fillRect(24, 8, 8, 8);
  // 脸 + 眼睛 + 嘴
  ctx.fillStyle = "#e0ac69";
  ctx.fillRect(8, 8, 8, 8);
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(9, 11, 2, 1);
  ctx.fillRect(13, 11, 2, 1);
  ctx.fillStyle = "#3b2c1f";
  ctx.fillRect(9, 11, 1, 1);
  ctx.fillRect(14, 11, 1, 1);
  ctx.fillRect(10, 14, 4, 1);
  // 上衣 + 袖子
  ctx.fillStyle = shirt;
  ctx.fillRect(20, 20, 8, 12);
  ctx.fillRect(44, 20, 4, 6);
  ctx.fillRect(36, 52, 4, 6);
  // 裤子 + 鞋
  ctx.fillStyle = "#35426b";
  ctx.fillRect(4, 20, 4, 12);
  ctx.fillRect(20, 52, 4, 8);
  ctx.fillStyle = "#2b2b2b";
  ctx.fillRect(4, 29, 4, 3);
  ctx.fillRect(20, 60, 4, 4);
  return canvas.toDataURL("image/png");
}

/** 画一张 64×32 的假披风（正面在 1,1 起 10×16，和真实贴图布局一致）。 */
function mockCapeDataUrl(variant: number): string {
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 32;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  ctx.fillStyle = ["#c0392b", "#1f6f54", "#3d4d8f", "#7a4bb5"][variant % 4];
  ctx.fillRect(1, 1, 10, 16);
  ctx.fillStyle = "#f5c845";
  ctx.fillRect(3, 4, 6, 3);
  ctx.fillRect(5, 9, 2, 6);
  return canvas.toDataURL("image/png");
}

/** 后端会额外给一张正面裁切图（previewDataUrl），mock 这里模拟一张 40×64 的。 */
function mockCapePreviewDataUrl(variant: number): string {
  const canvas = document.createElement("canvas");
  canvas.width = 40;
  canvas.height = 64;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  ctx.fillStyle = ["#c0392b", "#1f6f54", "#3d4d8f", "#7a4bb5"][variant % 4];
  ctx.fillRect(0, 0, 40, 64);
  ctx.fillStyle = "#f5c845";
  ctx.fillRect(8, 14, 24, 12);
  ctx.fillRect(16, 34, 8, 24);
  return canvas.toDataURL("image/png");
}

/** 内容卡片的图标占位：色块 + 首字母，避免 mock 依赖外网图片。 */
const mockIconCache = new Map<string, string>();
function mockIconDataUrl(label: string, color: string): string {
  const key = label + color;
  const cached = mockIconCache.get(key);
  if (cached) return cached;
  const canvas = document.createElement("canvas");
  canvas.width = 96;
  canvas.height = 96;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  ctx.fillStyle = color;
  ctx.fillRect(0, 0, 96, 96);
  ctx.fillStyle = "rgba(255,255,255,0.92)";
  ctx.font = "bold 46px 'Microsoft YaHei', system-ui, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(label.slice(0, 1).toUpperCase() || "?", 48, 52);
  const url = canvas.toDataURL("image/png");
  mockIconCache.set(key, url);
  return url;
}

// ---- 皮肤 / 披风 ----

let mockSkins: SkinItem[] = [
  { name: "zi_min", path: mockRoot + "\\skins\\zi_min.png", size: 3072, dataUrl: mockSkinDataUrl(0), source: "local" },
  { name: "steve_blue", path: mockRoot + "\\skins\\steve_blue.png", size: 2904, dataUrl: mockSkinDataUrl(3), source: "local" },
];

/** 内置 Steve / Alex：不落盘，直接引用打包进来的 PNG。 */
function mockBuiltinSkin(model: "default" | "slim"): SkinItem {
  const slim = model === "slim";
  return {
    name: slim ? "Alex" : "Steve",
    path: "builtin://" + (slim ? "alex" : "steve"),
    size: slim ? 2173 : 1893,
    dataUrl: slim ? builtinSkins.alex : builtinSkins.steve,
    builtin: true,
    source: "builtin",
  };
}

/** skin_fetch_mojang 的假实现：空名 / 无此玩家 / 无公开皮肤 / 服务不可达 各一条分支。 */
function mockFetchMojang(username: string): SkinItem {
  const name = username.trim();
  if (!name) throw new Error("玩家名不能为空");
  if (/^nouser$|^missing/i.test(name)) throw new Error("没有这个正版玩家：" + name);
  if (/^noskin/i.test(name)) throw new Error("该玩家没有公开皮肤");
  if (/^down|^timeout/i.test(name)) throw new Error("无法连接皮肤服务");
  const variant = name.length % 4;
  const item: SkinItem = {
    name: name,
    path: mockRoot + "\\skins\\" + name + ".png",
    size: 3072,
    dataUrl: mockSkinDataUrl(variant),
    source: "mojang",
    uuid: mockOfflineUuid(name),
  };
  mockSkins = [item, ...mockSkins.filter((skin) => skin.name !== name)];
  return { ...item };
}

interface MockCape {
  id: string;
  name: string;
  alias: string;
  url: string;
  /** 只有第一张带后端预生成的正面裁切图，其余走前端裁切分支 */
  previewDataUrl?: string;
}

const MOCK_CAPES: MockCape[] = [
  { id: "migrator", name: "Migrator", alias: "迁移者", url: mockCapeDataUrl(0), previewDataUrl: mockCapePreviewDataUrl(0) },
  { id: "mojang", name: "Mojang", alias: "官方员工", url: mockCapeDataUrl(2) },
  { id: "founders", name: "Founder's Cape", alias: "创始人", url: mockCapeDataUrl(1) },
  { id: "translator", name: "Translator Cape", alias: "译者", url: mockCapeDataUrl(3) },
];

/** 浏览器预览里用 localStorage 的 gugu.mock.cape 模拟四种账号形态。 */
const MOCK_CAPE_MODE_KEY = "gugu.mock.cape";

/**
 * 皮肤站（Yggdrasil）查皮肤的假实现：
 * 地址不合法 / 玩家不存在 / 没设皮肤 / 站点无响应 各走一条错误分支，便于自检。
 */
function mockSkinLookup(server: string, name: string): SkinLookupResult {
  const base = server.trim();
  if (!/^https?:\/\/[^\s]+$/i.test(base)) {
    throw new Error("皮肤站地址不对，应为 https://…/api/yggdrasil");
  }
  if (!/\/api\/yggdrasil\/?$/i.test(base)) {
    throw new Error("地址要填到 /api/yggdrasil 这一级");
  }
  const player = name.trim();
  if (!player) throw new Error("请填写玩家名或 UUID");
  if (/nouser|missing/i.test(player)) throw new Error("该玩家不存在");
  if (/noskin/i.test(player)) throw new Error("该玩家还没有设置皮肤");
  if (/down|timeout/i.test(base)) throw new Error("皮肤站无响应（超时）");
  const variant = player.length % 4;
  const hasCape = /cape|\d$/.test(player);
  return {
    uuid: mockOfflineUuid(player),
    name: player,
    skinUrl: mockSkinDataUrl(variant),
    capeUrl: hasCape ? mockCapeDataUrl(variant) : null,
    model: "default",
  };
}

/** 当前应用的皮肤 / 披风（mock 只存在内存里，刷新页面回到默认）。 */
let mockCurrentSkin = mockSkins[0].path;
let mockActiveCapeId: string | null = "migrator";

/**
 * capes_list 的假实现：披风只属于「微软账号」。
 * 离线账号 / 未登录 / 皮肤站账号都返回 supported=false，reason 直接给界面显示。
 */
function mockCapesList(): CapesListResult {
  const forced = window.localStorage.getItem(MOCK_CAPE_MODE_KEY);
  if (forced === "thirdparty") {
    return { supported: false, reason: "第三方（皮肤站）账号不支持披风", active: null, items: [] };
  }
  if (forced === "expired") {
    return { supported: false, reason: "登录已过期，请重新登录", active: null, items: [] };
  }
  const current = mockAccounts.find((account) => account.current) ?? null;
  if (forced === "offline" || !current || current.type !== "microsoft") {
    return {
      supported: false,
      reason: current ? "离线账号不支持披风，登录微软账号后可用" : "请先登录微软账号",
      active: null,
      items: [],
    };
  }
  return {
    supported: true,
    active: mockActiveCapeId,
    items: MOCK_CAPES.map((cape) => ({
      id: cape.id,
      name: cape.name,
      alias: cape.alias,
      url: cape.url,
      state: cape.id === mockActiveCapeId ? ("ACTIVE" as const) : ("INACTIVE" as const),
      previewDataUrl: cape.previewDataUrl ?? null,
    })),
  };
}

// ---- 账号 ----

let mockAccounts: Account[] = [
  {
    id: "acc-ms-1",
    name: "Jiji_GuoWang",
    type: "microsoft",
    uuid: "7f2c5d1e8a4b4c6f9d0a1b2c3d4e5f60",
    skinDataUrl: mockSkinDataUrl(0),
    current: true,
  },
  {
    id: "acc-tp-1",
    name: "littleskin_user",
    type: "thirdparty",
    uuid: "a1b2c3d4e5f60718293a4b5c6d7e8f90",
    server: "https://littleskin.cn/api/yggdrasil",
    skinDataUrl: mockSkinDataUrl(2),
    current: false,
  },
  {
    id: "acc-off-1",
    name: "asdas2w4r1q",
    type: "offline",
    uuid: "3a6f1f2c9b8e3f4a5c7d9e0b1a2c3d4e",
    // 离线账号没设皮肤 → 界面回落内置 Steve
    skinDataUrl: null,
    current: false,
  },
  {
    id: "acc-off-2",
    name: "dfsdfA4363446",
    type: "offline",
    uuid: "9c8b7a6d5e4f3a2b1c0d9e8f7a6b5c4d",
    skinDataUrl: null,
    current: false,
  },
];

/** 离线 UUID：和启动器后端同算法的假值，够 mock 用。 */
function mockOfflineUuid(name: string): string {
  let h1 = 0x811c9dc5;
  let h2 = 0x01000193;
  for (let i = 0; i < name.length; i += 1) {
    h1 = ((h1 ^ name.charCodeAt(i)) * 16777619) >>> 0;
    h2 = (h2 + name.charCodeAt(i) * (i + 7)) >>> 0;
  }
  const part = (n: number) => (n >>> 0).toString(16).padStart(8, "0").slice(0, 8);
  return part(h1) + part(h2) + part(h1 * 31 + h2) + part(h1 ^ h2);
}

function mockApplyAccount(id: string): void {
  mockAccounts = mockAccounts.map((account) => ({ ...account, current: account.id === id }));
}

/** 第三方（Yggdrasil 皮肤站）登录的假实现：地址 / 密码 / 角色 / 可达性各走一条错误分支。 */
const MOCK_AUTHLIB_KEY = "gugu.mock.authlib";

function mockThirdpartyLogin(request: { server: string; username: string; password: string }): Account {
  const server = request.server.trim();
  if (!server) throw new Error("请先填写皮肤站服务器地址");
  if (!/^https?:\/\/[^\s]+$/i.test(server)) throw new Error("服务器地址格式不对，应为 https://…/api/yggdrasil");
  if (!/\/api\/yggdrasil\/?$/i.test(server)) throw new Error("地址要填到 /api/yggdrasil 这一级");
  const username = request.username.trim();
  if (!username) throw new Error("请填写用户名或邮箱");
  if (!request.password) throw new Error("请填写密码");
  if (/down|timeout|unreachable/i.test(server)) throw new Error("服务器不可达（连接超时 8000ms）");
  if (/norole/i.test(username)) throw new Error("这个账号没有可用的角色，请先在皮肤站创建角色");
  if (/wrong|bad|401/i.test(request.password)) throw new Error("用户名或密码错误（HTTP 401）");
  if (mockAccounts.some((account) => account.name.toLowerCase() === username.toLowerCase())) {
    throw new Error("已经有同名账号了");
  }
  const created: Account = {
    id: "acc-tp-" + Date.now().toString(36),
    name: username,
    type: "thirdparty",
    server: server,
    uuid: mockOfflineUuid(server + username),
    skinDataUrl: mockSkinDataUrl(2),
    current: false,
  };
  mockAccounts = [...mockAccounts, created];
  return { ...created };
}

function mockAuthlibInstalled(): boolean {
  return window.localStorage.getItem(MOCK_AUTHLIB_KEY) === "installed";
}

function mockAuthlibStatus(): AuthlibStatus {
  if (!mockAuthlibInstalled()) return { installed: false, path: "", version: null };
  return {
    installed: true,
    path: mockRoot + "\\authlib-injector\\authlib-injector-1.2.7.jar",
    version: "1.2.7",
  };
}

/** 后端可能用 kind（"ms" / "microsoft" / "offline"）描述账号类型，前端统一折成 type。 */
function normalizeAccount(raw: Account): Account {
  const kind = String(raw.type ?? raw.kind ?? "").toLowerCase();
  const type: Account["type"] =
    kind === "microsoft" || kind === "ms" ? "microsoft" : kind === "thirdparty" || kind === "tp" ? "thirdparty" : "offline";
  return { ...raw, type };
}

// ---- 微软登录（设备码流程的假状态）----

const MOCK_MS_STATE = { deviceCode: "", userCode: "", polls: 0 };

/** Client ID 只接受 8-4-4-4-12 的 UUID（大小写都行）。 */
const MS_CLIENT_ID_PATTERN = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

/**
 * ms_profile 的假实现：账号当前皮肤 = 本地正在应用的那张（上传/取回后后端会把结果缓存回来）。
 * force 太频繁时返回 throttled，和真后端的节流语义一致。
 */
let mockMsProfileCache: {
  skinUrl: string;
  previewDataUrl: string;
  variant: "classic" | "slim";
  cachedAt: string;
  /** 缓存时本地正在用的那张皮肤：本地换了就不再复用缓存 */
  path: string;
} | null = null;
let mockMsProfileForcedAt = 0;
const MOCK_MS_PROFILE_FORCE_MS = 60_000;

function mockMsProfile(request: { force?: boolean }): MsProfile {
  // 自检用：记录 force 次数与最近一次返回的皮肤图（window.__guguMsSkin / window.__guguMsProfileForce）
  if (typeof window !== "undefined" && request.force) {
    const host = window as unknown as Record<string, unknown>;
    host.__guguMsProfileForce = (typeof host.__guguMsProfileForce === "number" ? host.__guguMsProfileForce : 0) + 1;
  }
  const profile = mockMsProfileInner(request);
  if (typeof window !== "undefined") {
    (window as unknown as Record<string, unknown>).__guguMsSkin = {
      url: profile.previewDataUrl ?? profile.skinUrl ?? "",
      fromCache: profile.fromCache === true,
      throttled: profile.throttled === true,
      force: request.force === true,
    };
  }
  return profile;
}

function mockMsProfileInner(request: { force?: boolean }): MsProfile {
  const account = mockAccounts.find((item) => item.current);
  if (!account || account.type !== "microsoft") throw new Error("请先登录微软账号");
  if (window.localStorage.getItem("gugu.mock.msToken") === "expired") throw new Error("登录已过期，请重新登录");
  const base = { name: account.name, uuid: account.uuid, capeUrl: null };
  if (window.localStorage.getItem("gugu.mock.msProfile") === "nocache") {
    throw new Error("Microsoft 接口返回 429，且本地没有缓存");
  }
  const useDefault = window.localStorage.getItem("gugu.mock.msSkin") === "default";
  if (request.force) {
    if (Date.now() - mockMsProfileForcedAt < MOCK_MS_PROFILE_FORCE_MS) {
      return { ...base, variant: "classic", skinUrl: null, throttled: true, fromCache: true, cachedAt: mockMsProfileCache?.cachedAt ?? null };
    }
    mockMsProfileForcedAt = Date.now();
  } else if (mockMsProfileCache && !useDefault && mockMsProfileCache.path === mockCurrentSkin) {
    return { ...base, ...mockMsProfileCache, fromCache: true };
  }
  const current = useDefault ? null : (mockSkins.find((skin) => skin.path === mockCurrentSkin) ?? null);
  const url = current?.dataUrl ?? "";
  mockMsProfileCache = {
    skinUrl: url,
    previewDataUrl: url,
    variant: "classic",
    cachedAt: new Date().toISOString(),
    path: mockCurrentSkin,
  };
  return { ...base, ...mockMsProfileCache, fromCache: false };
}

function mockMsAuthStatus(): MsAuthStatus {
  const value = readMockSettings().msClientId.trim();
  if (!value) {
    return { configured: false, hint: "还没有填写 Client ID", invalid: false };
  }
  if (!MS_CLIENT_ID_PATTERN.test(value)) {
    return { configured: false, hint: "已保存的 Client ID 格式无效，请重新填写", invalid: true };
  }
  return { configured: true, hint: "", invalid: false };
}

function mockMsStart(): MsLoginStart {
  const status = mockMsAuthStatus();
  if (!status.configured) {
    return { configured: false, userCode: null, verifyUrl: null, deviceCode: null, expiresIn: null };
  }
  MOCK_MS_STATE.deviceCode = "mock-device-" + Date.now().toString(36);
  MOCK_MS_STATE.userCode = "K7QM-4XZP";
  MOCK_MS_STATE.polls = 0;
  return {
    configured: true,
    userCode: MOCK_MS_STATE.userCode,
    verifyUrl: "https://www.microsoft.com/link",
    verificationUri: "https://www.microsoft.com/link",
    deviceCode: MOCK_MS_STATE.deviceCode,
    expiresIn: 900,
  };
}

function mockMsPoll(deviceCode: string): MsLoginPoll {
  if (!mockMsAuthStatus().configured) return { state: "error", account: null, message: "还没有填写 Client ID" };
  if (deviceCode !== MOCK_MS_STATE.deviceCode) {
    return { state: "error", account: null, message: "登录会话已失效，请重新发起" };
  }
  MOCK_MS_STATE.polls += 1;
  if (MOCK_MS_STATE.polls < 2) return { state: "pending", account: null, message: null };
  const existed = mockAccounts.find((account) => account.type === "microsoft");
  const account: Account = existed ?? {
    id: "acc-ms-" + Date.now().toString(36),
    name: "Jiji_GuoWang",
    type: "microsoft",
    uuid: "7f2c5d1e8a4b4c6f9d0a1b2c3d4e5f60",
    skinDataUrl: mockSkinDataUrl(0),
    current: false,
  };
  if (!existed) mockAccounts = [...mockAccounts, account];
  mockApplyAccount(account.id);
  return { state: "done", account: { ...account, current: true }, message: null };
}

// ---- 实例内的模组 / 资源包 / 光影 / 存档 ----

const MOCK_INSTANCE_FILES = new Map<string, ModFileItem[]>();

function mockFileItem(
  file: string,
  size: number,
  version: string | null,
  loader: string | null,
  source: string | null,
): ModFileItem {
  const enabled = !file.endsWith(".disabled");
  const base = enabled ? file : file.slice(0, -".disabled".length);
  return { file, name: base.replace(/\.(jar|zip)$/i, ""), size, enabled, version, loader, source };
}

const MOCK_INSTANCE_FILES_SEED: Record<ContentFileKind, ModFileItem[]> = {
  mod: [
    mockFileItem("fabric-api-0.115.0+1.21.4.jar", 2_097_152, "0.115.0+1.21.4", "Fabric", "Modrinth"),
    mockFileItem("sodium-fabric-0.6.5+mc1.21.4.jar", 1_048_576, "0.6.5", "Fabric", "Modrinth"),
    mockFileItem("lithium-fabric-0.14.3+mc1.21.4.jar", 786_432, "0.14.3", "Fabric", "Modrinth"),
    mockFileItem("iris-fabric-1.8.1+mc1.21.4.jar.disabled", 3_355_443, "1.8.1", "Fabric", "Modrinth"),
    mockFileItem("jei-1.21.4-fabric-19.21.0.247.jar", 1_572_864, "19.21.0.247", "Fabric", "CurseForge"),
    mockFileItem("handcrafted-1.21.4.jar.disabled", 524_288, null, null, null),
  ],
  resourcepack: [
    mockFileItem("vanilla-tweaks-1.21.4.zip", 262_144, "1.21.4", null, null),
    mockFileItem("stay-true-1.21.4.zip.disabled", 524_288, "1.21.4", null, null),
  ],
  shader: [
    mockFileItem("complementary-unbound-r5.3.zip", 1_048_576, "r5.3", "Iris", "Modrinth"),
    mockFileItem("bsl-shaders-8.4.zip.disabled", 786_432, "8.4", "Iris", "Modrinth"),
  ],
};

function mockInstanceFiles(instanceId: number, kind: ContentFileKind): ModFileItem[] {
  const key = instanceId + ":" + kind;
  const cached = MOCK_INSTANCE_FILES.get(key);
  if (cached) return cached;
  const seed = MOCK_INSTANCE_FILES_SEED[kind].map((item) => ({ ...item }));
  MOCK_INSTANCE_FILES.set(key, seed);
  return seed;
}

function mockStoreFiles(instanceId: number, kind: ContentFileKind, list: ModFileItem[]): ModFileItem[] {
  MOCK_INSTANCE_FILES.set(instanceId + ":" + kind, list);
  return list;
}

const MOCK_SAVES = new Map<number, SaveItem[]>();

function mockSaves(instanceId: number): SaveItem[] {
  const cached = MOCK_SAVES.get(instanceId);
  if (cached) return cached;
  const seed: SaveItem[] = [
    { name: "新的世界", size: 41_943_040, modifiedAt: secondsAgo(3_600), mcVersion: "1.21.4" },
    { name: "空岛生存", size: 128_974_848, modifiedAt: secondsAgo(86_400 * 3), mcVersion: "1.21.4" },
    { name: "红石实验场", size: 8_388_608, modifiedAt: secondsAgo(86_400 * 12), mcVersion: "1.20.6" },
  ];
  MOCK_SAVES.set(instanceId, seed);
  return seed;
}

// ---- 内容服务：假项目库 ----

interface MockProject {
  slug: string;
  kind: ContentType;
  title: string;
  summary: string;
  description: string;
  color: string;
  downloads: number;
  author: string;
  gameVersions: string[];
  loaders: string[];
}

const MOCK_PROJECTS: MockProject[] = [
  {
    slug: "sodium",
    kind: "mod",
    title: "Sodium",
    summary: "A modern rendering engine for Minecraft which greatly improves performance.",
    description:
      "## Sodium\n\nSodium is a modern rendering engine for Minecraft that drastically improves performance and frame times, while staying compatible with the vanilla pipeline.\n\n- Massive FPS boost\n- Better chunk rendering\n- Fixed graphics bugs",
    color: "#2f9e6f",
    downloads: 92_400_000,
    author: "jellysquid3",
    gameVersions: ["1.21.4", "1.21.3", "1.20.6"],
    loaders: ["Fabric", "Quilt"],
  },
  {
    slug: "fabric-api",
    kind: "mod",
    title: "Fabric API",
    summary: "Lightweight and modular API providing common hooks and intercompatibility measures.",
    description:
      "## Fabric API\n\nCore API for the Fabric mod loader, required by most Fabric mods.\n\n- Lifecycle events\n- Networking helpers\n- Registry sync",
    color: "#3f6fd1",
    downloads: 64_100_000,
    author: "modmuss50",
    gameVersions: ["1.21.4", "1.21.3", "1.20.6", "1.20.1"],
    loaders: ["Fabric"],
  },
  {
    slug: "iris",
    kind: "shader",
    title: "Iris Shaders",
    summary: "A modern shaders mod compatible with most OptiFine shaderpacks.",
    description: "## Iris Shaders\n\nIris is a modern shader pack loader for Minecraft, compatible with most OptiFine shaderpacks.",
    color: "#7a4bb5",
    downloads: 18_700_000,
    author: "IrisShaders",
    gameVersions: ["1.21.4", "1.20.6"],
    loaders: ["Fabric", "NeoForge"],
  },
  {
    slug: "complementary",
    kind: "shader",
    title: "Complementary Reimagined",
    summary: "A high quality shaderpack focused on natural lighting and colors.",
    description: "## Complementary Reimagined\n\nBalanced, high quality shaders with soft shadows and volumetric light.",
    color: "#c1554d",
    downloads: 6_320_000,
    author: "EminGT",
    gameVersions: ["1.21.4", "1.21.3"],
    loaders: ["Iris"],
  },
  {
    slug: "better-mc",
    kind: "modpack",
    title: "Better MC [FABRIC]",
    summary: "A curated modpack that keeps the vanilla feeling while adding content.",
    description: "## Better MC\n\nA curated modpack with quests, new biomes and dungeons, while keeping the vanilla feeling.",
    color: "#c9a227",
    downloads: 3_120_000,
    author: "LunaPixel",
    gameVersions: ["1.21.4", "1.20.1"],
    loaders: ["Fabric"],
  },
  {
    slug: "stay-true",
    kind: "resourcepack",
    title: "Stay True",
    summary: "A vanilla-like resourcepack with subtly improved textures.",
    description: "## Stay True\n\nVanilla-like 16x textures with subtle improvements.",
    color: "#1f6f54",
    downloads: 1_840_000,
    author: "Tom",
    gameVersions: ["1.21.4", "1.21.3", "1.20.6"],
    loaders: ["Minecraft"],
  },
  {
    slug: "fresh-animations",
    kind: "resourcepack",
    title: "Fresh Animations",
    summary: "Make your mobs move like never before with animated entity models.",
    description: "## Fresh Animations\n\nAnimated entity models for every mob, with smooth idle and walk cycles.",
    color: "#3d4d8f",
    downloads: 9_640_000,
    author: "FreshLX",
    gameVersions: ["1.21.4", "1.21.3"],
    loaders: ["Minecraft"],
  },
  {
    slug: "create",
    kind: "mod",
    title: "Create",
    summary: "Aesthetic technology that empowers the player with contraptions.",
    description: "## Create\n\nBuild kinetic contraptions, trains and factories with rotational power.",
    color: "#a1a7b3",
    downloads: 27_500_000,
    author: "simibubi",
    gameVersions: ["1.21.4", "1.20.1"],
    loaders: ["Forge", "Fabric", "NeoForge"],
  },
];

const MOCK_SUMMARY_ZH: Record<string, string> = {
  sodium: "一个大幅提升帧数与帧时间表现的现代化渲染引擎。",
  "fabric-api": "Fabric 加载器的核心 API，绝大多数 Fabric 模组的前置。",
  iris: "现代化的光影加载器，兼容大多数 OptiFine 光影包。",
  complementary: "高质量光影包，专注自然光与色彩表现。",
  "better-mc": "在保留原版手感的前提下加入新群系与地牢的精选整合包。",
  "stay-true": "接近原版的高清材质包，细节更耐看。",
  "fresh-animations": "给生物加上动画模型，动作更自然的材质包扩展。",
  create: "机械动力：用旋转力搭造齿轮、列车与工厂的美术向科技模组。",
};

const MOCK_DESCRIPTION_ZH: Record<string, string> = {
  sodium:
    "## Sodium\n\nSodium 是面向 Minecraft 的现代渲染引擎，显著提升帧数与帧时间表现，并尽量兼容原版渲染管线。\n\n- 大幅提升 FPS\n- 区块渲染更顺\n- 修复若干图形问题",
  "fabric-api":
    "## Fabric API\n\nFabric 加载器的核心 API，多数 Fabric 模组都需要它。\n\n- 生命周期事件\n- 网络通信辅助\n- 注册表同步",
  iris: "## Iris Shaders\n\nIris 是面向 Minecraft 的现代光影加载器，兼容大多数 OptiFine 光影包。",
  complementary: "## Complementary Reimagined\n\n风格均衡的高质量光影，带柔和阴影与体积光。",
  "better-mc": "## Better MC\n\n包含任务、新群系与地牢的精选整合包，同时保留原版手感。",
  "stay-true": "## Stay True\n\n接近原版的 16x 材质，细节上做了一点小改良。",
  "fresh-animations": "## Fresh Animations\n\n给每种生物加上动画模型，待机与行走动作更自然。",
  create: "## Create\n\n用旋转力搭造齿轮、列车与工厂的美术向科技模组。",
};

const MOCK_PROVIDER_LABEL: Record<ContentProvider, string> = {
  modrinth: "Modrinth",
  curseforge: "CurseForge",
  mcim: "MCIM 镜像",
};

function mockProjectFromId(provider: ContentProvider, projectId: string): MockProject | null {
  // 合成目录的 id 形如 provider:slug:index，取第二段才是 slug
  const parts = projectId.split(":");
  const slug = parts.length >= 2 ? parts[1] : projectId;
  const found = MOCK_PROJECTS.find((project) => project.slug === slug) ?? null;
  if (found) return found;
  return provider === "modrinth" ? null : null;
}

/** 虚拟内容库：435 条左右，够验分页与页码省略号。 */
const MOCK_CONTENT_TOTAL = 435;
const MOCK_CATEGORY_POOL: Record<string, string[]> = {
  mod: ["优化", "科技", "实用", "装饰", "冒险"],
  modpack: ["整合包", "冒险", "科技", "轻量"],
  shader: ["光影", "写实", "卡通"],
  resourcepack: ["材质", "写实", "16x"],
  datapack: ["数据包", "实用", "小游戏"],
  plugin: ["插件", "经济", "优化"],
};

function mockContentCatalog(provider: ContentProvider, kind: ContentKindTab): ContentItem[] {
  const bases = MOCK_PROJECTS;
  const items: ContentItem[] = [];
  for (let i = 0; i < MOCK_CONTENT_TOTAL; i += 1) {
    const base = bases[i % bases.length];
    const round = Math.floor(i / bases.length);
    const suffix = round === 0 ? "" : " #" + (round + 1);
    const cats = MOCK_CATEGORY_POOL[kind] ?? ["杂项"];
    items.push({
      id: provider + ":" + base.slug + ":" + i,
      provider,
      kind,
      title:
        // 译文版标题故意和 titleOriginal 明显不同，方便验证「关掉翻译显示原文」
        "译文·" +
        base.title +
        suffix +
        "（" +
        (kind === "mod"
          ? "模组"
          : kind === "modpack"
            ? "整合包"
            : kind === "shader"
              ? "光影"
              : kind === "resourcepack"
                ? "资源包"
                : kind === "plugin"
                  ? "插件"
                  : "数据包") +
        "）",
      titleOriginal: base.title + suffix,
      author: base.author,
      downloads: base.downloads - i * 137,
      follows: i % 4 === 0 ? 37300 - i * 11 : 0,
      updatedAt: secondsAgo(86_400 * (i % 30 + 1)),
      summary: base.summary,
      summaryOriginal: base.summary,
      categories: cats.slice(0, 3),
      loaders: base.loaders,
      iconUrl: mockIconDataUrl(base.title, base.color),
      translated: false,
    });
  }
  return items;
}

/** mock 翻译：命中内置词表返回真译文，否则加「【译】」前缀；次数用完就模拟限流。 */
const MOCK_TRANSLATE_LIMIT_KEY = "gugu.mock.translateLimited";
let mockTranslateCalls = 0;

function mockTranslate(texts: string[]): TranslateResult {
  mockTranslateCalls += 1;
  const forced = window.localStorage.getItem(MOCK_TRANSLATE_LIMIT_KEY) === "1";
  const limited = forced || mockTranslateCalls > 8;
  if (limited) return { results: texts.map(() => null), limited: true };
  const table = new Map<string, string>();
  for (const project of MOCK_PROJECTS) {
    const zhSummary = MOCK_SUMMARY_ZH[project.slug];
    if (zhSummary) table.set(project.summary, zhSummary);
    const zhDescription = MOCK_DESCRIPTION_ZH[project.slug];
    if (zhDescription) table.set(project.description, zhDescription);
  }
  return {
    results: texts.map((text) => table.get(text) ?? (text ? "【译】" + text : null)),
    limited: false,
  };
}

// ------------------------------------- mock 层：M4 背景 / 陶瓦联机 / 检查更新

/** 造一张 320×180 的渐变缩略图当背景素材（不依赖外网图片）。 */
const mockBackgroundCache = new Map<string, string>();
function mockBackgroundDataUrl(seed: number): string {
  const key = "bg-" + seed;
  const cached = mockBackgroundCache.get(key);
  if (cached) return cached;
  const canvas = document.createElement("canvas");
  canvas.width = 320;
  canvas.height = 180;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  const hue = (seed * 37) % 360;
  const gradient = ctx.createLinearGradient(0, 0, 320, 180);
  gradient.addColorStop(0, "hsl(" + hue + ", 58%, 62%)");
  gradient.addColorStop(1, "hsl(" + ((hue + 70) % 360) + ", 62%, 34%)");
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, 320, 180);
  ctx.fillStyle = "rgba(255,255,255,0.86)";
  ctx.font = "bold 44px 'Microsoft YaHei', system-ui, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(String(seed + 1).padStart(2, "0"), 160, 92);
  const url = canvas.toDataURL("image/png");
  mockBackgroundCache.set(key, url);
  return url;
}

const MOCK_WALLPAPER_DIR = "C:\\Program Files (x86)\\Steam\\steamapps\\workshop\\content\\431960";

/** mock 的视频地址根：浏览器里必然加载失败（本地 9 端口没人监听），用来验证错误叠加层。 */
const MOCK_BG_VIDEO_BASE = "http://127.0.0.1:9/mock-bg";

/** 本地图 26 张、壁纸引擎 45 张：为了能看到 20 张一页的分页效果。 */
const MOCK_BACKGROUNDS: Record<"local" | "wallpaper", BackgroundItem[]> = {
  local: Array.from({ length: 26 }, (_, index) => {
    const isVideo = index % 5 === 4;
    const file = "local-" + String(index + 1).padStart(2, "0") + (isVideo ? ".mp4" : ".png");
    const url = mockBackgroundDataUrl(index);
    return {
      file,
      path: mockRoot + "\\backgrounds\\" + file,
      // 视频给一个带 .mp4 的地址（真后端给的是 asset 协议地址）；预览缩略图仍然是画的图
      url: isVideo ? MOCK_BG_VIDEO_BASE + "/backgrounds/" + file : url,
      previewUrl: url,
      isVideo,
      size: 380_000 + index * 21_000,
    };
  }),
  wallpaper: Array.from({ length: 45 }, (_, index) => {
    const isVideo = index % 3 !== 2;
    const file = "wallpaper-" + String(index + 1).padStart(3, "0") + (isVideo ? ".mp4" : ".png");
    const url = mockBackgroundDataUrl(index + 40);
    return {
      file,
      path: MOCK_WALLPAPER_DIR + "\\" + String(100000 + index),
      url: isVideo ? MOCK_BG_VIDEO_BASE + "/workshop/431960/" + file : url,
      previewUrl: url,
      isVideo,
      size: isVideo ? 8_400_000 + index * 120_000 : 1_200_000 + index * 9_000,
    };
  }),
};

function mockBackgroundPage(kind: "local" | "wallpaper", page: number, pageSize: number): BackgroundPage {
  const list = MOCK_BACKGROUNDS[kind];
  const size = pageSize > 0 ? pageSize : 20;
  const start = Math.max(0, page) * size;
  // dir 由后端给：本地图在数据根下的 backgrounds，壁纸引擎是工坊目录
  const dir = kind === "local" ? mockRoot + "\\backgrounds" : MOCK_WALLPAPER_DIR;
  return { items: list.slice(start, start + size), total: list.length, page: Math.max(0, page), pageSize: size, dir };
}

const MOCK_TERA = {
  installed: false,
  version: "1.2.4",
  hosting: false,
  roomCode: null as string | null,
  players: [] as string[],
  ticker: null as number | null,
  logs: [] as string[],
};

function mockTeraLog(line: string): void {
  MOCK_TERA.logs = [...MOCK_TERA.logs, line].slice(-200);
  emitMock({ kind: "tera-log", payload: { line } });
}

function mockTeraState(): TeraState {
  const state: TeraState = {
    hosting: MOCK_TERA.hosting,
    roomCode: MOCK_TERA.roomCode,
    players: [...MOCK_TERA.players],
  };
  emitMock({ kind: "tera-state", payload: state });
  return state;
}

function mockTeraStopTicker(): void {
  if (MOCK_TERA.ticker !== null && typeof window !== "undefined") {
    window.clearInterval(MOCK_TERA.ticker);
    MOCK_TERA.ticker = null;
  }
}

/** 主机模式下每 3 秒往日志里塞一行，模拟有人进出。 */
function mockTeraStartTicker(): void {
  mockTeraStopTicker();
  if (typeof window === "undefined") return;
  let joined = 0;
  MOCK_TERA.ticker = window.setInterval(() => {
    if (!MOCK_TERA.hosting) {
      mockTeraStopTicker();
      return;
    }
    joined += 1;
    if (joined % 3 === 0) {
      const guest = ["Steve", "Alex", "Notch"][joined % 3];
      if (!MOCK_TERA.players.includes(guest)) MOCK_TERA.players = [...MOCK_TERA.players, guest];
      mockTeraLog("[room] " + guest + " 加入了房间");
      mockTeraState();
      return;
    }
    mockTeraLog("[net] 心跳正常，RTT " + (38 + Math.floor(Math.random() * 18)) + " ms");
  }, 3000);
}

const MOCK_UPDATE_OVERRIDE_KEY = "gugu.mock.update";

/** 官网新闻的假数据：3 条带译文（有原文可切），2 条只有原文。 */
const MOCK_NEWS: NewsItem[] = [
  {
    id: "news-1",
    title: "Minecraft 1.21.4 更新发布：苍白花园与新生物",
    summary: "1.21.4 带来了苍白花园生物群系、嘎吱怪与树脂系列方块，同时调整了多名生物的生成与掉落。",
    category: "Minecraft 最新新闻",
    date: "2026-10-24",
    coverUrl: mockBackgroundDataUrl(11),
    readMoreUrl: "https://www.minecraft.net/zh-hans/article/minecraft-1-21-4",
    translated: true,
    titleOriginal: "Minecraft 1.21.4 Release: Pale Garden and New Mobs",
    summaryOriginal:
      "1.21.4 introduces the Pale Garden biome, the Creaking, and resin blocks, along with tweaks to mob spawning and drops.",
  },
  {
    id: "news-2",
    title: "实机演示：在苍白花园里过夜会发生什么",
    summary: "我们搭了一座小木屋，测试嘎吱怪在夜晚的追击逻辑，并给出一份生存清单。",
    category: "Minecraft 最新新闻",
    date: "2026-10-21",
    coverUrl: mockBackgroundDataUrl(23),
    readMoreUrl: "https://www.minecraft.net/zh-hans/article/pale-garden-survival",
    translated: true,
    titleOriginal: "What happens if you spend the night in the Pale Garden",
    summaryOriginal: "We built a cabin, tested the Creaking's night behaviour, and put together a survival checklist.",
  },
  {
    id: "news-3",
    title: "模组作者访谈：把光影做得更像原版",
    summary: "本期访谈聊了聊如何在保留原版色彩的前提下做光影调校，以及社区反馈带来的改动。",
    category: "社区",
    date: "2026-10-18",
    coverUrl: mockBackgroundDataUrl(31),
    readMoreUrl: "https://www.minecraft.net/zh-hans/article/shaders-interview",
    translated: true,
    titleOriginal: "Creator interview: making shaders feel vanilla",
    summaryOriginal: "We talk colour grading, community feedback, and keeping the vanilla look.",
  },
  {
    id: "news-4",
    title: "Snapshot 26w03a is now available",
    summary: "This snapshot focuses on bug fixes, UI polish, and a few new command arguments.",
    category: "Minecraft 最新新闻",
    date: "2026-10-15",
    coverUrl: mockBackgroundDataUrl(44),
    readMoreUrl: "https://www.minecraft.net/en-us/article/minecraft-snapshot-26w03a",
    translated: false,
  },
  {
    id: "news-5",
    title: "Marketplace: fresh bundles for builders",
    summary: "A new set of builder-focused bundles arrives on the Marketplace this week.",
    category: "Minecraft 最新新闻",
    date: "2026-10-11",
    coverUrl: "",
    readMoreUrl: "https://www.minecraft.net/en-us/article/marketplace-builder-bundles",
    translated: false,
  },
];

const MOCK_NEWS_OVERRIDE_KEY = "gugu.mock.news";

// ------------------------------------------ mock 层：M6 服务端（多人 → 服务器）

const MOCK_SERVER_CORES: ServerCore[] = [
  { id: "vanilla", name: "原版", latestVersion: "1.21.4", downloadPage: "https://www.minecraft.net/download/server" },
  { id: "paper", name: "Paper", latestVersion: "1.21.4", downloadPage: "https://papermc.io/downloads/paper" },
  { id: "purpur", name: "Purpur", latestVersion: "1.21.4", downloadPage: "https://purpurmc.org/downloads" },
  { id: "fabric", name: "Fabric", latestVersion: "0.16.9", downloadPage: "https://fabricmc.net/use/server/" },
  { id: "forge", name: "Forge", latestVersion: "1.21.4-54.0.16", downloadPage: "https://files.minecraftforge.net/" },
  { id: "neoforge", name: "NeoForge", latestVersion: "21.4.76", downloadPage: "https://neoforged.net/" },
];

const MOCK_SERVER_VERSIONS: Record<string, ServerVersion[]> = {
  vanilla: [
    { version: "1.21.4", build: null, sizeBytes: 54_000_000, url: "https://www.minecraft.net/download/server" },
    { version: "1.20.1", build: null, sizeBytes: 48_000_000, url: "https://www.minecraft.net/download/server" },
    { version: "1.19.4", build: null, sizeBytes: 46_000_000, url: "https://www.minecraft.net/download/server" },
    { version: "1.18.2", build: null, sizeBytes: 42_000_000, url: "https://www.minecraft.net/download/server" },
  ],
  paper: [
    { version: "1.21.4", build: "232", sizeBytes: 51_000_000, url: "https://papermc.io/downloads/paper" },
    { version: "1.20.1", build: "196", sizeBytes: 47_000_000, url: "https://papermc.io/downloads/paper" },
    { version: "1.19.4", build: "550", sizeBytes: 45_000_000, url: "https://papermc.io/downloads/paper" },
    { version: "1.18.2", build: "388", sizeBytes: 41_000_000, url: "https://papermc.io/downloads/paper" },
  ],
  purpur: [
    { version: "1.21.4", build: "2320", sizeBytes: 52_000_000, url: "https://purpurmc.org/downloads" },
    { version: "1.20.1", build: "2110", sizeBytes: 48_000_000, url: "https://purpurmc.org/downloads" },
  ],
  fabric: [
    { version: "1.21.4", build: "0.16.9", sizeBytes: 2_400_000, url: "https://fabricmc.net/use/server/" },
    { version: "1.20.1", build: "0.15.11", sizeBytes: 2_300_000, url: "https://fabricmc.net/use/server/" },
  ],
  forge: [
    { version: "1.21.4", build: "54.0.16", sizeBytes: 12_000_000, url: "https://files.minecraftforge.net/" },
    { version: "1.20.1", build: "47.2.20", sizeBytes: 11_000_000, url: "https://files.minecraftforge.net/" },
  ],
  neoforge: [
    { version: "1.21.4", build: "21.4.76", sizeBytes: 13_000_000, url: "https://neoforged.net/" },
    { version: "1.20.1", build: "47.1.106", sizeBytes: 12_000_000, url: "https://neoforged.net/" },
  ],
};

// 种子数据：多人 → 服务器里一进来就有两台，插件安装、服务端详情、截图都靠它们
let mockServers: ServerInstance[] = [
  {
    id: "srv-paper-1",
    name: "Paper 生存服",
    core: "paper",
    mcVersion: "1.21.4",
    port: 25565,
    memoryMb: 4096,
    state: "stopped",
    players: 0,
    dir: mockRoot + "\\servers\\srv-paper-1",
  },
  {
    id: "srv-fabric-1",
    name: "Fabric 模组服",
    core: "fabric",
    mcVersion: "1.20.1",
    port: 25566,
    memoryMb: 6144,
    state: "running",
    players: 2,
    dir: mockRoot + "\\servers\\srv-fabric-1",
  },
];
const mockServerLogs = new Map<string, string[]>();
const mockServerTickers = new Map<string, number>();

mockServerLogs.set("srv-paper-1", [
  "[00:00:01] [Server thread/INFO]: Starting minecraft server version 1.21.4",
  "[00:00:02] [Server thread/INFO]: Loading properties",
  "[00:00:03] [Server thread/INFO]: Preparing level \"world\"",
  "[00:00:05] [Server thread/INFO]: Done (3.412s)! For help, type \"help\"",
  "[00:00:06] [Server thread/INFO]: 服务器已就绪，等待玩家加入",
]);
mockServerLogs.set("srv-fabric-1", [
  "[00:00:01] [Server thread/INFO]: Starting minecraft server version 1.20.1",
  "[00:00:01] [Server thread/INFO]: Loading 3 mods: fabric-api, sodium, lithium",
  "[00:00:04] [Server thread/INFO]: Done (2.842s)! For help, type \"help\"",
  "[00:00:09] [Server thread/INFO]: Steve joined the game",
  "[00:00:12] [Server thread/INFO]: Alex joined the game",
]);

function mockServerLog(id: string, line: string): void {
  const lines = [...(mockServerLogs.get(id) ?? []), line].slice(-500);
  mockServerLogs.set(id, lines);
  emitMock({ kind: "server-log", payload: { id, line } });
}

function mockServerState(id: string, state: ServerState, players?: number): void {
  mockServers = mockServers.map((server) => (server.id === id ? { ...server, state, players: players ?? server.players ?? 0 } : server));
  emitMock({ kind: "server-state", payload: { id, state, players } });
}

// ---- 服务端详情：内容 / 存档 / 配置 ----

const mockServerContent = new Map<string, ServerContentItem[]>();
const mockServerSaves = new Map<string, ServerSaveItem[]>();
const mockServerConfig = new Map<string, ServerConfigResult>();

const SERVER_CONTENT_SEED: Record<"mod" | "plugin", ServerContentItem[]> = {
  mod: [
    { fileName: "fabric-api-0.92.2.jar", sizeBytes: 2_100_400, enabled: true, provider: "modrinth", projectId: "fabric-api" },
    { fileName: "sodium-0.5.8.jar", sizeBytes: 1_240_900, enabled: true, provider: "modrinth", projectId: "sodium" },
    { fileName: "lithium-0.12.1.jar", sizeBytes: 780_300, enabled: false, provider: "modrinth", projectId: "lithium" },
  ],
  plugin: [
    { fileName: "EssentialsX-2.20.1.jar", sizeBytes: 3_402_100, enabled: true, provider: "modrinth", projectId: "essentialsx" },
    { fileName: "Vault-1.7.3.jar", sizeBytes: 168_400, enabled: true, provider: "modrinth", projectId: "vault" },
    { fileName: "ProtocolLib-5.1.0.jar", sizeBytes: 1_502_600, enabled: false, provider: "modrinth", projectId: "protocollib" },
  ],
};

function mockServerContentList(serverId: string, kind: "mod" | "plugin"): ServerContentItem[] {
  const key = serverId + ":" + kind;
  const cached = mockServerContent.get(key);
  if (cached) return cached;
  const seed = SERVER_CONTENT_SEED[kind].map((item) => ({ ...item }));
  mockServerContent.set(key, seed);
  return seed;
}

function mockServerSavesOf(serverId: string): ServerSaveItem[] {
  const cached = mockServerSaves.get(serverId);
  if (cached) return cached;
  const seed: ServerSaveItem[] = [
    { name: "world", sizeBytes: 512_344_064, updatedAt: secondsAgo(2_400) },
    { name: "world_nether", sizeBytes: 84_221_952, updatedAt: secondsAgo(2_400) },
    { name: "world_the_end", sizeBytes: 41_893_888, updatedAt: secondsAgo(86_400) },
  ];
  mockServerSaves.set(serverId, seed);
  return seed;
}

function mockServerConfigOf(serverId: string): ServerConfigResult {
  const cached = mockServerConfig.get(serverId);
  if (cached) return cached;
  const server = mockServers.find((item) => item.id === serverId);
  const fields: Record<string, string> = {
    "server-port": String(server?.port ?? 25565),
    motd: "咕咕启动器开的世界",
    "max-players": "20",
    "online-mode": "true",
    difficulty: "normal",
    gamemode: "survival",
    "level-name": "world",
    "view-distance": "10",
    "simulation-distance": "10",
    pvp: "true",
    "allow-flight": "false",
    "white-list": "false",
  };
  const text = [
    "#Minecraft server properties",
    "#" + new Date().toISOString(),
    ...Object.entries(fields).map(([key, value]) => key + "=" + value),
    "# 下面这行是启动器不认识的，写回时必须原样保留",
    "custom-unrecognised-key=keep-me",
  ].join("\n");
  const value: ServerConfigResult = { text, fields };
  mockServerConfig.set(serverId, value);
  return value;
}

/** 运行中的服务器每 3 秒来一行日志，偶尔有人进来。 */
function mockServerTicker(id: string): void {
  if (typeof window === "undefined") return;
  const existing = mockServerTickers.get(id);
  if (existing) window.clearInterval(existing);
  let tick = 0;
  const timer = window.setInterval(() => {
    const server = mockServers.find((item) => item.id === id);
    if (!server || server.state !== "running") {
      window.clearInterval(timer);
      mockServerTickers.delete(id);
      return;
    }
    tick += 1;
    if (tick % 4 === 0) {
      const players = Math.min(4, (server.players ?? 1) + (tick % 8 === 0 ? 1 : 0));
      mockServerState(id, "running", players);
      mockServerLog(id, "[Server thread/INFO]: Steve joined the game");
      return;
    }
    mockServerLog(id, "[Server thread/INFO]: Can't keep up! Is the server overloaded? Running " + (20 + tick) + "ms behind");
  }, 3000);
  mockServerTickers.set(id, timer);
}

function mockServerStopTicker(id: string): void {
  const timer = mockServerTickers.get(id);
  if (timer && typeof window !== "undefined") window.clearInterval(timer);
  mockServerTickers.delete(id);
}

// ------------------------------------------------------------ 对外 API

export const api = {
  appInfo(): Promise<AppInfo> {
    return call("app_info", undefined, () => ({
      name: PRODUCT,
      nameEn: PRODUCT_EN,
      version: VERSION,
      identifier: IDENTIFIER,
      tauriVersion: "2",
      dataRoot: MOCK_PATHS.root,
    }));
  },

  pathsInit(): Promise<PathsLayout> {
    return call("paths_init", undefined, () => MOCK_PATHS);
  },

  settingsGet(): Promise<Settings> {
    return call("settings_get", undefined, () => readMockSettings());
  },

  settingsSet(settings: Settings): Promise<Settings> {
    return call("settings_set", { settings }, () => {
      const next = { ...settings };
      window.localStorage.setItem(MOCK_SETTINGS_KEY, JSON.stringify(next));
      return next;
    }).then((saved) => {
      // 设置变了：字体 / Java / 背景这些缓存作废，但同一次保存不重复请求
      invalidateApiCache();
      return saved;
    });
  },

  versionsList(forceRefresh = false): Promise<VersionManifest> {
    // 后端字段名可能是 versionType / type / version_type，统一在 normalizeManifest 里折平
    return call("versions_list", { forceRefresh }, () => MOCK_VERSIONS).then((manifest) => normalizeManifest(manifest) ?? manifest);
  },

  instancesList(): Promise<Instance[]> {
    return call("instances_list", undefined, () => mockInstances);
  },

  instanceCreate(payload: InstanceCreate): Promise<Instance> {
    return call("instance_create", { payload }, () => {
      const created: Instance = {
        id: mockInstances.length + 1,
        name: payload.name,
        mcVersion: payload.mcVersion,
        loader: payload.loader ?? "Vanilla",
        loaderVersion: payload.loaderVersion ?? "",
        icon: "",
        pinnedHome: false,
        playTimeSecs: 0,
        createdAt: new Date().toISOString(),
        lastPlayedAt: null,
      };
      mockInstances = [...mockInstances, created];
      return created;
    });
  },

  instanceDelete(id: number): Promise<void> {
    return call("instance_delete", { id }, () => {
      mockInstances = mockInstances.filter((item) => item.id !== id);
    });
  },

  downloadSources(): Promise<DownloadSource[]> {
    return call("download_sources", undefined, () => MOCK_SOURCES);
  },

  /**
   * 给一个下载源测速。TODO(M2-A): 后端 sources_probe —— 真发一次小请求测往返延迟，
   * 自定义源用 target.base 当根地址；不可达时 ok=false、latencyMs=null。
   */
  sourcesProbe(target: SourceProbeTarget): Promise<SourceProbe> {
    return call("sources_probe", { target }, () => mockSourceProbe(target));
  },

  /**
   * 测一次资源描述翻译接口（用**已保存**的设置，前端点「测试连接」前会先保存草稿）。
   * TODO(M2-A): 后端 translation_test —— 内置源探一次内置接口；自定义源按 OpenAI 兼容
   * 约定 POST {baseUrl}/chat/completions，返回模型回显或具体错误（401 / 超时 / JSON 不符）。
   */
  translationTest(): Promise<TranslationTestResult> {
    return call("translation_test", undefined, () => mockTranslationTest(readMockSettings()));
  },

  /**
   * 抽一句每日一言：后端代发请求（前端跨域会被拦），留空的地址走后端内置默认。
   * TODO(M2-A): 后端 fun_saying —— 读 settings.sayingApiUrl，自动挑 text/saying/hitokoto/content 等字段。
   */
  funSaying(): Promise<SayingResult> {
    return call("fun_saying", undefined, () => {
      const text = MOCK_SAYINGS[mockSayingIndex % MOCK_SAYINGS.length];
      mockSayingIndex += 1;
      return { ok: true, text, source: "uapis.cn" };
    });
  },

  /**
   * GitHub 直链下载：立刻返回 taskId，进度走 install://progress / install://error。
   * TODO(M2-A): 后端 github_download —— 校验 URL、套加速节点、写进同一条下载任务流。
   */
  githubDownload(request: GithubDownloadRequest): Promise<{ taskId: string }> {
    return call("github_download", { request }, () => {
      const url = request.url.trim();
      if (!url) throw new Error("请先填写 GitHub 下载地址");
      if (!/^https?:\/\//i.test(url)) throw new Error("地址格式不正确：" + url);
      if (!GITHUB_URL.test(url)) throw new Error("这不是 GitHub 链接：" + url);
      const dir = request.dir.trim() || mockRoot + "\\DL";
      const name = request.fileName.trim() || mockNameFromUrl(url) || "download.bin";
      const taskId = "mock-gh-" + Date.now().toString(36);
      mockTasks = [
        {
          id: taskId,
          kind: "github",
          title: name,
          stage: "正在连接 GitHub",
          phase: "downloading",
          progress: 0.01,
          bytesPerSec: 1_800_000,
          receivedBytes: 0,
          totalBytes: 26_214_400,
          targetDir: dir,
          createdAt: new Date().toISOString(),
          finishedAt: null,
          error: null,
        },
        ...mockTasks,
      ];
      ensureMockTicker();
      return { taskId };
    });
  },

  /** 取消一条 GitHub 下载任务。TODO(M2-A): 后端 github_cancel。 */
  githubCancel(taskId: string): Promise<void> {
    return call("github_cancel", { taskId }, () => {
      mockTasks = mockTasks.filter((task) => task.id !== taskId);
    });
  },

  /** 打开 GitHub 下载的保存目录。TODO(M2-A): 后端 github_open_folder。 */
  githubOpenFolder(): Promise<void> {
    return call("github_open_folder", undefined, () => {
      /* 浏览器里没有资源管理器可开 */
    });
  },

  /** 选目录对话框，返回选中的绝对路径；取消返回空字符串。TODO(M2-A): 后端 pick_directory。 */
  pickDirectory(): Promise<string> {
    return call("pick_directory", undefined, () => mockRoot + "\\DL");
  },

  /** 系统里能显示中文的字体；后端枚举失败时返回空数组（前端提示并回落默认字体）。 */
  fontsList(): Promise<FontChoice[]> {
    return cached("fonts_list", () => call("fonts_list", undefined, () => MOCK_FONTS));
  },

  /** 只读更新状态（不发请求），关于页轮询用。 */
  updateStatus(): Promise<UpdateStatus> {
    return call("update_status", undefined, () => MOCK_UPDATE_STATUS);
  },

  /**
   * 立即检查一次更新（M4 形状：hasUpdate / latest / current / error）。
   * 浏览器预览里用 localStorage 的 gugu.mock.update = available|error 模拟另外两种结果。
   */
  updateCheck(): Promise<UpdateCheckResult> {
    return call("update_check", undefined, () => {
      const override = window.localStorage.getItem(MOCK_UPDATE_OVERRIDE_KEY);
      if (override === "available") {
        return { hasUpdate: true, latest: "1.0.1", current: VERSION, error: null };
      }
      if (override === "error") {
        return { hasUpdate: false, latest: null, current: VERSION, error: "网络不可达（已重试 2 次）" };
      }
      return { hasUpdate: false, latest: null, current: VERSION, error: null };
    });
  },

  /** manual 模式点「立即更新」；M1 只返回状态，实际安装在 M2 接入。 */
  updateInstall(): Promise<UpdateStatus> {
    return call("update_install", undefined, () => ({ ...MOCK_UPDATE_STATUS, message: "安装流程将在 M2 接入" }));
  },

  /** 启动实例：M1 后端返回「未实现」，这里把错误原样抛给调用方显示。 */
  instanceLaunch(id: number): Promise<void> {
    return call("instance_launch", { id }, () => {
      throw new Error("M1 骨架：启动器内核尚未接入");
    });
  },

  // ------------------------------------------------------ 下载中心（M2-B）

  /** 全部下载任务（进行中 + 已完成）。TODO(M2-A): 后端 downloads_list。 */
  downloadsList(): Promise<DownloadTask[]> {
    return call("downloads_list", undefined, () => mockTasks.map((task) => ({ ...task })));
  },

  /** 取消一条进行中的任务。TODO(M2-A): 后端 download_cancel。 */
  downloadCancel(id: string): Promise<void> {
    return call("download_cancel", { id }, () => {
      mockTasks = mockTasks.filter((task) => task.id !== id);
    });
  },

  /** 打开任务落地目录。TODO(M2-A): 后端 download_open_folder。 */
  downloadOpenFolder(id: string): Promise<void> {
    return call("download_open_folder", { id }, () => {
      /* 浏览器里没有资源管理器可开 */
    });
  },

  /**
   * 安装一个游戏版本（新建实例后的第一步）。
   * 立刻返回，进度与失败都走 install://progress / install://error。
   * TODO(M2-A): 后端 version_install。
   */
  versionInstall(instanceId: number, versionId: string): Promise<void> {
    return call("version_install", { instanceId, versionId }, () => {
      startMockInstall(versionId + " 版本安装", mockRoot + "\\versions\\" + versionId);
    });
  },

  // ------------------------------------------------------ 实例（M2-B）

  /** 打开整个实例目录。TODO(M2-A): 后端 instance_open_folder。 */
  instanceOpenFolder(id: number): Promise<void> {
    return call("instance_open_folder", { id }, () => {
      /* 浏览器里没有资源管理器可开 */
    });
  },

  /** 实例详情页「设置」的数据。TODO(M2-A): 后端 instance_settings_get。 */
  instanceSettingsGet(id: number): Promise<InstanceSettings> {
    return call("instance_settings_get", { id }, () => ({ ...mockInstanceSettings(id) }));
  },

  /**
   * 保存实例设置；改名时后端要同步改目录名与 versions 目录下的文件夹。
   * TODO(M2-A): 后端 instance_settings_set。
   */
  instanceSettingsSet(id: number, settings: InstanceSettings): Promise<InstanceSettings> {
    return call("instance_settings_set", { id, settings }, () => {
      MOCK_INSTANCE_SETTINGS.set(id, { ...settings });
      mockInstances = mockInstances.map((item) => (item.id === id ? { ...item, name: settings.name } : item));
      return { ...settings };
    });
  },

  /**
   * 选择实例图标：弹文件选择框（只收 png），把选中的文件复制成 <实例目录>/icon.png。
   * 返回新的图标绝对路径；用户取消时返回空字符串。
   * TODO(M2-A): 后端 instance_pick_icon。
   */
  instancePickIcon(id: number): Promise<string> {
    return call("instance_pick_icon", { id }, () => {
      const next = mockInstanceSettings(id);
      next.iconPath = mockRoot + "\\instances\\" + id + "\\icon.png";
      MOCK_INSTANCE_SETTINGS.set(id, next);
      mockInstances = mockInstances.map((item) => (item.id === id ? { ...item, icon: next.iconPath } : item));
      return next.iconPath;
    });
  },

  /** 系统里可用的 Java 运行时（后端已去重）。TODO(M2-A): 后端 java_list。 */
  javaList(): Promise<JavaRuntime[]> {
    return cached("java_list", () => call("java_list", undefined, () => MOCK_JAVA));
  },

  /** Java 下载源。TODO(M5): 后端 java_sources。 */
  javaSources(): Promise<JavaSource[]> {
    return cached("java_sources", () => call("java_sources", undefined, () => MOCK_JAVA_SOURCES));
  },

  /** 给各源测一次延迟。TODO(M5): 后端 java_probe_sources。 */
  javaProbeSources(): Promise<JavaSourceProbe[]> {
    return call("java_probe_sources", undefined, () => [
      { sourceId: "bellsoft", ok: true, latencyMs: 171 },
      { sourceId: "adoptium", ok: true, latencyMs: 355 },
      { sourceId: "tuna", ok: true, latencyMs: 177 },
      { sourceId: "zulu", ok: true, latencyMs: 300 },
      { sourceId: "oracle", ok: true, latencyMs: 72 },
    ]);
  },

  /**
   * 某个加载器在某 MC 版本下可用的构建。TODO(M15): 后端 loader_versions。
   * 不支持的组合后端抛错（如「该版本没有 Forge 构建」），界面显示后端原文。
   */
  loaderVersions(request: { loader: "fabric" | "forge" | "neoforge"; mcVersion: string }): Promise<LoaderVersion[]> {
    return call("loader_versions", { ...request }, () => mockLoaderVersions(request.loader, request.mcVersion));
  },

  /**
   * 可下载的 Java 版本。TODO(M15): 后端 java_versions ——
   * source 为空：并发查全部源，同版本保留延迟最低的那条；指定 source：返回该源所有 major 的版本。
   */
  javaVersions(request: { source?: string; major?: number }): Promise<JavaVersion[]> {
    return call("java_versions", { ...request }, () => {
      const source = request.source && request.source !== "auto" ? request.source : "";
      const major = request.major ?? 0;
      const byMajor = (item: JavaVersion) => !major || item.major === major;
      if (source) {
        // 单源：这个源下所有 major 的版本，按 major 倒序
        return MOCK_JAVA_VERSIONS.filter((item) => item.source === source && byMajor(item)).sort((a, b) => b.major - a.major);
      }
      // 全部：同版本只留延迟最低的那条
      const best = new Map<string, JavaVersion>();
      for (const item of MOCK_JAVA_VERSIONS) {
        if (!byMajor(item)) continue;
        const prev = best.get(item.version);
        if (!prev || (MOCK_JAVA_LATENCY[item.source] ?? 999) < (MOCK_JAVA_LATENCY[prev.source] ?? 999)) {
          best.set(item.version, item);
        }
      }
      return [...best.values()].sort((a, b) => b.major - a.major || a.version.localeCompare(b.version));
    });
  },

  /** 下载并安装一个 Java。TODO(M5): 后端 java_install。 */
  javaInstall(request: { source: string; version: string; major: number }): Promise<{ taskId: string }> {
    return call("java_install", { ...request }, () => {
      startMockInstall("Java " + request.major + " · " + request.version, mockRoot + "\\runtimes\\java-" + request.major, "install");
      const found = MOCK_JAVA_VERSIONS.find((item) => item.version === request.version && item.source === request.source);
      if (found && !mockInstalledJava.some((item) => item.version === found.version && item.major === found.major)) {
        mockInstalledJava.push(found);
      }
      return { taskId: mockTasks[0].id };
    });
  },

  /** 整合包安装：直接创建一个新实例，返回 taskId + instanceId。TODO(M9): 后端 modpack_install。 */
  modpackInstall(request: ModpackInstallRequest): Promise<{ taskId: string; instanceId: number }> {
    return call("modpack_install", { ...request }, () => {
      const project = mockProjectFromId(request.provider, request.projectId);
      const name = request.instanceName?.trim() || project?.title || "整合包实例";
      const created: Instance = {
        id: (mockInstances.reduce((max, item) => Math.max(max, item.id), 0) || 0) + 1,
        name: name,
        mcVersion: project?.gameVersions[0] ?? "1.21.4",
        loader: project?.loaders[0] ?? "Fabric",
        loaderVersion: "",
        icon: "",
        pinnedHome: false,
        playTimeSecs: 0,
        createdAt: new Date().toISOString(),
        lastPlayedAt: null,
        packName: project?.title ?? name,
        packState: "installed",
      };
      mockInstances = [...mockInstances, created];
      startMockInstall("安装整合包：" + name, mockRoot + "\\instances\\" + created.id, "install");
      if (typeof window !== "undefined" && window.localStorage.getItem("gugu.mock.modpack") === "fail") {
        mockInstances = mockInstances.map((item) => (item.id === created.id ? { ...item, packState: "incomplete" } : item));
        throw new Error("整合包文件校验失败（mock）：已创建实例，可稍后重试安装");
      }
      return { taskId: mockTasks[0].id, instanceId: created.id };
    });
  },

  // ------------------------------------ 检查更新 / 进入存档（M10）

  /** 检查某类文件的更新。TODO(M10): 后端 mods_check_updates。 */
  modsCheckUpdates(request: { instanceId: number; kind: string }): Promise<ModCheckResult> {
    return call("mods_check_updates", { ...request }, () => {
      const updatable = request.kind === "mod" ? 3 : request.kind === "resourcepack" ? 2 : 0;
      const updates: ModUpdateItem[] = Array.from({ length: updatable }, (_, index) => ({
        fileName: request.kind + "-sample-" + (index + 1) + ".jar",
        path: mockRoot + "\\instances\\" + request.instanceId + "\\" + request.kind + "s",
        kind: request.kind,
        currentVersion: "1." + (index + 2) + ".0",
        latestVersion: "1." + (index + 3) + ".0",
        downloadUrl: "https://example.invalid/" + request.kind + "-" + (index + 1),
        sha1: null,
      }));
      return {
        checked: 12,
        updates: updates,
        failed: updatable > 0 ? [{ fileName: "unknown-" + request.kind + ".jar", reason: "未记录安装信息且指纹未命中" }] : [],
      };
    });
  },

  /** 应用更新。TODO(M10): 后端 mods_apply_updates。 */
  modsApplyUpdates(request: { instanceId: number; kind: string; items: ModUpdateItem[] }): Promise<ModApplyResult> {
    return call("mods_apply_updates", { ...request }, () => ({ updated: request.items.length, failed: [] }));
  },

  /** 启动并直接进入存档。TODO(M10): 后端 instance_launch_with_save。 */
  instanceLaunchWithSave(request: { instanceId: number; levelName: string }): Promise<{ pid: number }> {
    return call("instance_launch_with_save", { ...request }, () => {
      const target = mockInstances.find((item) => item.id === request.instanceId);
      const major = Number((target?.mcVersion ?? "1.21").split(".")[1] ?? 21);
      if (major < 12) throw new Error("该版本不支持直接进入存档，请手动在游戏内选择");
      return { pid: 4242 + request.instanceId };
    });
  },

  // -------------------------------------- 外部启动器导入（M9 HMCL / PCL2）

  /** 扫描 HMCL / PCL2 根目录。TODO(M9): 后端 external_scan。 */
  externalScan(request: { root: string }): Promise<ExternalScanResult> {
    return call("external_scan", { ...request }, () => {
      const root = request.root.trim();
      if (!root) throw new Error("请先选择一个目录");
      if (/notfound|invalid/i.test(root)) throw new Error("这不是 HMCL / PCL2 的目录（没找到 versions 子目录）");
      const launcher: ExternalScanResult["launcher"] = /pcl/i.test(root) ? "pcl2" : /hmcl/i.test(root) ? "hmcl" : "hmcl";
      const base = [
        { id: "ext-1", name: "HMCL 1.20.1 Fabric", mcVersion: "1.20.1", loader: "Fabric", isolated: true, modCount: 12, broken: null },
        { id: "ext-2", name: "PCL2 1.19.4 原版", mcVersion: "1.19.4", loader: "Vanilla", isolated: false, modCount: 0, broken: null },
        { id: "ext-3", name: "1.16.5 Forge 整合", mcVersion: "1.16.5", loader: "Forge", isolated: false, modCount: 43, broken: null },
        { id: "ext-4", name: "损坏的版本", mcVersion: "1.18.2", loader: "Forge", isolated: false, modCount: 3, broken: "缺少 version.json（可能被手动删过）" },
        { id: "ext-5", name: "1.21.1 NeoForge", mcVersion: "1.21.1", loader: "NeoForge", isolated: true, modCount: 5, broken: null },
      ];
      return {
        launcher: launcher,
        minecraftDir: root,
        candidates: base.map((item) => ({
          id: item.id,
          name: item.name,
          mcVersion: item.mcVersion,
          loader: item.loader,
          loaderVersion: item.loader === "Vanilla" ? null : "0.16.9",
          path: root + "\\versions\\" + item.name,
          isolated: item.isolated,
          hasMods: item.modCount > 0,
          modCount: item.modCount,
          sizeBytes: 380_000_000 + item.modCount * 12_000_000,
          javaMajor: Number(item.mcVersion.split(".")[1]) >= 20 ? 17 : 8,
          broken: item.broken,
        })),
      };
    });
  },

  /** 导入选中的版本：link=引用原目录，copy=复制到咕咕目录。TODO(M9): 后端 external_import。 */
  externalImport(request: { root: string; ids: string[]; mode: "link" | "copy" }): Promise<ExternalImportResult> {
    return call("external_import", { ...request }, () => {
      const imported: Instance[] = [];
      const failed: Array<{ id: string; reason: string }> = [];
      let nextId = (mockInstances.reduce((max, item) => Math.max(max, item.id), 0) || 0) + 1;
      for (const id of request.ids) {
        if (id === "ext-broken" || id === "ext-4") {
          failed.push({ id, reason: "缺少 version.json" });
          continue;
        }
        const created: Instance = {
          id: nextId,
          name: (request.mode === "copy" ? "[复制] " : "") + "导入实例 " + id.replace("ext-", ""),
          mcVersion: "1.20.1",
          loader: "Fabric",
          loaderVersion: "",
          icon: "",
          pinnedHome: false,
          playTimeSecs: 0,
          createdAt: new Date().toISOString(),
          lastPlayedAt: null,
        };
        nextId += 1;
        imported.push(created);
      }
      mockInstances = [...mockInstances, ...imported];
      return { instances: imported, failed };
    });
  },

  // ------------------------------------------------ 存储统计 / 清理（M7）

  /** 数据目录各分项体积。TODO(M7): 后端 storage_stats。 */
  storageStats(): Promise<StorageStats> {
    return call("storage_stats", undefined, () => {
      const base: Array<Omit<StorageStatItem, "bytes">> = [
        { key: "instances", label: "实例", files: 12_034, path: mockRoot + "\\instances" },
        { key: "versions", label: "版本与依赖库", files: 3_921, path: mockRoot + "\\versions" },
        { key: "libraries", label: "依赖库", files: 1_240, path: mockRoot + "\\libraries" },
        { key: "assets", label: "资源文件", files: 8_120, path: mockRoot + "\\assets" },
        { key: "cache", label: "下载缓存", files: 480, path: mockRoot + "\\cache" },
        { key: "logs", label: "日志", files: 36, path: mockRoot + "\\logs" },
        { key: "runtimes", label: "Java 运行时", files: 210, path: mockRoot + "\\runtimes" },
      ];
      const items: StorageStatItem[] = base.map((item) => ({ ...item, bytes: mockStorageBytes[item.key] }));
      const totalBytes = items.reduce((sum, item) => sum + item.bytes, 0);
      return { totalBytes, items };
    });
  },

  /** 清理某个分项，返回释放的字节数。TODO(M7): 后端 storage_clean。 */
  storageClean(key: StorageKey): Promise<{ freedBytes: number }> {
    return call("storage_clean", { key }, () => {
      const freed = key === "cache" ? mockStorageBytes.cache : key === "logs" ? mockStorageBytes.logs : Math.round(mockStorageBytes[key] * 0.25);
      mockStorageBytes[key] = Math.max(0, mockStorageBytes[key] - freed);
      if (key === "versions") mockStorageBytes.libraries = Math.round(mockStorageBytes.libraries * 0.7);
      return { freedBytes: freed };
    });
  },

  /** 最近游玩的实例（按 lastPlayed 倒序）。TODO(M7): 后端 instances_recent。 */
  instancesRecent(request: { limit: number }): Promise<Instance[]> {
    return call("instances_recent", { ...request }, () =>
      mockInstances
        .slice()
        .sort((a, b) => (b.lastPlayedAt ?? "").localeCompare(a.lastPlayedAt ?? ""))
        .slice(0, Math.max(1, request.limit)),
    );
  },

  // ------------------------------------------------ 数据目录 / 重启（M5）

  /** 当前数据目录与体积。TODO(M5): 后端 data_dir_get。 */
  dataDirGet(): Promise<DataDirInfo> {
    return call("data_dir_get", undefined, () => ({
      path: MOCK_DATA_DIR.path,
      sizeBytes: MOCK_DATA_DIR.sizeBytes,
      fileCount: MOCK_DATA_DIR.fileCount,
      isDefault: true,
    }));
  },

  /** 选一个目标目录；取消返回空串。TODO(M5): 后端 data_dir_pick。 */
  dataDirPick(): Promise<string> {
    return call("data_dir_pick", undefined, () => "D:\\minecraft-data");
  },

  /** 迁移数据目录：立刻返回 taskId，进度走下载中心。TODO(M5): 后端 data_dir_migrate。 */
  dataDirMigrate(request: { target: string }): Promise<{ taskId: string; restartRequired?: boolean }> {
    return call("data_dir_migrate", { ...request }, () => {
      startMockInstall("迁移数据目录 → " + request.target, request.target, "install");
      return { taskId: mockTasks[0].id, restartRequired: true };
    });
  },

  /** 恢复默认数据目录（只改设置，不搬文件）。TODO(M5): 后端 data_dir_reset_default。 */
  dataDirResetDefault(): Promise<void> {
    return call("data_dir_reset_default", undefined, () => {
      /* mock 里默认目录就是 mockRoot，不用动 */
    });
  },

  /** 重启启动器。TODO(M5): 后端 app_restart。 */
  appRestart(): Promise<void> {
    return call("app_restart", undefined, () => {
      window.location.reload();
    });
  },

  /** 实例的日志文件列表（.mc\\logs\\*.log）。TODO(M2-A): 后端 logs_list。 */
  logsList(_instanceId: number): Promise<LogFile[]> {
    return call("logs_list", { instanceId: _instanceId }, () => MOCK_LOG_FILES);
  },

  /** 读一个日志文件的全文。TODO(M2-A): 后端 log_read。 */
  logRead(_instanceId: number, name: string): Promise<string> {
    return call("log_read", { instanceId: _instanceId, name }, () => mockLogContent(name));
  },

  /** 打开日志目录。TODO(M2-A): 后端 log_open_folder。 */
  logOpenFolder(_instanceId: number): Promise<void> {
    return call("log_open_folder", { instanceId: _instanceId }, () => {
      /* 浏览器里没有资源管理器可开 */
    });
  },

  // ------------------------------------------------------ 实例内容（M3-2）

  /** 列出实例内的模组 / 资源包 / 光影。TODO(M3-2): 后端 mods_list。 */
  modsList(instanceId: number, kind: ContentFileKind): Promise<ModFileItem[]> {
    return call("mods_list", { instanceId, kind }, () => mockInstanceFiles(instanceId, kind).map((item) => ({ ...item })));
  },

  /** 启用/禁用：改后缀 .jar ⇄ .jar.disabled。返回改完的整份列表。TODO(M3-2): 后端 mods_set_enabled。 */
  modsSetEnabled(instanceId: number, kind: ContentFileKind, file: string, enabled: boolean): Promise<ModFileItem[]> {
    return call("mods_set_enabled", { instanceId, kind, file, enabled }, () => {
      const list = mockInstanceFiles(instanceId, kind).map((item) => {
        if (item.file !== file) return item;
        const base = item.file.endsWith(".disabled") ? item.file.slice(0, -".disabled".length) : item.file;
        return { ...item, file: enabled ? base : base + ".disabled", enabled };
      });
      return mockStoreFiles(instanceId, kind, list).map((item) => ({ ...item }));
    });
  },

  /** 删除一个文件。TODO(M3-2): 后端 mods_delete。 */
  modsDelete(instanceId: number, kind: ContentFileKind, file: string): Promise<ModFileItem[]> {
    return call("mods_delete", { instanceId, kind, file }, () => {
      const list = mockInstanceFiles(instanceId, kind).filter((item) => item.file !== file);
      return mockStoreFiles(instanceId, kind, list).map((item) => ({ ...item }));
    });
  },

  /** 导入本地文件（后端弹文件框拿到绝对路径）。TODO(M3-2): 后端 mods_import。 */
  modsImport(instanceId: number, kind: ContentFileKind, paths: string[]): Promise<ModFileItem[]> {
    return call("mods_import", { instanceId, kind, paths }, () => {
      // 空数组＝让后端自己弹文件框挑；mock 里直接造两个文件，方便自检
      const picked =
        paths.length > 0
          ? paths
          : [
              "C:\\Users\\Public\\mods\\new-block-1.0.0.jar",
              "C:\\Users\\Public\\mods\\new-util-0.3.2.jar",
            ];
      const list = mockInstanceFiles(instanceId, kind);
      const added = picked.map((path, index) => {
        const file = path.split(/[\\/]/).pop() ?? "imported-" + index + ".jar";
        return mockFileItem(file, 419_430 + index * 1024, null, null, null);
      });
      return mockStoreFiles(instanceId, kind, [...list, ...added]).map((item) => ({ ...item }));
    });
  },

  /** 打开模组/资源包/光影目录。TODO(M3-2): 后端 mods_open_folder。 */
  modsOpenFolder(instanceId: number, kind: ContentFileKind): Promise<void> {
    return call("mods_open_folder", { instanceId, kind }, () => {
      /* 浏览器里没有资源管理器可开 */
    });
  },

  /** 实例存档列表。TODO(M3-2): 后端 saves_list。 */
  savesList(instanceId: number): Promise<SaveItem[]> {
    return call("saves_list", { instanceId }, () => mockSaves(instanceId).map((item) => ({ ...item })));
  },

  /** 删除存档（连同目录）。TODO(M3-2): 后端 saves_delete。 */
  savesDelete(instanceId: number, name: string): Promise<SaveItem[]> {
    return call("saves_delete", { instanceId, name }, () => {
      const next = mockSaves(instanceId).filter((item) => item.name !== name);
      MOCK_SAVES.set(instanceId, next);
      return next.map((item) => ({ ...item }));
    });
  },

  /** 导出存档为 zip，返回落地路径。TODO(M3-2): 后端 saves_export。 */
  savesExport(instanceId: number, name: string): Promise<{ path: string }> {
    return call("saves_export", { instanceId, name }, () => ({
      path: mockRoot + "\\DL\\saves\\" + name + ".zip",
    }));
  },

  /** 打开存档目录。TODO(M3-2): 后端 saves_open_folder。 */
  savesOpenFolder(instanceId: number): Promise<void> {
    return call("saves_open_folder", { instanceId }, () => {
      /* 浏览器里没有资源管理器可开 */
    });
  },

  /** 导出实例包：立刻返回 taskId，进度走下载中心（kind=export）。TODO(M3-2): 后端 instance_export。 */
  instanceExport(id: number, target: string): Promise<{ taskId: string }> {
    return call("instance_export", { id, target }, () => {
      const instance = mockInstances.find((item) => item.id === id);
      const name = (instance?.name ?? "实例").replace(/[\\/:*?"<>|]/g, "_");
      startMockInstall(name + ".zip 导出", target || mockRoot + "\\DL", "export");
      return { taskId: mockTasks[0].id };
    });
  },

  // ---------------------------------------------------------- 皮肤（M3-2）

  /** 内置皮肤（Steve / Alex），不落盘；界面在列表最前面固定两项。TODO(M5): 后端 skin_default。 */
  skinDefault(model: "default" | "slim"): Promise<SkinItem> {
    return call("skin_default", { model }, () => mockBuiltinSkin(model));
  },

  /** 按正版玩家名取皮肤（uapis 公开接口），返回已落盘的条目。TODO(M5): 后端 skin_fetch_mojang。 */
  skinFetchMojang(request: { username: string }): Promise<SkinItem> {
    return call("skin_fetch_mojang", { ...request }, () => mockFetchMojang(request.username));
  },

  /** 本地 .mc\\skins\\ 下的皮肤列表（带 data URL 缩略图）。TODO(M3-2): 后端 skins_list。 */
  skinsList(): Promise<SkinItem[]> {
    return call("skins_list", undefined, () => mockSkins.map((skin) => ({ ...skin })));
  },

  /** 删除本地皮肤（只删 .mc\\skins 里的文件，不动账号外观）。TODO(M14): 后端 skins_delete。 */
  skinsDelete(request: { name: string }): Promise<void> {
    return call("skins_delete", { ...request }, () => {
      const target = mockSkins.find((skin) => skin.name === request.name);
      if (!target) throw new Error("皮肤不存在：" + request.name);
      mockSkins = mockSkins.filter((skin) => skin.name !== request.name);
      if (mockCurrentSkin === target.path) mockCurrentSkin = "";
    });
  },

  /** 把某个皮肤设为当前皮肤。TODO(M3-2): 后端 skin_apply。 */
  skinApply(path: string): Promise<void> {
    return call("skin_apply", { path }, () => {
      mockCurrentSkin = path;
    });
  },

  /** 上传一张 PNG（base64），落到 .mc\\skins\\ 并返回新条目。TODO(M3-2): 后端 skin_upload。 */
  /**
   * 正版账号上传皮肤（离线/第三方账号没有这个能力，调用方需自行判断）。
   * 磁盘上有文件的皮肤传 pngPath；内置 / 打包资源的皮肤没有路径，改传 pngBase64。
   * TODO(M11): 后端 ms_skin_upload 已加 pngBase64 参数，两者都传时以 base64 为准。
   */
  msSkinUpload(request: {
    pngPath?: string;
    pngBase64?: string;
    variant: "classic" | "slim";
  }): Promise<{ ok: boolean; variant: string; uploadedAt: string; capeSynced: boolean }> {
    // 自检用：记下这次上传到底带了路径还是 base64（界面不显示）
    if (typeof window !== "undefined") {
      const host = window as unknown as Record<string, unknown>;
      const seen = Array.isArray(host.__guguSkinUpload) ? (host.__guguSkinUpload as unknown[]) : [];
      host.__guguSkinUpload = [
        ...seen,
        {
          hasPath: typeof request.pngPath === "string" && request.pngPath.length > 0,
          hasBase64: typeof request.pngBase64 === "string" && request.pngBase64.length > 0,
          base64Length: typeof request.pngBase64 === "string" ? request.pngBase64.length : 0,
          variant: request.variant,
        },
      ].slice(-20);
    }
    return call("ms_skin_upload", { ...request }, () => {
      const account = mockAccounts.find((item) => item.current);
      if (!account || account.type !== "microsoft") throw new Error("只有正版账号才能上传皮肤");
      // 自检用：gugu.mock.skinUpload = badSize 模拟后端报「皮肤尺寸不对」
      if (window.localStorage.getItem("gugu.mock.skinUpload") === "badSize") {
        throw new Error("皮肤尺寸不对：需要 64×32 或 64×64 的 PNG");
      }
      const base64 = (request.pngBase64 ?? "").trim();
      const path = (request.pngPath ?? "").trim();
      if (!base64 && !path) throw new Error("没有可上传的皮肤图片");
      // 两者都传时以 base64 为准（和后端一致）
      if (base64 && !base64.startsWith("iVBOR")) throw new Error("皮肤尺寸不对：需要 64×32 或 64×64 的 PNG");
      if (!base64 && !/\.png$/i.test(path)) throw new Error("皮肤尺寸不对：需要 64×32 或 64×64 的 PNG");
      return { ok: true, variant: request.variant, uploadedAt: new Date().toISOString(), capeSynced: true };
    });
  },

  skinUpload(pngBase64: string): Promise<SkinItem> {
    return call("skin_upload", { pngBase64 }, () => {
      const name = "skin-" + (mockSkins.length + 1) + "-" + Date.now().toString(36).slice(-4);
      const item: SkinItem = {
        name,
        path: mockRoot + "\\skins\\" + name + ".png",
        size: Math.max(1024, Math.floor((pngBase64.length * 3) / 4)),
        dataUrl: pngBase64.startsWith("data:") ? pngBase64 : "data:image/png;base64," + pngBase64,
      };
      mockSkins = [item, ...mockSkins];
      return { ...item };
    });
  },

  /** 当前正在使用的皮肤路径。TODO(M3-2): 后端 skin_current。 */
  skinCurrent(): Promise<string> {
    return call("skin_current", undefined, () => mockCurrentSkin);
  },

  /** 账号上的披风列表；supported=false 时 reason 说明为什么不支持，items 恒为空。 */
  capesList(): Promise<CapesListResult> {
    return call("capes_list", undefined, () => mockCapesList());
  },

  /** 装备披风；capeId=null 表示不使用披风。 */
  capeSetActive(request: { capeId: string | null }): Promise<CapeState> {
    return call("cape_set_active", request, () => {
      mockActiveCapeId = request.capeId;
      return { active: mockActiveCapeId };
    });
  },

  // ---------------------------------------------------------- 账号（M3-2）

  /** 全部账号（含 current 标记）。TODO(M3-2): 后端 accounts_list。 */
  accountsList(): Promise<Account[]> {
    return call("accounts_list", undefined, () => mockAccounts.map((account) => ({ ...account }))).then((list) =>
      list.map(normalizeAccount),
    );
  },

  /** 添加离线账号：名称 3–16 位，重名/重复由后端再校验一次。TODO(M3-2): 后端 account_add_offline。 */
  accountAddOffline(name: string): Promise<Account> {
    return call("account_add_offline", { name }, () => {
      const trimmed = name.trim();
      if (trimmed.length < 3 || trimmed.length > 16) throw new Error("离线账号名称需为 3–16 个字符");
      if (mockAccounts.some((account) => account.name.toLowerCase() === trimmed.toLowerCase())) {
        throw new Error("已经有同名账号了");
      }
      const created: Account = {
        id: "acc-off-" + Date.now().toString(36),
        name: trimmed,
        type: "offline",
        uuid: mockOfflineUuid(trimmed),
        skinDataUrl: mockSkinDataUrl(mockAccounts.length),
        current: false,
      };
      mockAccounts = [...mockAccounts, created];
      return { ...created };
    });
  },

  /** 切换当前账号。TODO(M3-2): 后端 account_apply。 */
  accountApply(id: string): Promise<void> {
    return call("account_apply", { id }, () => {
      mockApplyAccount(id);
    });
  },

  /** 删除账号（当前账号删掉后后端会挑一个顶上，没有就留空）。TODO(M3-2): 后端 account_delete。 */
  accountDelete(id: string): Promise<void> {
    return call("account_delete", { id }, () => {
      mockAccounts = mockAccounts.filter((account) => account.id !== id);
      if (mockAccounts.length > 0 && !mockAccounts.some((account) => account.current)) {
        mockApplyAccount(mockAccounts[0].id);
      }
    });
  },

  /**
   * 第三方（Yggdrasil 皮肤站）账号登录：成功后返回新账号，由界面再 account_apply 设为当前。
   * TODO(M15): 后端 account_add_thirdparty —— 走 authlib-injector，失败要回原文（401 / 不可达 / 无角色）。
   */
  accountAddThirdparty(request: { server: string; username: string; password: string }): Promise<Account> {
    return call("account_add_thirdparty", { ...request }, () => mockThirdpartyLogin(request));
  },

  /** 刷新第三方账号的登录态（重新换 accessToken，顺带取一张最新皮肤）。TODO(M15): 后端 account_refresh_thirdparty。 */
  accountRefreshThirdparty(request: { id: string }): Promise<Account> {
    return call("account_refresh_thirdparty", { ...request }, () => {
      const found = mockAccounts.find((account) => account.id === request.id);
      if (!found || found.type !== "thirdparty") throw new Error("这不是第三方账号，无法刷新登录");
      if (/down|timeout|unreachable/i.test(found.server ?? "")) throw new Error("服务器不可达（连接超时 8000ms）");
      return { ...found, skinDataUrl: mockSkinDataUrl(2) };
    });
  },

  /** authlib-injector 是否已就绪（第三方登录的前置）。TODO(M15): 后端 authlib_status。 */
  authlibStatus(): Promise<AuthlibStatus> {
    return call("authlib_status", undefined, () => mockAuthlibStatus());
  },

  /** 下载 authlib-injector（进度走下载中心）。TODO(M15): 后端 authlib_ensure。 */
  authlibEnsure(): Promise<{ taskId: string }> {
    return call("authlib_ensure", undefined, () => {
      startMockInstall("authlib-injector 1.2.7", mockRoot + "\\authlib-injector", "mod");
      window.localStorage.setItem(MOCK_AUTHLIB_KEY, "installed");
      return { taskId: mockTasks[0].id };
    });
  },

  /** 微软登录是否可用（看 settings.msClientId）。TODO(M5): 后端 ms_auth_status。 */
  msAuthStatus(): Promise<MsAuthStatus> {
    return call("ms_auth_status", undefined, () => mockMsAuthStatus());
  },

  /** 开始微软设备码登录；configured=false 表示还没在设置里填 Client ID。TODO(M3-2): 后端 ms_login_start。 */
  msLoginStart(): Promise<MsLoginStart> {
    return call("ms_login_start", undefined, () => mockMsStart());
  },

  /**
   * 微软账号当前正在使用的外观（皮肤 / 披风 / 模型）。
   * 「恢复正版皮肤」用它把账号上的皮肤取回本地。TODO(M14): 后端 ms_profile。
   */
  msProfile(request: { force?: boolean } = {}): Promise<MsProfile> {
    // 自检用：记下调用次数（界面不显示）
    if (typeof window !== "undefined") {
      const host = window as unknown as Record<string, unknown>;
      host.__guguMsProfile = (typeof host.__guguMsProfile === "number" ? host.__guguMsProfile : 0) + 1;
    }
    return call("ms_profile", { ...request }, () => mockMsProfile(request));
  },

  /**
   * 第三方（皮肤站）账号正在使用的皮肤，皮肤页只读预览用。
   * TODO(M15): 后端 thirdparty_skin。
   */
  thirdpartySkin(request: { accountId?: string } = {}): Promise<ThirdpartySkin> {
    return call("thirdparty_skin", { ...request }, () => {
      const account = mockAccounts.find((item) => (request.accountId ? item.id === request.accountId : item.current));
      if (!account || account.type !== "thirdparty") throw new Error("当前不是第三方账号");
      const result: ThirdpartySkin = {
        server: account.server ?? "https://littleskin.cn/api/yggdrasil",
        name: account.name,
        uuid: account.uuid,
        skinUrl: mockSkinDataUrl(2),
        previewDataUrl: mockSkinDataUrl(2),
        capeUrl: null,
        variant: "classic" as const,
      };
      // 自检用：记下这次返回的皮肤图，界面上的 3D 预览要和它一致（window.__guguTpSkin）
      if (typeof window !== "undefined") {
        (window as unknown as Record<string, unknown>).__guguTpSkin = {
          url: result.previewDataUrl,
          accountId: account.id,
          name: account.name,
        };
      }
      return result;
    });
  },

  /** 轮询微软登录结果。TODO(M3-2): 后端 ms_login_poll。 */
  msLoginPoll(deviceCode: string): Promise<MsLoginPoll> {
    return call("ms_login_poll", { deviceCode }, () => mockMsPoll(deviceCode));
  },

  // ------------------------------------------------------ 内容服务（M3-2）

  /** 内容搜索（分页 + 排序 + 分类）。TODO(M3-2): 后端 content_search。 */
  contentSearch(query: ContentSearchQuery): Promise<ContentSearchResult> {
    // 自检用：记下每次请求的参数（window.__guguContentReq），界面不显示
    if (typeof window !== "undefined") {
      const host = window as unknown as Record<string, unknown>;
      const seen = Array.isArray(host.__guguContentReq) ? (host.__guguContentReq as unknown[]) : [];
      host.__guguContentReq = [
        ...seen,
        {
          kind: query.kind,
          page: query.page,
          pageSize: query.pageSize,
          sort: query.sort,
          loader: query.loader ?? "",
          categories: query.categories ?? [],
          mcVersion: query.mcVersion ?? "",
        },
      ].slice(-60);
    }
    return call("content_search", { ...query }, () => {
      if (window.localStorage.getItem("gugu.mock.content") === "error") {
        throw new Error("内容服务暂时不可用（mock）");
      }
      // 自检用：gugu.mock.content = 403 模拟「CurseForge 没填 Key / 镜像不可用」
      if (window.localStorage.getItem("gugu.mock.content") === "403") {
        throw new Error("下载失败：内容接口返回 HTTP 403（Forbidden）");
      }
      // 模拟后端 kind_mapping 缺分支：MCIM 不认整合包
      if (query.provider === "mcim" && query.kind === "modpack") {
        throw new Error("不支持的内容类型：modpack");
      }
      const needle = query.query.trim().toLowerCase();
      let list = mockContentCatalog(query.provider, query.kind);
      if (needle.length > 0) {
        list = list.filter(
          (item) =>
            item.title.toLowerCase().includes(needle) ||
            (item.titleOriginal ?? "").toLowerCase().includes(needle) ||
            item.summary.toLowerCase().includes(needle),
        );
      }
      if (query.loader) list = list.filter((item) => item.loaders.includes(query.loader as string));
      if (query.categories && query.categories.length > 0) {
        list = list.filter((item) => query.categories!.some((cat) => item.categories.includes(cat)));
      }
      if (query.sort === "updated") {
        list = list.slice().sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
      } else if (query.sort !== "relevance") {
        list = list.slice().sort((a, b) => b.downloads - a.downloads);
      }
      const pageSize = Math.max(1, query.pageSize || 20);
      const page = Math.max(1, query.page || 1);
      const start = (page - 1) * pageSize;
      return {
        items: list.slice(start, start + pageSize),
        total: list.length,
        page,
        pageSize,
      };
    });
  },

  /** 项目详情。TODO(M3-2): 后端 content_detail。 */
  contentDetail(provider: ContentProvider, projectId: string): Promise<ProjectDetail> {
    return call("content_detail", { provider, projectId }, () => {
      const project = mockProjectFromId(provider, projectId);
      if (!project) throw new Error("项目不存在：" + projectId);
      return {
        projectId,
        provider,
        title: project.title,
        summary: project.summary,
        description: project.description,
        descriptionTranslated: null,
        iconUrl: mockIconDataUrl(project.title, project.color),
        downloads: project.downloads,
        author: project.author,
        url: "https://modrinth.com/project/" + project.slug,
        license: "MIT",
        updatedAt: secondsAgo(86_400 * 2),
      };
    });
  },

  /** 项目的版本列表（按 MC 版本 / 加载器过滤）。TODO(M3-2): 后端 content_versions。 */
  /** 前置依赖；后端未实现时返回空数组。TODO(M9): 后端 content_dependencies。 */
  contentDependencies(provider: ContentProvider, projectId: string): Promise<ContentDependency[]> {
    return call("content_dependencies", { provider, projectId }, () => {
      const slug = projectId.split(":").pop() ?? "";
      if (slug.indexOf("fabric-api") >= 0) return [];
      return [
        { projectId: provider + ":fabric-api", name: "Fabric API", kind: "required", provider },
        { projectId: provider + ":cloth-config", name: "Cloth Config", kind: "required", provider },
        { projectId: provider + ":modmenu", name: "Mod Menu", kind: "optional", provider },
      ];
    });
  },

  contentVersions(
    provider: ContentProvider,
    projectId: string,
    mcVersion?: string,
    loader?: string,
    channel?: VersionChannel,
  ): Promise<VersionItem[]> {
    return call("content_versions", { provider, projectId, mcVersion, loader, channel }, () => {
      const project = mockProjectFromId(provider, projectId);
      if (!project) return [];
      const base = project.gameVersions.filter((version) => !mcVersion || version === mcVersion);
      const games = base.length > 0 ? base : project.gameVersions;
      const rows: VersionItem[] = [];
      const channels: VersionChannel[] = ["release", "beta", "alpha"];
      for (let index = 0; index < 6; index += 1) {
        const game = games[index % games.length];
        const kind = channels[index % 3];
        rows.push({
          versionId: project.slug + "-" + game + "-" + channel + "-" + (index + 1),
          name: project.title + " " + (index + 1),
          gameVersions: [game],
          loaders: project.loaders.filter((item) => !loader || item === loader || item === "Minecraft"),
          downloads: Math.floor(project.downloads / (index + 2)),
          fileSize: 1_048_576 * (index + 1),
          publishedAt: secondsAgo(86_400 * (index + 1) * 7),
          dependencies: [],
          channel: kind,
          serverLabel: "Server, " + game,
        });
      }
      return channel ? rows.filter((row) => (row.channel ?? "release") === channel) : rows;
    });
  },

  /** 安装到实例：立刻返回 taskId，进度进下载中心。TODO(M3-2): 后端 mod_install。 */
  modInstall(request: {
    provider: ContentProvider;
    projectId: string;
    versionId?: string;
    instanceId: number;
    withDependencies?: boolean;
  }): Promise<{ taskId: string }> {
    return call("mod_install", { ...request }, () => {
      const project = mockProjectFromId(request.provider, request.projectId);
      const instance = mockInstances.find((item) => item.id === request.instanceId);
      if (!instance) throw new Error("实例不存在：" + request.instanceId);
      const title = (project?.title ?? request.projectId) + " → " + instance.name;
      const kind: ContentFileKind = project?.kind === "shader" ? "shader" : project?.kind === "resourcepack" ? "resourcepack" : "mod";
      startMockInstall(title, mockRoot + "\\instances\\" + instance.id, "mod");
      if (project) {
        const list = mockInstanceFiles(instance.id, kind);
        const file = project.slug + "-" + (request.versionId ?? "latest").slice(-12) + (kind === "mod" ? ".jar" : ".zip");
        mockStoreFiles(instance.id, kind, [
          ...list,
          mockFileItem(file, 1_048_576, project.gameVersions[0], project.loaders[0] ?? null, MOCK_PROVIDER_LABEL[request.provider]),
        ]);
      }
      return { taskId: mockTasks[0].id };
    });
  },

  /** 批量翻译（译文优先展示）。limited=true 表示内置接口命中频率上限。TODO(M3-2): 后端 translate_texts。 */
  translateTexts(texts: string[]): Promise<TranslateResult> {
    return call("translate_texts", { texts }, () => mockTranslate(texts));
  },

  // ------------------------------------------------------ 背景外观（M4）

  /** 一页 20 张的背景列表（本地图 / 壁纸引擎），顺带返回该类背景所在目录。TODO(M15): 后端 background_list 会带 dir。 */
  backgroundList(kind: "local" | "wallpaper", page: number, pageSize = 20): Promise<BackgroundPage> {
    // 自检用：记下每次请求的 kind，确认只会出现 local / wallpaper 两种（window.__guguBgKinds）
    if (typeof window !== "undefined") {
      const host = window as unknown as Record<string, unknown>;
      const seen = Array.isArray(host.__guguBgKinds) ? (host.__guguBgKinds as string[]) : [];
      host.__guguBgKinds = [...seen, kind];
    }
    return cached("background_list:" + kind + ":" + page + ":" + pageSize, () =>
      call("background_list", { kind, page, pageSize }, () => mockBackgroundPage(kind, page, pageSize)),
    );
  },

  /** 应用某张背景（方案 B / D）。TODO(M4): 后端 background_apply。 */
  backgroundApply(request: { kind: "local" | "wallpaper"; file: string }): Promise<void> {
    // 自检用：记下这次调用的键名（window.__guguBgApply），确认是 { kind, file }
    if (typeof window !== "undefined") {
      const host = window as unknown as Record<string, unknown>;
      const seen = Array.isArray(host.__guguBgApply) ? (host.__guguBgApply as unknown[]) : [];
      host.__guguBgApply = [...seen, { kind: request.kind, file: request.file, keys: Object.keys(request) }].slice(-20);
    }
    return call("background_apply", { ...request }, () => {
      // 真机曾报「missing required key file」：这里必须是 { kind, file } 两个键
      if (!request.file) throw new Error("没有选中要应用的背景文件");
      /* mock 只记设置，背景层由前端按 settings.appearance 渲染 */
    });
  },

  /** 壁纸引擎默认目录（创意工坊 431960）。TODO(M4): 后端 wallpaper_dir_default。 */
  wallpaperDirDefault(): Promise<string> {
    return cached("wallpaper_dir_default", () => call("wallpaper_dir_default", undefined, () => MOCK_WALLPAPER_DIR));
  },

  // ------------------------------------------------------ 陶瓦联机（M4）

  /** 陶瓦联机是否已安装。TODO(M4): 后端 tera_status。 */
  teraStatus(): Promise<TeraStatus> {
    return call("tera_status", undefined, () => ({
      installed: MOCK_TERA.installed,
      version: MOCK_TERA.version,
      source: "bundled",
      license: "AGPL-3.0",
      homepage: "https://github.com/burningtnt/Terracotta",
    }));
  },

  /** 安装陶瓦联机：立刻返回 taskId，进度走下载中心。TODO(M4): 后端 tera_install。 */
  teraInstall(): Promise<{ taskId: string }> {
    return call("tera_install", undefined, () => {
      startMockInstall("陶瓦联机 " + MOCK_TERA.version, mockRoot + "\\tools\\tera", "install");
      if (typeof window !== "undefined") {
        window.setTimeout(() => {
          MOCK_TERA.installed = true;
          mockTeraLog("[install] 陶瓦联机 " + MOCK_TERA.version + " 安装完成");
        }, 1600);
      }
      return { taskId: mockTasks[0].id };
    });
  },

  /** 开一个房间。TODO(M4): 后端 tera_start_host。 */
  teraStartHost(options: { roomName?: string; port?: number }): Promise<TeraState> {
    return call("tera_start_host", { ...options }, () => {
      if (!MOCK_TERA.installed) throw new Error("还没有安装陶瓦联机");
      MOCK_TERA.hosting = true;
      MOCK_TERA.roomCode = "TERA-8F2K";
      MOCK_TERA.players = ["你"];
      mockTeraLog("[host] 房间已创建：" + (options.roomName?.trim() || "未命名房间") + " · 端口 " + (options.port ?? 23000 + Math.floor(Math.random() * 2000)));
      mockTeraLog("[host] 联机码 " + MOCK_TERA.roomCode + "（发给朋友就能加入）");
      mockTeraStartTicker();
      return mockTeraState();
    });
  },

  /** 用联机码加入房间。TODO(M4): 后端 tera_join。 */
  teraJoin(code: string): Promise<TeraState> {
    return call("tera_join", { code }, () => {
      if (!MOCK_TERA.installed) throw new Error("还没有安装陶瓦联机");
      const trimmed = code.trim().toUpperCase();
      if (!/^[A-Z0-9-]{6,12}$/.test(trimmed)) throw new Error("联机码格式不对，应为 6–12 位字母或数字");
      if (trimmed.startsWith("0000")) throw new Error("房间不存在或主机已关闭");
      MOCK_TERA.hosting = false;
      MOCK_TERA.roomCode = trimmed;
      MOCK_TERA.players = ["你", "主机玩家"];
      mockTeraLog("[join] 正在连接 " + trimmed + " …");
      mockTeraLog("[join] 已加入房间，RTT 42 ms");
      return mockTeraState();
    });
  },

  /** 结束联机（主机关闭 / 加入方退出）。TODO(M4): 后端 tera_stop。 */
  teraStop(): Promise<void> {
    return call("tera_stop", undefined, () => {
      MOCK_TERA.hosting = false;
      MOCK_TERA.roomCode = null;
      MOCK_TERA.players = [];
      mockTeraStopTicker();
      mockTeraLog("[room] 已结束联机");
      mockTeraState();
    });
  },

  // ------------------------------------------------ 皮肤站查皮肤（M5）

  /** 官网最新新闻（首条在主页展示，可翻页）。TODO(M5): 后端 news_list。 */
  newsList(request: { limit: number }): Promise<NewsItem[]> {
    return call("news_list", { ...request }, () => {
      if (window.localStorage.getItem(MOCK_NEWS_OVERRIDE_KEY) === "error") {
        throw new Error("新闻接口暂时不可用");
      }
      return MOCK_NEWS.slice(0, Math.max(1, Math.min(request.limit, MOCK_NEWS.length)));
    });
  },

  /** 从皮肤站（Yggdrasil）查某个玩家的皮肤，只用于预览。TODO(M5): 后端 skin_lookup_thirdparty。 */
  skinLookupThirdparty(request: { server: string; name: string }): Promise<SkinLookupResult> {
    return call("skin_lookup_thirdparty", { ...request }, () => mockSkinLookup(request.server, request.name));
  },

  /** 把一张远程皮肤的图片存进 .mc\skins\。TODO(M5): 后端 skin_import_from_url。 */
  skinImportFromUrl(request: { url: string; name?: string }): Promise<SkinItem> {
    return call("skin_import_from_url", { ...request }, () => {
      const raw = request.name?.trim() || "thirdparty-" + Date.now().toString(36).slice(-4);
      const name = raw.replace(/\.png$/i, "");
      // 同名（同一张）已经在本地：不重复落盘，直接复用
      const existing = mockSkins.find((skin) => skin.name === name);
      if (existing) return { ...existing, reused: true };
      const item: SkinItem = {
        name: name,
        path: mockRoot + "\\skins\\" + name + ".png",
        size: 3072,
        dataUrl: request.url,
      };
      mockSkins = [item, ...mockSkins.filter((skin) => skin.name !== name)];
      return { ...item };
    });
  },


  // ------------------------------------------------ 服务端（M6 多人 → 服务器）

  /** 服务端核心列表。TODO(M6): 后端 server_cores。 */
  serverCores(request: { mcVersion?: string } = {}): Promise<ServerCore[]> {
    return cached("server_cores:" + (request.mcVersion ?? ""), () =>
      call("server_cores", { ...request }, () => MOCK_SERVER_CORES.map((core) => ({ ...core }))),
    );
  },

  /** 某个核心可用的游戏版本。TODO(M6): 后端 server_versions。 */
  serverVersions(request: { core: string; mcVersion?: string }): Promise<ServerVersion[]> {
    return call("server_versions", { ...request }, () => {
      const list = MOCK_SERVER_VERSIONS[request.core] ?? [];
      return list.filter((item) => !request.mcVersion || item.version === request.mcVersion);
    });
  },

  /** 创建服务器：立刻返回 id + taskId，下载进度走下载中心。TODO(M6): 后端 server_create。 */
  serverCreate(request: ServerCreateRequest): Promise<{ id: string; taskId: string }> {
    return call("server_create", { ...request }, () => {
      if (!request.eula) throw new Error("需要先同意 Minecraft EULA");
      if (mockServers.some((server) => server.port === request.port)) {
        throw new Error("端口 " + request.port + " 已被其它服务器占用");
      }
      const id = "srv-" + Date.now().toString(36);
      const created: ServerInstance = {
        id,
        name: request.name.trim() || "服务器 " + (mockServers.length + 1),
        core: request.core,
        mcVersion: request.mcVersion,
        port: request.port,
        memoryMb: request.memoryMb,
        state: "starting",
        players: 0,
        dir: mockRoot + "\\servers\\" + id,
      };
      mockServers = [...mockServers, created];
      mockServerLogs.set(id, [
        "[Server thread/INFO]: Downloading " + request.core + " " + request.mcVersion,
        "[Server thread/INFO]: Preparing " + request.memoryMb + " MB heap on port " + request.port,
      ]);
      startMockInstall(request.core + " " + request.mcVersion + " 服务端", created.dir, "install");
      if (typeof window !== "undefined") {
        window.setTimeout(() => {
          const current = mockServers.find((item) => item.id === id);
          if (current && current.state === "starting") mockServerState(id, "stopped", 0);
        }, 4000);
      }
      return { id, taskId: mockTasks[0].id };
    });
  },

  /** 服务器列表。TODO(M6): 后端 server_list。 */
  serverList(): Promise<ServerInstance[]> {
    return call("server_list", undefined, () => mockServers.map((server) => ({ ...server })));
  },

  /** 启动服务器：状态经 server://state 推。TODO(M6): 后端 server_start。 */
  serverStart(id: string): Promise<void> {
    return call("server_start", { id }, () => {
      mockServerState(id, "starting", 0);
      mockServerLog(id, "[Server thread/INFO]: Starting minecraft server version " + (mockServers.find((s) => s.id === id)?.mcVersion ?? ""));
      if (typeof window !== "undefined") {
        window.setTimeout(() => {
          mockServerState(id, "running", 1);
          mockServerLog(id, "[Server thread/INFO]: Done (2.341s)! For help, type \"help\"");
          mockServerLog(id, "[Server thread/INFO]: 你 joined the game");
          mockServerTicker(id);
        }, 900);
      }
    });
  },

  /** 停止服务器。TODO(M6): 后端 server_stop。 */
  serverStop(id: string): Promise<void> {
    return call("server_stop", { id }, () => {
      mockServerState(id, "stopping", 0);
      if (typeof window !== "undefined") {
        window.setTimeout(() => {
          mockServerStopTicker(id);
          mockServerState(id, "stopped", 0);
          mockServerLog(id, "[Server thread/INFO]: ThreadedAnvilChunkStorage closed");
        }, 700);
      }
    });
  },

  /** 删除服务器（含目录）。TODO(M6): 后端 server_delete。 */
  serverDelete(id: string): Promise<void> {
    return call("server_delete", { id }, () => {
      mockServerStopTicker(id);
      mockServers = mockServers.filter((server) => server.id !== id);
      mockServerLogs.delete(id);
    });
  },

  /** 打开服务器目录。TODO(M6): 后端 server_open_folder。 */
  serverOpenFolder(id: string): Promise<void> {
    return call("server_open_folder", { id }, () => {
      /* 浏览器里没有资源管理器可开 */
    });
  },

  /** 读 server.properties / 内存与端口设置。TODO(M6): 后端 server_read_properties。 */
  serverReadProperties(id: string): Promise<Record<string, string>> {
    return call("server_read_properties", { id }, () => {
      const server = mockServers.find((item) => item.id === id);
      return {
        "server-port": String(server?.port ?? 25565),
        "max-players": "20",
        "level-name": "world",
        "view-distance": "10",
        "online-mode": "true",
      };
    });
  },

  /** 写回 server.properties 的若干项。TODO(M6): 后端 server_write_properties。 */
  serverWriteProperties(id: string, patch: Record<string, string>): Promise<Record<string, string>> {
    return call("server_write_properties", { id, patch }, () => {
      if (patch["server-port"]) {
        mockServers = mockServers.map((server) =>
          server.id === id ? { ...server, port: Number(patch["server-port"]) || server.port } : server,
        );
      }
      return patch;
    });
  },

  // -------------------------------- 服务端详情：内容 / 存档 / 配置 / 日志（M10）

  /** 服务端里的模组或插件。TODO(M10): 后端 server_content_list。 */
  serverContentList(request: { serverId: string; kind: "mod" | "plugin" }): Promise<ServerContentItem[]> {
    return call("server_content_list", { ...request }, () =>
      mockServerContentList(request.serverId, request.kind).map((item) => ({ ...item })),
    );
  },

  /** 把一个资源装进服务端（插件走这里）。TODO(M10): 后端 server_content_install。 */
  serverContentInstall(request: {
    serverId: string;
    provider: ContentProvider;
    projectId: string;
    versionId?: string;
  }): Promise<{ taskId: string }> {
    return call("server_content_install", { ...request }, () => {
      const list = mockServerContentList(request.serverId, "plugin");
      const fileName = request.projectId + "-latest.jar";
      if (!list.some((item) => item.fileName === fileName)) {
        mockServerContent.set(request.serverId + ":plugin", [
          { fileName, sizeBytes: 1_048_576, enabled: true, provider: request.provider, projectId: request.projectId },
          ...list,
        ]);
      }
      startMockInstall(request.projectId + " 插件", "servers\\" + request.serverId + "\\plugins", "install");
      return { taskId: mockTasks[0]?.id ?? "mock-task" };
    });
  },

  /** 启用 / 禁用服务端里的一个 jar。TODO(M10): 后端 server_content_toggle。 */
  serverContentToggle(request: { serverId: string; kind: "mod" | "plugin"; fileName: string; enabled: boolean }): Promise<void> {
    return call("server_content_toggle", { ...request }, () => {
      const list = mockServerContentList(request.serverId, request.kind).map((item) =>
        item.fileName === request.fileName ? { ...item, enabled: request.enabled } : item,
      );
      mockServerContent.set(request.serverId + ":" + request.kind, list);
    });
  },

  /** 服务端世界列表。TODO(M10): 后端 server_saves_list。 */
  serverSavesList(request: { serverId: string }): Promise<ServerSaveItem[]> {
    return call("server_saves_list", { ...request }, () => mockServerSavesOf(request.serverId).map((item) => ({ ...item })));
  },

  /** 把世界目录备份成 zip。TODO(M10): 后端 server_saves_backup（契约外，缺这条后端会回错误）。 */
  serverSavesBackup(request: { serverId: string; name: string }): Promise<{ path: string }> {
    return call("server_saves_backup", { ...request }, () => ({
      path: mockRoot + "\\servers\\" + request.serverId + "\\backups\\" + request.name + "-" + Date.now().toString(36) + ".zip",
    }));
  },

  /** 读 server.properties：原文 + 常用项。TODO(M10): 后端 server_config_read。 */
  serverConfigRead(request: { serverId: string }): Promise<ServerConfigResult> {
    return call("server_config_read", { ...request }, () => {
      const value = mockServerConfigOf(request.serverId);
      return { text: value.text, fields: { ...value.fields } };
    });
  },

  /** 写回 server.properties：只改 patch 里给的键，未识别的行与注释原样保留。TODO(M10): 后端 server_config_write。 */
  serverConfigWrite(request: { serverId: string; patch: Record<string, string> }): Promise<void> {
    return call("server_config_write", { ...request }, () => {
      const current = mockServerConfigOf(request.serverId);
      const fields = { ...current.fields, ...request.patch };
      const text = current.text
        .split("\n")
        .map((line) => {
          const eq = line.indexOf("=");
          if (eq <= 0) return line;
          const key = line.slice(0, eq).trim();
          return Object.prototype.hasOwnProperty.call(request.patch, key) ? key + "=" + request.patch[key] : line;
        })
        .join("\n");
      mockServerConfig.set(request.serverId, { text, fields });
      if (request.patch["server-port"]) {
        const port = Number(request.patch["server-port"]);
        if (Number.isFinite(port)) {
          mockServers = mockServers.map((server) => (server.id === request.serverId ? { ...server, port } : server));
        }
      }
    });
  },

  /** 向服务端控制台发一条指令（服务端没运行时后端会拒）。TODO(M12): 后端 server_send_command。 */
  serverSendCommand(request: { serverId: string; command: string }): Promise<void> {
    return call("server_send_command", { ...request }, () => {
      const server = mockServers.find((item) => item.id === request.serverId);
      if (!server || server.state !== "running") throw new Error("服务端未运行，先点启动");
      const command = request.command.trim().replace(/^\//, "");
      if (!command) throw new Error("指令不能为空");
      // 回显行由界面自己补（带高亮），这里只模拟后端执行结果
      if (/^list\b/.test(command)) {
        mockServerLog(request.serverId, "[Server thread/INFO]: There are " + (server.players ?? 0) + " of a max of 20 players online");
      } else if (/^tps\b/.test(command)) {
        mockServerLog(request.serverId, "[Server thread/INFO]: TPS from last 1m, 5m, 15m: 20.0, 20.0, 20.0");
      } else if (/^save-all\b/.test(command)) {
        mockServerLog(request.serverId, "[Server thread/INFO]: Saved the game");
      } else if (/^stop\b/.test(command)) {
        mockServerLog(request.serverId, "[Server thread/INFO]: Stopping the server");
      } else {
        mockServerLog(request.serverId, "[Server thread/INFO]: " + command + " 已执行");
      }
    });
  },

  /** 控制台状态：running / stdinOpen。TODO(M12): 后端 server_console_state。 */
  serverConsoleState(request: { serverId: string }): Promise<ServerConsoleState> {
    return call("server_console_state", { ...request }, () => {
      const server = mockServers.find((item) => item.id === request.serverId);
      const running = server?.state === "running";
      return { running, stdinOpen: running };
    });
  },

  /** 服务端日志尾部。TODO(M10): 后端 server_logs_tail。 */
  serverLogsTail(request: { serverId: string; lines: number }): Promise<string[]> {
    return call("server_logs_tail", { ...request }, () =>
      (mockServerLogs.get(request.serverId) ?? []).slice(-Math.max(1, request.lines)),
    );
  },

  // ------------------------------------------------------ 外部链接（M4）

  /** 用系统浏览器打开外链。TODO(M4): 后端 open_external（或换成 tauri-opener 插件）。 */
  openExternal(url: string): Promise<void> {
    return call("open_external", { url }, () => {
      // 自检用：浏览器预览里 window.open 可能被拦，这里记下真实调用（window.__guguOpened）
      if (typeof window !== "undefined") {
        const host = window as unknown as Record<string, unknown>;
        const seen = Array.isArray(host.__guguOpened) ? (host.__guguOpened as string[]) : [];
        host.__guguOpened = [...seen, url].slice(-40);
      }
      window.open(url, "_blank", "noopener,noreferrer");
    });
  },
};

export type TauriApi = typeof api;

/**
 * 订阅启动器事件：install://progress、install://error、game://log、game://exit。
 * 浏览器预览下由 mock ticker 推同形状事件，界面代码不需要区分运行时。
 */
export async function subscribeLauncherEvents(handler: (event: LauncherEvent) => void): Promise<() => void> {
  if (!isTauri()) {
    mockHandlers.add(handler);
    ensureMockTicker();
    return () => {
      mockHandlers.delete(handler);
    };
  }
  const { listen } = await import("@tauri-apps/api/event");
  const unlisteners = await Promise.all([
    listen<InstallProgress>("install://progress", (event) => handler({ kind: "install-progress", payload: event.payload })),
    listen<{ taskId: string; message: string }>("install://error", (event) =>
      handler({ kind: "install-error", payload: event.payload }),
    ),
    // 单实例：已经有窗口在跑了，后端把新实例拦下并通知这里
    listen<{ message?: string }>("app://second-instance", (event) =>
      handler({ kind: "second-instance", payload: event.payload ?? {} }),
    ),
    listen<{ line: string }>("tera://log", (event) => handler({ kind: "tera-log", payload: event.payload })),
    listen<TeraState>("tera://state", (event) => handler({ kind: "tera-state", payload: event.payload })),
    listen<{ id: string; line: string }>("server://log", (event) => handler({ kind: "server-log", payload: event.payload })),
    listen<{ id: string; state: ServerState; players?: number }>("server://state", (event) =>
      handler({ kind: "server-state", payload: event.payload }),
    ),
    listen<{ instanceId: number; line: string }>("game://log", (event) => handler({ kind: "game-log", payload: event.payload })),
    listen<{ instanceId: number; code: number }>("game://exit", (event) => handler({ kind: "game-exit", payload: event.payload })),
  ]);
  return () => {
    unlisteners.forEach((off) => off());
  };
}

/** 把实例目录下的 icon.png 变成浏览器可显示的 URL；空路径或浏览器预览返回空字符串。 */
export function iconFileUrl(path: string): string {
  if (!path || !isTauri()) return "";
  try {
    return convertFileSrc(path);
  } catch {
    return "";
  }
}
