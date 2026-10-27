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
  BackgroundItem,
  BackgroundPage,
  CapeItem,
  ContentFileKind,
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
  DataDirInfo,
  LauncherEvent,
  LogFile,
  ModFileItem,
  MsAuthStatus,
  MsLoginPoll,
  NewsItem,
  MsLoginStart,
  PathsLayout,
  ProjectDetail,
  SaveItem,
  Settings,
  SayingResult,
  ServerCore,
  ServerCreateRequest,
  ServerInstance,
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
  updateMode: "auto",
  downloadSource: "auto",
  concurrentDownloads: 8,
  perFileConnections: 4,
  customMirror: "",
  resourceSource: "modrinth",
  translationSource: "builtin",
  translationBaseUrl: "",
  translationApiKey: "",
  translationModel: "",
  translationTemperature: 0,
  translationMaxTokens: 0,
  curseforgeApiKey: "",
  msClientId: "",
  thirdpartyAuthServer: "",
  closeAction: "minimize",
  autoDownloadJava: true,
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
      if (base.includes("example.com") || base.includes("invalid")) {
        resolve({ id: target.id, ok: false, latencyMs: null, message: "镜像不可用" });
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
  { id: "auto", name: "自动（测速最快）", description: "按下面几个源的实际延迟挑最快的" },
  { id: "bellsoft", name: "BellSoft Liberica", description: "覆盖面广，含 JRE 与全量 JDK" },
  { id: "adoptium", name: "Adoptium (Temurin)", description: "社区维护的 OpenJDK 构建" },
  { id: "tuna", name: "清华 TUNA 镜像", description: "国内镜像，下载稳定" },
  { id: "zulu", name: "Azul Zulu", description: "Azul 提供的 OpenJDK 构建" },
  { id: "oracle", name: "Oracle JDK", description: "官网下载，需要接受 Oracle 许可", requiresLicense: true },
];

const MOCK_JAVA_VERSIONS: JavaVersion[] = [
  { source: "bellsoft", version: "25.0.2", major: 25, lts: true, sizeBytes: 214_958_080, url: "https://bell-sw.com/pages/downloads/" },
  { source: "adoptium", version: "25.0.2+10", major: 25, lts: true, sizeBytes: 208_666_624, url: "https://adoptium.net/" },
  { source: "tuna", version: "21.0.10+7", major: 21, lts: true, sizeBytes: 190_840_832, url: "https://mirrors.tuna.tsinghua.edu.cn/Adoptium/" },
  { source: "zulu", version: "21.0.10", major: 21, lts: true, sizeBytes: 199_229_440, url: "https://www.azul.com/downloads/" },
  { source: "adoptium", version: "17.0.18+8", major: 17, lts: true, sizeBytes: 181_403_648, url: "https://adoptium.net/" },
  { source: "bellsoft", version: "17.0.18", major: 17, lts: true, sizeBytes: 185_597_952, url: "https://bell-sw.com/pages/downloads/" },
  { source: "oracle", version: "17.0.18", major: 17, lts: true, sizeBytes: 190_000_000, url: "https://www.oracle.com/java/technologies/downloads/" },
  { source: "adoptium", version: "8.0.482+8", major: 8, lts: true, sizeBytes: 109_051_904, url: "https://adoptium.net/" },
  { source: "bellsoft", version: "8.0.482", major: 8, lts: true, sizeBytes: 106_954_752, url: "https://bell-sw.com/pages/downloads/" },
  { source: "zulu", version: "26.0.0-ea", major: 26, lts: false, sizeBytes: 205_520_896, url: "https://www.azul.com/downloads/" },
];

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

/** 画一张 64×32 的假披风。 */
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

const mockCapes: CapeItem[] = [
  { name: "Migrator", path: mockRoot + "\\capes\\Migrator.png", size: 1280, dataUrl: mockCapeDataUrl(0) },
  { name: "Mojang", path: mockRoot + "\\capes\\Mojang.png", size: 1156, dataUrl: mockCapeDataUrl(2) },
];

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
let mockCurrentCape = mockCapes[0].name;

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

// ---- 微软登录（设备码流程的假状态）----

const MOCK_MS_STATE = { deviceCode: "", userCode: "", polls: 0 };

/** Client ID 只接受 8-4-4-4-12 的 UUID（大小写都行）。 */
const MS_CLIENT_ID_PATTERN = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

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
  const slug = projectId.includes(":") ? projectId.split(":").pop() ?? "" : projectId;
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
      title: base.title + suffix + "（" + (kind === "mod" ? "模组" : kind === "modpack" ? "整合包" : kind === "shader" ? "光影" : kind === "resourcepack" ? "资源包" : "数据包") + "）",
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

/** 本地图 26 张、壁纸引擎 45 张：为了能看到 20 张一页的分页效果。 */
const MOCK_BACKGROUNDS: Record<"local" | "wallpaper", BackgroundItem[]> = {
  local: Array.from({ length: 26 }, (_, index) => {
    const file = "local-" + String(index + 1).padStart(2, "0") + (index % 5 === 4 ? ".mp4" : ".png");
    const url = mockBackgroundDataUrl(index);
    return {
      file,
      path: mockRoot + "\\backgrounds\\" + file,
      url,
      previewUrl: url,
      isVideo: file.endsWith(".mp4"),
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
      url: isVideo ? "" : url,
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
  return { items: list.slice(start, start + size), total: list.length, page: Math.max(0, page), pageSize: size };
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

let mockServers: ServerInstance[] = [];
const mockServerLogs = new Map<string, string[]>();
const mockServerTickers = new Map<string, number>();

function mockServerLog(id: string, line: string): void {
  const lines = [...(mockServerLogs.get(id) ?? []), line].slice(-500);
  mockServerLogs.set(id, lines);
  emitMock({ kind: "server-log", payload: { id, line } });
}

function mockServerState(id: string, state: ServerState, players?: number): void {
  mockServers = mockServers.map((server) => (server.id === id ? { ...server, state, players: players ?? server.players ?? 0 } : server));
  emitMock({ kind: "server-state", payload: { id, state, players } });
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

  /** 可下载的 Java 版本。TODO(M5): 后端 java_versions。 */
  javaVersions(request: { source?: string; major?: number }): Promise<JavaVersion[]> {
    return call("java_versions", { ...request }, () =>
      MOCK_JAVA_VERSIONS.filter((item) => {
        if (request.major && item.major !== request.major) return false;
        if (request.source && request.source !== "auto" && item.source !== request.source) return false;
        return true;
      }),
    );
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

  /** 把某个皮肤设为当前皮肤。TODO(M3-2): 后端 skin_apply。 */
  skinApply(path: string): Promise<void> {
    return call("skin_apply", { path }, () => {
      mockCurrentSkin = path;
    });
  },

  /** 上传一张 PNG（base64），落到 .mc\\skins\\ 并返回新条目。TODO(M3-2): 后端 skin_upload。 */
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

  /** 本地披风列表。TODO(M3-2): 后端 capes_list。 */
  capesList(): Promise<CapeItem[]> {
    return call("capes_list", undefined, () => mockCapes.map((cape) => ({ ...cape })));
  },

  /** 应用/取消披风（传空字符串＝取消）。TODO(M3-2): 后端 cape_apply。 */
  capeApply(name: string): Promise<void> {
    return call("cape_apply", { name }, () => {
      mockCurrentCape = name;
    });
  },

  /** 当前披风名，空字符串＝没有。TODO(M3-2): 后端 cape_current。 */
  capeCurrent(): Promise<string> {
    return call("cape_current", undefined, () => mockCurrentCape);
  },

  // ---------------------------------------------------------- 账号（M3-2）

  /** 全部账号（含 current 标记）。TODO(M3-2): 后端 accounts_list。 */
  accountsList(): Promise<Account[]> {
    return call("accounts_list", undefined, () => mockAccounts.map((account) => ({ ...account })));
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

  /** 微软登录是否可用（看 settings.msClientId）。TODO(M5): 后端 ms_auth_status。 */
  msAuthStatus(): Promise<MsAuthStatus> {
    return call("ms_auth_status", undefined, () => mockMsAuthStatus());
  },

  /** 开始微软设备码登录；configured=false 表示还没在设置里填 Client ID。TODO(M3-2): 后端 ms_login_start。 */
  msLoginStart(): Promise<MsLoginStart> {
    return call("ms_login_start", undefined, () => mockMsStart());
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
        },
      ].slice(-60);
    }
    return call("content_search", { ...query }, () => {
      if (window.localStorage.getItem("gugu.mock.content") === "error") {
        throw new Error("内容服务暂时不可用（mock）");
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
  contentVersions(
    provider: ContentProvider,
    projectId: string,
    mcVersion?: string,
    loader?: string,
  ): Promise<VersionItem[]> {
    return call("content_versions", { provider, projectId, mcVersion, loader }, () => {
      const project = mockProjectFromId(provider, projectId);
      if (!project) return [];
      const base = project.gameVersions.filter((version) => !mcVersion || version === mcVersion);
      const games = base.length > 0 ? base : project.gameVersions;
      return games.slice(0, 3).map((game, index) => ({
        versionId: project.slug + "-" + game + "-" + (index + 1),
        name: project.title + " " + game + (index === 0 ? " (最新)" : ""),
        gameVersions: [game],
        loaders: project.loaders.filter((item) => !loader || item === loader || item === "Minecraft"),
        downloads: Math.floor(project.downloads / (index + 2)),
        fileSize: 1_048_576 * (index + 1),
        publishedAt: secondsAgo(86_400 * (index + 1) * 7),
        dependencies:
          index === 0 && project.slug !== "fabric-api"
            ? [{ projectId: provider + ":fabric-api", name: "Fabric API", kind: "required" }]
            : [],
      }));
    });
  },

  /** 安装到实例：立刻返回 taskId，进度进下载中心。TODO(M3-2): 后端 mod_install。 */
  modInstall(request: {
    provider: ContentProvider;
    projectId: string;
    versionId?: string;
    instanceId: number;
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

  /** 一页 20 张的背景列表（本地图 / 壁纸引擎）。TODO(M4): 后端 background_list。 */
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
  backgroundApply(kind: "local" | "wallpaper", file: string): Promise<void> {
    return call("background_apply", { kind, file }, () => {
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
      version: MOCK_TERA.installed ? MOCK_TERA.version : null,
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

  // ------------------------------------------------------ 外部链接（M4）

  /** 用系统浏览器打开外链。TODO(M4): 后端 open_external（或换成 tauri-opener 插件）。 */
  openExternal(url: string): Promise<void> {
    return call("open_external", { url }, () => {
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
