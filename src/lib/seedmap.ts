/**
 * 种子地图的后端封装（M12）。
 *
 * 命令契约（由并行实现 FFI 的后端代理提供，别在这里自己发明字段）：
 *   seedmap_versions()                                   -> [{ mc, label }]
 *   seedmap_biomes({ mc, dimension, seed, scale, x0, z0, tilesX, tilesY })
 *                                                        -> { width, height, scale, originX, originZ, rgba, biomes? }
 *   seedmap_biome_at({ mc, dimension, seed, x, z })      -> { biomeId, biomeName, x, z }
 *   seedmap_spawn({ mc, seed })                          -> { x, z }
 *   seedmap_structures({ mc, seed, structure, x, z, radius }) -> [{ kind, x, z }]
 *
 * scale 表示「1 px 对应多少方块」；dimension 0 主世界 / 1 下界 / 2 末地。
 * 浏览器预览（isTauri() === false）走本文件的 mock：用确定性噪声合成地形，
 * 不连后端也能看到彩色地图、结构标记与群系读数。
 */
import { invoke } from "@tauri-apps/api/core";
import { biomeRgb, biomeName as biomeLabel } from "./biomeColors";
import { isTauri } from "./tauri-api";

// ------------------------------------------------------------------ 类型

export type SeedmapDimension = 0 | 1 | 2;

export interface SeedmapVersion {
  mc: string;
  label: string;
}

export interface SeedmapBiomesRequest {
  mc: string;
  dimension: SeedmapDimension;
  seed: string;
  /** 1 px 对应多少方块 */
  scale: number;
  x0: number;
  z0: number;
  tilesX: number;
  tilesY: number;
}

export interface SeedmapBiomesResult {
  width: number;
  height: number;
  scale: number;
  originX: number;
  originZ: number;
  /** PNG base64（以 iVBOR 开头）或原始 RGBA base64 */
  rgba: string;
  /** 每个像素的群系 id，行优先；用来做「高亮地形」遮罩 */
  biomes?: number[];
}

export interface SeedmapBiomeAtRequest {
  mc: string;
  dimension: SeedmapDimension;
  seed: string;
  x: number;
  z: number;
}

export interface SeedmapBiomeAtResult {
  biomeId: number;
  biomeName: string;
  x: number;
  z: number;
}

export interface SeedmapSpawnRequest {
  mc: string;
  seed: string;
}

export interface SeedmapSpawnResult {
  x: number;
  z: number;
}

export interface SeedmapStructuresRequest {
  mc: string;
  seed: string;
  structure: string;
  x: number;
  z: number;
  radius: number;
}

export interface SeedmapStructure {
  kind: string;
  x: number;
  z: number;
}

/** 解码成可以直接 drawImage 的位图 + 坐标换算需要的那几个字段。 */
export interface SeedmapTile {
  canvas: HTMLCanvasElement;
  width: number;
  height: number;
  scale: number;
  originX: number;
  originZ: number;
  biomes: number[] | null;
}

// -------------------------------------------------------------- 自检记录

/**
 * 自检用：所有种子地图命令的参数都记到 window.__guguSeedmapReq（最多 60 条）。
 * CDP 靠它断言「拖拽后请求参数变了」「勾结构后会请求 seedmap_structures」。
 */
function record(cmd: string, args: unknown): void {
  if (typeof window === "undefined") return;
  const host = window as unknown as Record<string, unknown>;
  const list = Array.isArray(host.__guguSeedmapReq) ? (host.__guguSeedmapReq as Array<Record<string, unknown>>) : [];
  const entry: Record<string, unknown> = { cmd };
  if (args && typeof args === "object") Object.assign(entry, args as Record<string, unknown>);
  host.__guguSeedmapReq = [...list, entry].slice(-60);
}

async function call<T>(cmd: string, args: Record<string, unknown> | undefined, mock: () => T | Promise<T>): Promise<T> {
  record(cmd, args);
  if (isTauri()) return (await invoke(cmd, args)) as T;
  return await mock();
}

// ------------------------------------------------------------------ mock

const MOCK_DELAY_KEY = "gugu.mock.seedmapDelay";
const MOCK_FAIL_KEY = "gugu.mock.seedmapFail";

/** 顺序跟真后端一致：从新到旧，首项就是默认选中项。 */
const MOCK_VERSIONS: SeedmapVersion[] = [
  { mc: "26.3", label: "26.3" },
  { mc: "1.21.4", label: "1.21.4" },
  { mc: "1.20.1", label: "1.20.1" },
  { mc: "1.19.4", label: "1.19.4" },
  { mc: "1.18.2", label: "1.18.2" },
  { mc: "1.12.2", label: "1.12.2" },
];

/** mock 的「计算耗时」：默认 220ms，自检可以用 gugu.mock.seedmapDelay 调大/调小。 */
function mockDelay(): number {
  const raw = window.localStorage.getItem(MOCK_DELAY_KEY);
  const value = raw === null ? Number.NaN : Number(raw);
  return Number.isFinite(value) && value >= 0 ? value : 220;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

/** 让失败态可复现：localStorage 里放 gugu.mock.seedmapFail=1 时，地形请求故意报错。 */
function mockGuard(): void {
  if (window.localStorage.getItem(MOCK_FAIL_KEY) === "1") {
    throw new Error("cubiomes 计算失败：读取版本数据时出错（mock 注入，清掉 gugu.mock.seedmapFail 即恢复）");
  }
}

/** 种子可能是负数或超长数字，一律按字符串滚动哈希成 32 位整数，够 mock 用。 */
function seedHash(seed: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < seed.length; i += 1) hash = (Math.imul(hash, 31) + seed.charCodeAt(i)) | 0;
  return hash >>> 0;
}

function hash2(seed: number, x: number, z: number): number {
  let h = Math.imul(x | 0, 374761393) ^ Math.imul(z | 0, 668265263) ^ Math.imul(seed | 0, 2246822519);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

/** 双线性插值的 value noise，0..1。 */
function valueNoise(seed: number, x: number, z: number): number {
  const xi = Math.floor(x);
  const zi = Math.floor(z);
  const tx = x - xi;
  const tz = z - zi;
  const sx = tx * tx * (3 - 2 * tx);
  const sz = tz * tz * (3 - 2 * tz);
  const a = hash2(seed, xi, zi);
  const b = hash2(seed, xi + 1, zi);
  const c = hash2(seed, xi, zi + 1);
  const d = hash2(seed, xi + 1, zi + 1);
  const top = a + (b - a) * sx;
  const bottom = c + (d - c) * sx;
  return top + (bottom - top) * sz;
}

function fbm(seed: number, x: number, z: number, octaves: number): number {
  let total = 0;
  let amp = 1;
  let freq = 1;
  let norm = 0;
  for (let i = 0; i < octaves; i += 1) {
    total += valueNoise(seed + i * 7919, x * freq, z * freq) * amp;
    norm += amp;
    amp *= 0.5;
    freq *= 2;
  }
  return total / norm;
}

/** 合成地形：温度 / 湿度 / 高度三种噪声映射到群系 id，纯函数，坐标稳定。 */
function mockBiomeId(dimension: number, seed: number, wx: number, wz: number): number {
  if (dimension === 1) {
    const nether = fbm(seed ^ 0x5eed01, wx / 260, wz / 260, 3);
    if (nether < 0.2) return 170;
    if (nether < 0.42) return 8;
    if (nether < 0.6) return 171;
    if (nether < 0.76) return 172;
    return 173;
  }
  if (dimension === 2) {
    const end = fbm(seed ^ 0x5eed02, wx / 900, wz / 900, 3);
    if (end < 0.35) return 43;
    if (end < 0.55) return 9;
    if (end < 0.72) return 41;
    if (end < 0.88) return 40;
    return 42;
  }
  // 尺度往大了调：群系块连片才有地图感，太碎会糊成噪声
  const continent = fbm(seed ^ 0x11, wx / 1400, wz / 1400, 3);
  const temp = fbm(seed ^ 0x22, wx / 1050, wz / 1050, 2);
  const humid = fbm(seed ^ 0x33, wx / 780, wz / 780, 2);
  const alt = fbm(seed ^ 0x44, wx / 560, wz / 560, 2);
  // 海岸线附近拉一条河，地图上看起来更像真的
  if (Math.abs(continent - 0.42) < 0.004) return temp < 0.26 ? 11 : 7;
  if (continent < 0.42) {
    if (continent < 0.28) {
      if (temp > 0.72) return 47;
      if (temp > 0.5) return 48;
      if (temp < 0.24) return 50;
      return 24;
    }
    if (temp > 0.7) return 44;
    if (temp > 0.45) return 45;
    if (temp < 0.25) return 10;
    return 0;
  }
  if (alt > 0.8) {
    if (temp < 0.3) return 181;
    if (temp < 0.45) return 179;
    return 34;
  }
  if (temp < 0.26) return humid > 0.55 ? 30 : 12;
  if (temp > 0.72) {
    if (humid < 0.32) return 2;
    if (humid < 0.55) return 35;
    return 21;
  }
  if (humid < 0.3) return 3;
  if (humid < 0.42) return 37;
  if (humid < 0.55) return 1;
  if (humid < 0.68) return 4;
  if (humid < 0.8) return 27;
  return 29;
}

function clampInt(value: number, min: number, max: number, fallback: number): number {
  const number = Number.isFinite(value) ? Math.round(value) : fallback;
  return Math.max(min, Math.min(max, number));
}

function base64FromBytes(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x4000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...Array.from(bytes.subarray(i, i + chunk)));
  }
  return window.btoa(binary);
}

function bytesFromBase64(payload: string): Uint8Array {
  const binary = window.atob(payload);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** 末地特意走 PNG 分支：让页面里「PNG / 原始 RGBA」两条解码路径都能被真跑到。 */
function rgbaToPngBase64(width: number, height: number, rgba: Uint8ClampedArray): string {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return base64FromBytes(new Uint8Array(rgba.buffer, rgba.byteOffset, rgba.byteLength));
  const image = ctx.createImageData(width, height);
  image.data.set(rgba);
  ctx.putImageData(image, 0, 0);
  const url = canvas.toDataURL("image/png");
  return url.slice(url.indexOf(",") + 1);
}

function mockBiomesResult(request: SeedmapBiomesRequest): SeedmapBiomesResult {
  const tilesX = clampInt(request.tilesX, 1, 512, 64);
  const tilesY = clampInt(request.tilesY, 1, 512, 64);
  const scale = Number.isFinite(request.scale) && request.scale > 0 ? request.scale : 1;
  const originX = Math.round(Number.isFinite(request.x0) ? request.x0 : 0);
  const originZ = Math.round(Number.isFinite(request.z0) ? request.z0 : 0);
  const dimension = request.dimension === 1 ? 1 : request.dimension === 2 ? 2 : 0;
  const seed = seedHash(request.seed ?? "");
  const biomes = new Array<number>(tilesX * tilesY);
  const rgba = new Uint8ClampedArray(tilesX * tilesY * 4);
  for (let row = 0; row < tilesY; row += 1) {
    const worldZ = originZ + row * scale;
    for (let col = 0; col < tilesX; col += 1) {
      const worldX = originX + col * scale;
      const id = mockBiomeId(dimension, seed, worldX, worldZ);
      const index = row * tilesX + col;
      biomes[index] = id;
      const [r, g, b] = biomeRgb(id);
      const offset = index * 4;
      rgba[offset] = r;
      rgba[offset + 1] = g;
      rgba[offset + 2] = b;
      rgba[offset + 3] = 255;
    }
  }
  const rgbaBase64 =
    dimension === 2 ? rgbaToPngBase64(tilesX, tilesY, rgba) : base64FromBytes(new Uint8Array(rgba.buffer, rgba.byteOffset, rgba.byteLength));
  return { width: tilesX, height: tilesY, scale, originX, originZ, rgba: rgbaBase64, biomes };
}

/** 结构在网格上撒点，种子与坐标决定位置 → 同一视口反复请求结果一致。 */
const MOCK_STRUCTURE_SPACING: Record<string, number> = {
  village: 512,
  monument: 1024,
  mansion: 2048,
  stronghold: 1400,
  fortress: 640,
  bastion: 900,
  outpost: 700,
  shipwreck: 336,
  ruins: 384,
  treasure: 288,
};

function mockStructures(request: SeedmapStructuresRequest): SeedmapStructure[] {
  const seed = seedHash(request.seed ?? "");
  const spacing = MOCK_STRUCTURE_SPACING[request.structure] ?? 512;
  const radius = Math.max(64, Math.min(24000, Number.isFinite(request.radius) ? request.radius : 1024));
  const centerX = Number.isFinite(request.x) ? request.x : 0;
  const centerZ = Number.isFinite(request.z) ? request.z : 0;
  const kind = request.structure;
  const kindSeed = seedHash(kind);
  const out: SeedmapStructure[] = [];
  const fromX = Math.floor((centerX - radius) / spacing);
  const toX = Math.floor((centerX + radius) / spacing);
  const fromZ = Math.floor((centerZ - radius) / spacing);
  const toZ = Math.floor((centerZ + radius) / spacing);
  for (let gx = fromX; gx <= toX; gx += 1) {
    for (let gz = fromZ; gz <= toZ; gz += 1) {
      const roll = hash2(seed ^ kindSeed, gx, gz);
      if (roll < 0.42) continue;
      const x = Math.round(gx * spacing + (hash2(seed ^ 0x7a1, gx, gz) - 0.5) * spacing * 0.6);
      const z = Math.round(gz * spacing + (hash2(seed ^ 0x7a2, gx, gz) - 0.5) * spacing * 0.6);
      if (Math.abs(x - centerX) > radius || Math.abs(z - centerZ) > radius) continue;
      out.push({ kind, x, z });
    }
  }
  // 只留离中心最近的 48 个：地图视口只显示中心附近一小块，先扫到的左上角一批会全在视野外
  out.sort((left, right) => (left.x - centerX) ** 2 + (left.z - centerZ) ** 2 - ((right.x - centerX) ** 2 + (right.z - centerZ) ** 2));
  return out.slice(0, 48);
}

function mockBiomeAt(request: SeedmapBiomeAtRequest): SeedmapBiomeAtResult {
  const dimension = request.dimension === 1 ? 1 : request.dimension === 2 ? 2 : 0;
  const id = mockBiomeId(dimension, seedHash(request.seed ?? ""), request.x, request.z);
  return { biomeId: id, biomeName: biomeLabel(id), x: request.x, z: request.z };
}

function mockSpawn(request: SeedmapSpawnRequest): SeedmapSpawnResult {
  const seed = seedHash(request.seed ?? "");
  const x = Math.round((hash2(seed ^ 0x31, 11, 23) - 0.5) * 4000);
  return { x, z: Math.round((hash2(seed ^ 0x51, 29, 7) - 0.5) * 4000) };
}

// ------------------------------------------------------------------ API

/**
 * 版本列表归一化：真后端回数组，但字段可能是 mc / id / version，值也可能是数字；
 * 万一被包成 { versions: [...] } 也认。归一化后「第一项」才是可靠的默认选中项。
 */
export function normalizeVersions(raw: unknown): SeedmapVersion[] {
  const wrapped = raw && typeof raw === "object" ? (raw as { versions?: unknown }).versions : undefined;
  const list: unknown[] = Array.isArray(raw) ? raw : Array.isArray(wrapped) ? wrapped : [];
  const out: SeedmapVersion[] = [];
  for (const item of list) {
    if (typeof item === "string" || typeof item === "number") {
      const mc = String(item).trim();
      if (mc) out.push({ mc, label: mc });
      continue;
    }
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    const mc = String(record.mc ?? record.id ?? record.version ?? "").trim();
    if (!mc) continue;
    const label = String(record.label ?? record.name ?? mc).trim();
    out.push({ mc, label: label || mc });
  }
  return out;
}

export const seedmapApi = {
  versions(): Promise<SeedmapVersion[]> {
    return call("seedmap_versions", undefined, () => MOCK_VERSIONS.map((item) => ({ ...item }))).then(normalizeVersions);
  },

  async biomes(request: SeedmapBiomesRequest): Promise<SeedmapBiomesResult> {
    record("seedmap_biomes", { ...request });
    if (isTauri()) return (await invoke("seedmap_biomes", { ...request })) as SeedmapBiomesResult;
    await sleep(mockDelay());
    mockGuard();
    return mockBiomesResult(request);
  },

  async biomeAt(request: SeedmapBiomeAtRequest): Promise<SeedmapBiomeAtResult> {
    record("seedmap_biome_at", { ...request });
    if (isTauri()) return (await invoke("seedmap_biome_at", { ...request })) as SeedmapBiomeAtResult;
    await sleep(30);
    return mockBiomeAt(request);
  },

  async spawn(request: SeedmapSpawnRequest): Promise<SeedmapSpawnResult> {
    record("seedmap_spawn", { ...request });
    if (isTauri()) return (await invoke("seedmap_spawn", { ...request })) as SeedmapSpawnResult;
    await sleep(120);
    return mockSpawn(request);
  },

  async structures(request: SeedmapStructuresRequest): Promise<SeedmapStructure[]> {
    record("seedmap_structures", { ...request });
    if (isTauri()) return (await invoke("seedmap_structures", { ...request })) as SeedmapStructure[];
    await sleep(140);
    const list = mockStructures(request);
    return list;
  },
};

// -------------------------------------------------------- 位图解码 / 上色

function isPngPayload(payload: string): boolean {
  return payload.startsWith("iVBOR") || payload.startsWith("data:image/png");
}

function drawPng(canvas: HTMLCanvasElement, url: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => {
      const ctx = canvas.getContext("2d");
      if (!ctx) {
        reject(new Error("当前环境不支持 canvas 绘制"));
        return;
      }
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
      resolve();
    };
    image.onerror = () => reject(new Error("地图 PNG 解码失败"));
    image.src = url;
  });
}

/**
 * 后端可能给 PNG base64，也可能给原始 RGBA base64（900KB 级别的原始字节更省 CPU）。
 * 两种都在这里折成一张 canvas：PNG 直接画，RGBA 走 ImageData；
 * 万一给的是「一像素一字节」的群系索引，就按 biomeColors 的表自己上色。
 */
export async function decodeBiomes(result: SeedmapBiomesResult, fallbackWidth: number, fallbackHeight: number): Promise<HTMLCanvasElement> {
  const width = clampInt(result.width, 1, 8192, fallbackWidth);
  const height = clampInt(result.height, 1, 8192, fallbackHeight);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const payload = String(result.rgba ?? "").trim();
  if (!payload) throw new Error("后端没有返回地图像素数据");
  if (isPngPayload(payload)) {
    await drawPng(canvas, payload.startsWith("data:") ? payload : "data:image/png;base64," + payload);
    return canvas;
  }
  const bytes = bytesFromBase64(payload);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("当前环境不支持 canvas 绘制");
  const image = ctx.createImageData(width, height);
  const pixels = width * height;
  if (bytes.length >= pixels * 4) {
    image.data.set(bytes.subarray(0, pixels * 4));
  } else if (bytes.length >= pixels) {
    for (let i = 0; i < pixels; i += 1) {
      const [r, g, b] = biomeRgb(bytes[i]);
      image.data[i * 4] = r;
      image.data[i * 4 + 1] = g;
      image.data[i * 4 + 2] = b;
      image.data[i * 4 + 3] = 255;
    }
  } else {
    throw new Error("地图像素数据长度不对：" + bytes.length + " 字节，期望 " + pixels * 4);
  }
  ctx.putImageData(image, 0, 0);
  return canvas;
}

/** 请求一屏瓦片并解码成可绘制的位图。 */
export async function loadTile(request: SeedmapBiomesRequest): Promise<SeedmapTile> {
  const result = await seedmapApi.biomes(request);
  const canvas = await decodeBiomes(result, request.tilesX, request.tilesY);
  return {
    canvas,
    width: canvas.width,
    height: canvas.height,
    scale: Number.isFinite(result.scale) && result.scale > 0 ? result.scale : request.scale,
    originX: Number.isFinite(result.originX) ? result.originX : request.x0,
    originZ: Number.isFinite(result.originZ) ? result.originZ : request.z0,
    biomes: Array.isArray(result.biomes) && result.biomes.length >= canvas.width * canvas.height ? result.biomes : null,
  };
}

/** 后端报错可能直接是字符串（Tauri 的 Err(String)），统一折成一句中文。 */
export function seedmapErrorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
