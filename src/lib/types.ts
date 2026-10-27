/** 与 Rust 侧 serde 序列化保持一致的共享类型（M1：只覆盖已暴露的 command）。 */

/** 界面只提供简体中文；保留类型别名，以后加语言只改这里。 */
export type Lang = "zh-CN";
export type ThemeMode = "light" | "dark";
export type UpdateMode = "auto" | "manual" | "off";
export type DownloadSourceId = "official" | "bmclapi" | "custom" | "sjtu" | "auto";
/** 资源/模组来源：Modrinth / CurseForge / MCIM 镜像（auto、official 为历史值，UI 不再提供）。 */
export type ResourceSourceId = "auto" | "official" | "modrinth" | "curseforge" | "mcim";
/** 资源描述翻译来源：内置接口（默认，有频率限制）或自定义 OpenAI 兼容接口。 */
export type TranslationSource = "builtin" | "custom";
/** GitHub 加速节点：auto 或已测通的固定节点 id。 */
export type GithubAccelerator = "auto" | "ghf" | "ghfast" | "cnxiaobai" | "tangbai" | "chenc" | "xyz";

export interface Settings {
  language: Lang;
  theme: ThemeMode;
  accent: string;
  /** 自定义主题色（十六进制）；未自定义时不写，界面回落到 accent */
  themeColor?: string;
  updateMode: UpdateMode;
  downloadSource: DownloadSourceId;
  /** 同时下载文件数（1–32，默认 8） */
  concurrentDownloads: number;
  /** 单文件分片线程数（1–16，默认 4） */
  perFileConnections: number;
  /** 自定义镜像根地址，仅 downloadSource === "custom" 时生效 */
  customMirror: string;
  resourceSource: ResourceSourceId;
  /** 自动翻译开关：关掉后资源卡片显示原文（titleOriginal / summaryOriginal） */
  translateEnabled: boolean;
  /** 资源描述翻译来源 */
  translationSource: TranslationSource;
  /** 自定义翻译接口根地址（OpenAI 兼容，自动拼 /chat/completions） */
  translationBaseUrl: string;
  /** 自定义翻译接口的 API Key：只存本机 settings.json */
  translationApiKey: string;
  translationModel: string;
  /** 0 表示沿用默认 0.2 */
  translationTemperature: number;
  /** 0 表示沿用默认 512 */
  translationMaxTokens: number;
  /** CurseForge API Key；留空时走 MCIM 兼容端点 */
  curseforgeApiKey: string;
  /** 微软登录的 OAuth Client ID（在「设置 → 内容服务」里填写，8-4-4-4-12 UUID） */
  msClientId: string;
  /** 皮肤站地址（只用于皮肤页「从皮肤站获取」，记住上次输入） */
  thirdpartyAuthServer: string;
  /** 关闭主窗口时的行为 */
  closeAction: CloseAction;
  /** 需要 Java 时是否自动下载（默认 true） */
  autoDownloadJava: boolean;
  /** 服务端核心下载镜像根地址；留空=官方源 */
  serverMirrorUrl: string;
  /** 每日一言接口地址；留空=内置默认 */
  sayingApiUrl: string;
  /** GitHub 下载保存目录；留空=数据根/DL */
  githubDownloadDir: string;
  javaPath: string;
  memoryMb: number;
  githubAccelerator: GithubAccelerator;
  /** 界面显示字体（来自 fonts_list()），默认「微软雅黑」 */
  uiFont: string;
  /** 背景四方案（互斥，只记当前生效的一种） */
  appearance: AppearanceSettings;
}

/** 关闭窗口的行为：每次询问 / 最小化到后台 / 退出程序。 */
export type CloseAction = "ask" | "minimize" | "exit";

/** 背景方案（枚举值与后端一致）：A 渐变色 / B 本地图 / C API 地址 / D 壁纸引擎。 */
export type BackgroundMode = "gradient" | "local" | "api" | "wallpaper";

/** 轮播方式：单张 / 顺序 / 随机。 */
export type BackgroundPlay = "single" | "shuffle" | "random";

export interface AppearanceSettings {
  mode: BackgroundMode;
  /** A：上面永远白色（gradientFrom），下面用主题色（gradientTo） */
  gradientFrom: string;
  gradientTo: string;
  /** B：本地图文件名（.mc\backgrounds） */
  localFile: string;
  localMode: BackgroundPlay;
  /** 轮播间隔（秒） */
  localIntervalSec: number;
  /** C：图片 URL */
  apiUrl: string;
  apiMode: BackgroundPlay;
  apiIntervalSec: number;
  /** D：壁纸引擎（创意工坊 431960） */
  wallpaperFile: string;
  /** 手动指定的壁纸目录；留空＝用 wallpaper_dir_default() */
  wallpaperDir: string;
  wallpaperMode: BackgroundPlay;
  wallpaperIntervalSec: number;
}

/** background_list 的元素（本地图 / 壁纸引擎共用）。 */
export interface BackgroundItem {
  file: string;
  path: string;
  /** 真正用于背景层播放的地址（视频是 mp4，图片是 png/jpg） */
  url: string;
  /** 网格里的封面（视频壁纸的封面由后端抽帧，等于 url 时直接用 url） */
  previewUrl: string;
  isVideo: boolean;
  size: number;
}

export interface BackgroundPage {
  items: BackgroundItem[];
  total: number;
  page: number;
  pageSize: number;
}

/** 陶瓦联机安装状态：bundled = 随安装包内嵌，downloaded = 用户单独下载。 */
export interface TeraStatus {
  installed: boolean;
  version: string | null;
  source?: "bundled" | "downloaded" | null;
  license?: string | null;
  homepage?: string | null;
}

/** 陶瓦联机运行状态（tera://state 事件与 start/join 的返回同形状）。 */
export interface TeraState {
  hosting: boolean;
  roomCode: string | null;
  players: string[];
}

/** 官网新闻（news_list 的元素）。 */
export interface NewsItem {
  id: string;
  /** 标题（有译文时是中文） */
  title: string;
  /** 摘要（有译文时是中文） */
  summary: string;
  /** 分类徽标，例如「Minecraft 最新新闻」 */
  category: string;
  /** YYYY-MM-DD */
  date: string;
  /** 封面图地址，空串表示没有图（前端画渐变占位） */
  coverUrl: string;
  /** 「阅读全文」跳转地址 */
  readMoreUrl: string;
  /** 后端是否给出了中文译文；false 时界面不显示「原文」按钮 */
  translated: boolean;
  /** 原文标题（可选，用于「原文」切换） */
  titleOriginal?: string;
  /** 原文摘要（可选） */
  summaryOriginal?: string;
}

/** 检查更新结果（update_check 的返回）。 */
export interface UpdateCheckResult {
  hasUpdate: boolean;
  latest: string | null;
  current: string;
  error: string | null;
}

export type UpdatePhase =
  | "idle"
  | "checking"
  | "uptodate"
  | "available"
  | "downloading"
  | "installing"
  | "failed";

/** 「关于」页那一行更新状态（Rust update::check::UpdateStatus）。 */
export interface UpdateStatus {
  phase: UpdatePhase;
  currentVersion: string;
  remoteVersion: string | null;
  message: string;
  checkedAt: number | null;
  canInstall: boolean;
}

/** 系统里能显示中文的字体：family 名 + 中文显示名 + 别名链 + 来源。 */
export interface FontChoice {
  family: string;
  /** 中文显示名，例如「微软雅黑」；缺省用 family */
  displayName?: string;
  /** 同名字体的别名（例如 SimSun / 宋体），写 CSS 时一起回退 */
  aliases?: string[];
  source: string;
}

export interface AppInfo {
  name: string;
  nameEn: string;
  version: string;
  identifier: string;
  tauriVersion: string;
  dataRoot: string;
}

export interface PathsLayout {
  root: string;
  assets: string;
  libraries: string;
  versions: string;
  instances: string;
  runtimes: string;
  skins: string;
  gameIcons: string;
  cache: string;
  logs: string;
  diagnostics: string;
  settingsFile: string;
  dbFile: string;
  versionManifestFile: string;
}

/**
 * 版本清单里的一条。字段名兼容三种后端序列化：
 * - camelCase 的 versionType（我们约定的规范形状）
 * - Mojang 原样的 type（"release" / "snapshot" / "old_beta" / "old_alpha"）
 * - snake_case 的 version_type / release_time
 * 统一用 lib/versions.ts 里的 versionTypeOf() 读取，不要在页面里直接读字段。
 */
export interface VersionSummary {
  id: string;
  versionType?: string;
  type?: string;
  version_type?: string;
  url?: string;
  releaseTime?: string;
  release_time?: string;
}

export interface VersionManifest {
  latestRelease: string;
  latestSnapshot: string;
  versions: VersionSummary[];
  fetchedAt: string | null;
  fromCache: boolean;
  /** 真实后端可能叫 latest.release（嵌套对象），normalizeManifest 会一起处理 */
  latest?: { release?: string; snapshot?: string };
}

export interface Instance {
  id: number;
  name: string;
  mcVersion: string;
  loader: string;
  loaderVersion: string;
  icon: string;
  pinnedHome: boolean;
  playTimeSecs: number;
  createdAt: string;
  lastPlayedAt: string | null;
  /** 整合包名（由 modpack_install 写入） */
  packName?: string;
  /** 整合包安装状态：incomplete 时实例卡上要提示可重试 */
  packState?: "installed" | "incomplete";
}

/** modpack_install 的入参：整合包直接建实例，不需要先选实例。 */
export interface ModpackInstallRequest {
  provider: ContentProvider;
  projectId: string;
  versionId?: string;
  instanceName?: string;
  memoryMb?: number;
}

export interface InstanceCreate {
  name: string;
  mcVersion: string;
  loader?: string;
  loaderVersion?: string;
  /** 实例内存上限（MB），新建实例悬浮窗里的滑块 */
  memoryMb?: number;
}

/** 每日一言（fun_saying 的返回）。 */
export interface SayingResult {
  ok: boolean;
  text: string;
  /** 文案来源（接口域名或 builtin） */
  source: string;
}

/** GitHub 直链下载请求（github_download 的入参）。 */
export interface GithubDownloadRequest {
  url: string;
  /** 保存目录；空字符串＝后端用 数据根/DL */
  dir: string;
  /** 文件名；空字符串＝后端从 URL 推断 */
  fileName: string;
}

/** 翻译接口连通性测试结果（translation_test 的返回）。 */
export interface TranslationTestResult {
  ok: boolean;
  /** 往返耗时（毫秒），失败为 null */
  latencyMs: number | null;
  /** 成功时是模型回显（如 ok），失败时是具体错误 */
  message: string;
}

/** 测速目标：内置源只给 id，自定义源还要带上用户填的根地址。 */
export interface SourceProbeTarget {
  id: DownloadSourceId;
  /** 自定义镜像的根地址（其它源留空） */
  base?: string;
}

/** 测速结果（sources_probe 的返回）：ok=false 时 latencyMs 为 null。 */
export interface SourceProbe {
  id: DownloadSourceId;
  ok: boolean;
  latencyMs: number | null;
  message: string;
}

export interface DownloadSource {
  id: DownloadSourceId;
  label: string;
  description: string;
  base: string;
}

/** 下载任务阶段：文案走 i18n 的 downloads.phase.*。 */
export type DownloadPhase =
  | "queued"
  | "downloading"
  | "verifying"
  | "extracting"
  | "finished"
  | "failed"
  | "cancelled";

/** 任务来源：游戏版本安装 / GitHub 直链下载 / 导出实例包 / 内容安装（进度同一条流）。 */
export type DownloadKind = "install" | "github" | "export" | "mod";

/** 下载中心一条任务（downloads_list() 的元素）。 */
export interface DownloadTask {
  id: string;
  /** 缺省按 install 处理 */
  kind?: DownloadKind;
  title: string;
  /** 阶段文字，例如「正在下载 libraries/…」 */
  stage: string;
  phase: DownloadPhase;
  /** 0..1 */
  progress: number;
  /** 传输速度（字节/秒），非下载阶段为 0 */
  bytesPerSec: number;
  receivedBytes: number | null;
  totalBytes: number | null;
  /** 任务落地目录，「打开文件夹」用 */
  targetDir: string;
  createdAt: string;
  finishedAt: string | null;
  error: string | null;
}

/**
 * 安装进度事件（install://progress）。字段名与 Rust 侧 serde camelCase 对齐。
 * TODO(M2-A): Rust 侧按这个形状发事件。
 */
export interface InstallProgress {
  taskId: string;
  title?: string;
  phase: DownloadPhase;
  stage: string;
  progress: number;
  bytesPerSec: number;
  receivedBytes: number | null;
  totalBytes: number | null;
}

/** 安装失败事件（install://error）。 */
export interface InstallError {
  taskId: string;
  message: string;
}

/** 游戏日志事件（game://log），一行一条。 */
export interface GameLog {
  instanceId: number;
  line: string;
}

/** 游戏退出事件（game://exit）。 */
export interface GameExit {
  instanceId: number;
  code: number;
}

/** 陶瓦联机日志（tera://log），一行一条。 */
export interface TeraLog {
  line: string;
}

/** 第二个实例被拦下时后端发的事件（app://second-instance）。 */
export interface SecondInstance {
  /** 后端给的一句话，缺省时前端用内置文案 */
  message?: string;
}

/** 启动器事件总线的统一载体（页面只订阅这一个）。 */
export type LauncherEvent =
  | { kind: "second-instance"; payload: SecondInstance }
  | { kind: "server-log"; payload: { id: string; line: string } }
  | { kind: "server-state"; payload: { id: string; state: ServerState; players?: number } }
  | { kind: "tera-log"; payload: TeraLog }
  | { kind: "tera-state"; payload: TeraState }
  | { kind: "install-progress"; payload: InstallProgress }
  | { kind: "install-error"; payload: InstallError }
  | { kind: "game-log"; payload: GameLog }
  | { kind: "game-exit"; payload: GameExit };

/** 系统里可用的 Java 运行时（java_list()，后端已按 path 去重）。 */
export interface JavaRuntime {
  /** 运行时根目录 */
  path: string;
  /** 真正要执行的 javaw.exe 绝对路径 */
  javaExe: string;
  /** 完整版本号，例如 17.0.18 */
  version: string;
  /** 主版本号，例如 17 */
  major: number;
  arch: string;
  /** 发行商：System / BellSoft Liberica / Adoptium / Zulu / Oracle / Bundled */
  vendor: string;
  /** 是否是设置里当前选中的那个 */
  isDefault: boolean;
}

/** Java 下载源（java_sources()）。 */
export interface JavaSource {
  id: string;
  name: string;
  description: string;
  /** Oracle JDK 需要接受许可：按钮改成「前往下载页」 */
  requiresLicense?: boolean;
}

/** 源测速结果（java_probe_sources()）。 */
export interface JavaSourceProbe {
  sourceId: string;
  ok: boolean;
  latencyMs: number | null;
  error?: string | null;
}

/** 可下载的 Java 版本（java_versions()）。 */
export interface JavaVersion {
  source: string;
  version: string;
  major: number;
  lts: boolean;
  sizeBytes: number;
  url: string;
}

/** 服务端状态：已停止 / 启动中 / 运行中 / 停止中。 */
export type ServerState = "stopped" | "starting" | "running" | "stopping";

/** server_cores 的元素。 */
export interface ServerCore {
  id: string;
  name: string;
  latestVersion?: string | null;
  downloadPage?: string | null;
}

/** server_versions 的元素。 */
export interface ServerVersion {
  version: string;
  build?: string | null;
  sizeBytes?: number | null;
  url: string;
}

/** server_list 的元素。 */
export interface ServerInstance {
  id: string;
  name: string;
  core: string;
  mcVersion: string;
  port: number;
  memoryMb: number;
  state: ServerState;
  players?: number;
  dir: string;
}

/** server_content_list 的元素：服务端里的一个 jar。 */
export interface ServerContentItem {
  fileName: string;
  sizeBytes: number;
  enabled: boolean;
  provider?: ContentProvider | null;
  projectId?: string | null;
}

/** server_saves_list 的元素：服务端世界目录。 */
export interface ServerSaveItem {
  name: string;
  sizeBytes: number;
  updatedAt: string;
}

/** server_config_read 的返回：原文 + 常用项。 */
export interface ServerConfigResult {
  text: string;
  fields: Record<string, string>;
}

/** server_console_state 的返回：控制台能不能敲指令。 */
export interface ServerConsoleState {
  running: boolean;
  stdinOpen: boolean;
}

/** server_create 的入参。 */
export interface ServerCreateRequest {
  name: string;
  core: string;
  mcVersion: string;
  memoryMb: number;
  port: number;
  eula: boolean;
}

/** 模组/光影/资源包检查更新的结果。 */
export interface ModUpdateItem {
  fileName: string;
  path: string;
  kind: string;
  currentVersion?: string | null;
  latestVersion: string;
  downloadUrl: string;
  sha1?: string | null;
}

export interface ModCheckResult {
  checked: number;
  updates: ModUpdateItem[];
  failed: Array<{ fileName: string; reason: string }>;
}

export interface ModApplyResult {
  updated: number;
  failed: Array<{ fileName: string; reason: string }>;
}

/** 外部启动器导入（HMCL / PCL2）。 */
export interface ExternalCandidate {
  id: string;
  name: string;
  mcVersion: string;
  loader: string;
  loaderVersion?: string | null;
  path: string;
  isolated: boolean;
  hasMods: boolean;
  modCount?: number | null;
  sizeBytes: number;
  javaMajor?: number | null;
  broken?: string | null;
}

export interface ExternalScanResult {
  launcher: "hmcl" | "pcl2" | "unknown";
  minecraftDir: string;
  candidates: ExternalCandidate[];
}

export interface ExternalImportResult {
  instances: Instance[];
  failed: Array<{ id: string; reason: string }>;
}

/** 存储页的分项 key。 */
export type StorageKey = "instances" | "versions" | "libraries" | "assets" | "cache" | "logs" | "runtimes";

export interface StorageStatItem {
  key: StorageKey;
  label: string;
  bytes: number;
  files: number;
  path: string;
}

export interface StorageStats {
  totalBytes: number;
  items: StorageStatItem[];
}

/** 数据目录信息（data_dir_get()）。 */
export interface DataDirInfo {
  path: string;
  sizeBytes: number;
  fileCount: number;
  isDefault: boolean;
}

/** 实例的可编辑设置（实例详情页「设置」标签）。 */
export interface InstanceSettings {
  /** 后端给的可直接进入存档标记（= mcVersion >= 1.20） */
  supportsQuickPlay?: boolean;
  name: string;
  note: string;
  memoryMb: number;
  windowWidth: number;
  windowHeight: number;
  /** 空字符串＝跟随全局设置 */
  javaPath: string;
  /** 实例目录下的 icon.png 绝对路径，空字符串＝没有自定义图标 */
  iconPath: string;
}

/** 实例日志文件（.mc/logs/*.log）。 */
export interface LogFile {
  name: string;
  path: string;
  size: number;
  modifiedAt: string;
}

// ============================================================ M3-2 实例内容

/** 实例内容目录种类：模组 / 资源包 / 光影。 */
export type ContentFileKind = "mod" | "resourcepack" | "shader";

/** mods_list / mods_set_enabled / mods_delete / mods_import 的元素。 */
export interface ModFileItem {
  /** 磁盘上的真实文件名（禁用时是 xxx.jar.disabled） */
  file: string;
  /** 去掉扩展名与 .disabled 的显示名 */
  name: string;
  size: number;
  enabled: boolean;
  /** 从元数据里识别出的版本号，识别不出为 null */
  version: string | null;
  /** 识别出的加载器（Fabric / Forge / NeoForge …），识别不出为 null */
  loader: string | null;
  /** 来源徽标（Modrinth / CurseForge / 手动导入），可为 null */
  source: string | null;
}

/** saves_list 的元素。 */
export interface SaveItem {
  name: string;
  size: number;
  /** ISO 时间串 */
  modifiedAt: string;
  /** 存档 level.dat 里的游戏版本，识别不出为空字符串 */
  mcVersion: string;
}

// ================================================================ M3-2 皮肤

/** skins_list 的元素：本地 .mc\skins\ 下的一张皮肤。 */
/** 皮肤来源：内置 / 本地保存 / 正版玩家 / 皮肤站。 */
export type SkinSource = "local" | "mojang" | "thirdparty" | "builtin";

export interface SkinItem {
  name: string;
  path: string;
  size: number;
  /** 皮肤 PNG 的地址（data URL 或打包后的资源 URL），供 <img>/canvas 直接画 */
  dataUrl: string;
  /** 内置皮肤（Steve / Alex）：不可删除，带「内置」徽标 */
  builtin?: boolean;
  /** 来源徽标 */
  source?: SkinSource;
  /** 正版 / 皮肤站玩家的 UUID */
  uuid?: string;
}

/** skin_lookup_thirdparty 的返回：皮肤站上某个玩家的皮肤（只用于预览）。 */
export interface SkinLookupResult {
  uuid: string;
  name: string;
  skinUrl: string;
  capeUrl?: string | null;
  /** "default" 经典 / "slim" 纤细，缺省按贴图自动判断 */
  model?: SkinModel | null;
}

/** capes_list 的元素：账号里可选的一张披风。 */
export interface CapeListItem {
  id: string;
  name: string;
  /** 同一张披风的另一种叫法（有就跟着名称一起显示） */
  alias?: string | null;
  /** 披风贴图（64×32 PNG）的地址 */
  url: string;
  state: "ACTIVE" | "INACTIVE";
  /** 后端预生成的正面裁切图；没有就用 url 自己裁 */
  previewDataUrl?: string | null;
}

/** capes_list 的结果：supported=false 时 items 为空，reason 是原样给用户看的一句话。 */
export interface CapesListResult {
  supported: boolean;
  reason?: string | null;
  active?: string | null;
  items: CapeListItem[];
}

/** cape_set_active 的结果。 */
export interface CapeState {
  active: string | null;
}

/** 皮肤模型：default = 经典（Steve），slim = 纤细（Alex）。 */
/** 皮肤模型：default/classic 都是经典（4×3 手臂），slim 是纤细（3×3 手臂）。 */
export type SkinModel = "default" | "classic" | "slim";

// ================================================================ M3-2 账号

export type AccountType = "microsoft" | "offline";

export interface Account {
  id: string;
  name: string;
  type: AccountType;
  /** 后端新字段（"ms" / "offline"）；前端统一折成 type，兼容两种写法 */
  kind?: string;
  /** 游戏内 UUID（离线账号是 name 的离线 UUID） */
  uuid: string;
  /** 优先使用账号自带皮肤，没有就用当前皮肤 */
  skinDataUrl: string | null;
  /** 是否当前正在使用 */
  current: boolean;
}

/** ms_login_start 的返回；configured=false 表示这个构建没内置 client id。 */
/** ms_auth_status 的返回：有没有可用的 Client ID。 */
export interface MsAuthStatus {
  configured: boolean;
  /** 给界面用的一句话说明（未配置原因 / 已配置时的空串） */
  hint: string;
  /** 读盘时发现已保存的值不是合法 UUID */
  invalid?: boolean;
}

export interface MsLoginStart {
  configured: boolean;
  userCode: string | null;
  verifyUrl: string | null;
  /** 新契约的名字（和 verifyUrl 同值，两个都认） */
  verificationUri?: string | null;
  deviceCode: string | null;
  expiresIn: number | null;
}

/** ms_profile 的返回：微软账号当前正在使用的外观。 */
export interface MsProfile {
  name: string;
  uuid: string;
  /** textures.minecraft.net 的贴图地址；空表示账号用的是默认皮肤 */
  skinUrl?: string | null;
  capeUrl?: string | null;
  variant?: "classic" | "slim" | null;
}

/** ms_login_poll 的返回。 */
export interface MsLoginPoll {
  state: "pending" | "done" | "error";
  account: Account | null;
  message: string | null;
}

// ============================================================ M3-2 内容服务

/** 内容来源：与 settings.resourceSource 同集合（auto/official 不会出现在内容服务页）。 */
export type ContentProvider = "modrinth" | "curseforge" | "mcim";

/** 内容类型胶囊：模组 / 整合包 / 光影 / 资源包。 */
export type ContentType = "mod" | "modpack" | "shader" | "resourcepack";

/** 内容类型（含数据包 / 插件）。 */
export type ContentKindTab = ContentType | "datapack" | "plugin";

/** 排序方式：下载量 / 最近更新 / 相关度。 */
export type ContentSort = "downloads" | "updated" | "relevance";

/** content_search 的入参（分页版）。 */
export interface ContentSearchQuery {
  provider: ContentProvider;
  kind: ContentKindTab;
  query: string;
  mcVersion?: string;
  loader?: string;
  categories?: string[];
  sort?: ContentSort;
  page: number;
  pageSize: number;
}

/** 搜索结果里的项目卡片。 */
export interface ContentItem {
  /** 项目 id（详情 / 安装都用它） */
  id: string;
  provider: ContentProvider;
  kind: ContentKindTab;
  /** 标题（译文优先；没译文时等于原文） */
  title: string;
  titleOriginal?: string;
  author: string;
  downloads: number;
  /** 关注数；没有就不显示 */
  follows?: number;
  /** ISO 时间，卡片显示相对时间 */
  updatedAt: string;
  summary: string;
  summaryOriginal?: string;
  categories: string[];
  loaders: string[];
  iconUrl: string;
  /** 后端是否给出了译文；false 时前端会去 translate_texts 补 */
  translated: boolean;
}

export interface ContentSearchResult {
  items: ContentItem[];
  total: number;
  page: number;
  pageSize: number;
}

/** content_detail 的返回。 */
export interface ProjectDetail {
  projectId: string;
  provider: ContentProvider;
  title: string;
  summary: string;
  /** 正文（Markdown 原文） */
  description: string;
  /** 正文译文，未翻译为 null */
  descriptionTranslated: string | null;
  iconUrl: string;
  downloads: number;
  author: string;
  /** 项目主页 */
  url: string;
  license: string;
  updatedAt: string;
}

/** 版本列表里的依赖提示。 */
export interface VersionDependency {
  projectId: string;
  name: string;
  /** required / optional / incompatible */
  kind: string;
}

/** 版本通道：正式版 / 测试版 / 先行版。 */
export type VersionChannel = "release" | "beta" | "alpha";

/** 前置依赖条目（content_dependencies 的返回）。 */
export interface ContentDependency {
  projectId: string;
  name: string;
  /** required / optional */
  kind: "required" | "optional";
  /** 提供方 */
  provider?: ContentProvider;
}

export interface VersionItem {
  /** 安装时传给 mod_install 的 versionId */
  versionId: string;
  /** 显示名（版本号或文件名） */
  name: string;
  gameVersions: string[];
  loaders: string[];
  downloads: number;
  fileSize: number;
  publishedAt: string;
  dependencies: VersionDependency[];
  /** 版本通道，缺省按 release 处理 */
  channel?: VersionChannel;
  /** 服务端侧标识，形如 "Server, 1.21.4" */
  serverLabel?: string;
}

/** translate_texts 的返回：results 与入参一一对应，翻不出来是 null。 */
export interface TranslateResult {
  results: Array<string | null>;
  /** 内置接口命中频率上限时为 true（界面只提示，不弹错误框） */
  limited: boolean;
}


