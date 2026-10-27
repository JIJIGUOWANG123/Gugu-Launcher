/**
 * 种子地图页（M12）：照着参照图做的单页地图工具。
 *
 * 分工：
 *   - 视口 → 后端瓦片：lib/seedmap.ts（命令封装 + mock）
 *   - 群系上色 / 中文名：lib/biomeColors.ts
 *   - 本文件只做布局、交互、canvas 合成（底图 / 阴影 / 高亮遮罩 / 区块线 / 结构标记）
 *
 * 坐标约定：视口左上角对应世界坐标 (center.x - w*bpp/2, center.z - h*bpp/2)，
 * bpp = ZOOM_LEVELS[zoom]（1 CSS px 对应多少方块）。底图按「返回的 origin/width/scale」换算，
 * 所以后端 scale 与请求值不完全一致时也不会错位。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { biomeName, searchBiomes } from "../lib/biomeColors";
import { copyText } from "../lib/clipboard";
import { useT } from "../lib/i18n";
import {
  loadTile,
  seedmapApi,
  seedmapErrorText,
  type SeedmapDimension,
  type SeedmapStructure,
  type SeedmapTile,
  type SeedmapVersion,
} from "../lib/seedmap";

/** 离散缩放档位：1 CSS px 对应的方块数。滚轮只在这些档位之间跳。 */
const ZOOM_LEVELS = [1, 2, 4, 8, 16, 32, 64];
/** 单次请求的瓦片上限定额：tilesX * tilesY ≤ 512 * 512 */
const MAX_TILES = 512;
const TILE_BUDGET = MAX_TILES * MAX_TILES;
const HISTORY_KEY = "gugu.seedmap.history";
const HISTORY_MAX = 5;
const BIOME_THROTTLE_MS = 120;

interface StructureKind {
  id: string;
  labelKey: string;
  color: string;
}

/** 结构类型：id 就是后端 seedmap_structures 的 structure 参数。 */
const STRUCTURES: StructureKind[] = [
  { id: "village", labelKey: "seedmap.structure.village", color: "#f5c845" },
  { id: "monument", labelKey: "seedmap.structure.monument", color: "#3fa9d8" },
  { id: "mansion", labelKey: "seedmap.structure.mansion", color: "#c98b5f" },
  { id: "stronghold", labelKey: "seedmap.structure.stronghold", color: "#b0bec5" },
  { id: "fortress", labelKey: "seedmap.structure.fortress", color: "#e0625b" },
  { id: "bastion", labelKey: "seedmap.structure.bastion", color: "#8d8d8d" },
  { id: "outpost", labelKey: "seedmap.structure.outpost", color: "#d7a86e" },
  { id: "shipwreck", labelKey: "seedmap.structure.shipwreck", color: "#4db6ac" },
  { id: "ruins", labelKey: "seedmap.structure.ruins", color: "#90a4ae" },
  { id: "treasure", labelKey: "seedmap.structure.treasure", color: "#ffb300" },
];

const STRUCTURE_COLOR = new Map(STRUCTURES.map((item) => [item.id, item.color]));

const DIMENSIONS: Array<{ value: SeedmapDimension; labelKey: string }> = [
  { value: 0, labelKey: "seedmap.overworld" },
  { value: 1, labelKey: "seedmap.nether" },
  { value: 2, labelKey: "seedmap.end" },
];

/** 后端只接受 1..64 的整数 scale（M12 真机回归后放宽到任意整数，上限仍是 64）。 */
const MAX_SEEDMAP_SCALE = 64;

interface TilePlan {
  tilesX: number;
  tilesY: number;
  /** 请求给后端的 scale：1 瓦片像素 = 多少方块，已夹在 1..64 的整数 */
  scale: number;
  x0: number;
  z0: number;
  /** 画布 1px 实际覆盖多少方块：scale 被夹住时这一屏盖不满视口，靠 drawImage 拉伸补齐 */
  viewBpp: number;
}

/**
 * 视口 → 请求参数。
 * 视口像素太多（1400×500 ≈ 70 万 > 512²）时不是缩小区块，而是让「一个瓦片像素覆盖 factor 个屏幕像素」，
 * 这样既能保住「1px ≈ N 方块」的档位，又不会让 tilesX * tilesY 爆量。
 */
function planTile(width: number, height: number, centerX: number, centerZ: number, blocksPerPx: number): TilePlan {
  const w = Math.max(1, Math.round(width));
  const h = Math.max(1, Math.round(height));
  let factor = 1;
  // 两个硬约束都要满足：单边 ≤ 512 张瓦片，且 tilesX * tilesY ≤ 512 * 512
  while (
    (Math.ceil(w / factor) > MAX_TILES || Math.ceil(h / factor) > MAX_TILES || Math.ceil(w / factor) * Math.ceil(h / factor) > TILE_BUDGET) &&
    factor < 64
  ) {
    factor += 1;
  }
  const tilesX = Math.min(MAX_TILES, Math.max(1, Math.ceil(w / factor)));
  const tilesY = Math.min(MAX_TILES, Math.max(1, Math.ceil(h / factor)));
  // 兜底：后端只吃 1..64 的整数，别把非法 scale 发出去（宁可退化成一屏内拉伸）
  const scale = Math.max(1, Math.min(MAX_SEEDMAP_SCALE, Math.round(blocksPerPx * factor)));
  return {
    tilesX,
    tilesY,
    scale,
    x0: Math.round(centerX - (tilesX * scale) / 2),
    z0: Math.round(centerZ - (tilesY * scale) / 2),
    viewBpp: Math.max(1 / 64, (tilesX * scale) / w),
  };
}

function readHistory(): string[] {
  try {
    const raw = window.localStorage.getItem(HISTORY_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is string => typeof item === "string" && item.length > 0).slice(0, HISTORY_MAX);
  } catch {
    return [];
  }
}

function writeHistory(list: string[]): void {
  try {
    window.localStorage.setItem(HISTORY_KEY, JSON.stringify(list.slice(0, HISTORY_MAX)));
  } catch {
    /* 存不进去也不影响本次使用 */
  }
}

/** 随机 i64：取 8 字节按二进制补码折成 BigInt，再转成十进制字符串（可能带负号）。 */
function randomSeed(): string {
  const bytes = new Uint8Array(8);
  window.crypto.getRandomValues(bytes);
  let value = 0n;
  for (const byte of bytes) value = (value << 8n) | BigInt(byte);
  return BigInt.asIntN(64, value).toString();
}

function clampIndex(value: number, max: number): number {
  return Math.max(0, Math.min(max, value));
}

function parseCoord(text: string, fallback: number): number {
  const value = Number(text.trim());
  return Number.isFinite(value) ? Math.round(value) : fallback;
}

function cssVar(name: string, fallback: string): string {
  const value = window.getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

/** 山体阴影用的平滑噪声（前端近似，后端不返回高度图）。 */
function noiseAt(x: number, z: number): number {
  const xi = Math.floor(x);
  const zi = Math.floor(z);
  const tx = x - xi;
  const tz = z - zi;
  const sx = tx * tx * (3 - 2 * tx);
  const sz = tz * tz * (3 - 2 * tz);
  const hash = (cx: number, cz: number) => {
    let h = Math.imul(cx | 0, 374761393) ^ Math.imul(cz | 0, 668265263);
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    h ^= h >>> 16;
    return (h >>> 0) / 4294967296;
  };
  const top = hash(xi, zi) + (hash(xi + 1, zi) - hash(xi, zi)) * sx;
  const bottom = hash(xi, zi + 1) + (hash(xi + 1, zi + 1) - hash(xi, zi + 1)) * sx;
  return top + (bottom - top) * sz;
}

/** 叠一层「阴影 / 高光」，透明度很低，只为了让平原不那么平。 */
function buildShadeLayer(tile: SeedmapTile): HTMLCanvasElement | null {
  const scale = Math.max(1, Math.round(Math.max(tile.width, tile.height) / 96));
  const width = Math.max(1, Math.floor(tile.width / scale));
  const height = Math.max(1, Math.floor(tile.height / scale));
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  const image = ctx.createImageData(width, height);
  const cellBlocks = Math.max(24, tile.scale * 6);
  for (let row = 0; row < height; row += 1) {
    for (let col = 0; col < width; col += 1) {
      const wx = (tile.originX + (col * scale + scale / 2) * tile.scale) / cellBlocks;
      const wz = (tile.originZ + (row * scale + scale / 2) * tile.scale) / cellBlocks;
      const relief = noiseAt(wx, wz) * 0.7 + noiseAt(wx * 2.7, wz * 2.7) * 0.3;
      const offset = (row * width + col) * 4;
      if (relief >= 0.5) {
        image.data[offset] = 255;
        image.data[offset + 1] = 255;
        image.data[offset + 2] = 255;
        image.data[offset + 3] = Math.round((relief - 0.5) * 190);
      } else {
        image.data[offset] = 0;
        image.data[offset + 1] = 0;
        image.data[offset + 2] = 0;
        image.data[offset + 3] = Math.round((0.5 - relief) * 210);
      }
    }
  }
  ctx.putImageData(image, 0, 0);
  let alphaSum = 0;
  for (let i = 3; i < image.data.length; i += 4) alphaSum += image.data[i];
  const host = window as unknown as Record<string, unknown>;
  host.__guguSeedmapShade = {
    width,
    height,
    samples: width * height,
    alphaMean: Math.round((alphaSum / Math.max(1, width * height)) * 10) / 10,
  };
  return canvas;
}

/** 高亮地形：目标群系留亮，其余像素压暗（遮罩贴回底图即可，别改底图本身的像素）。 */
function buildHighlightMask(tile: SeedmapTile, biomeId: number): HTMLCanvasElement | null {
  if (!tile.biomes) return null;
  const canvas = document.createElement("canvas");
  canvas.width = tile.width;
  canvas.height = tile.height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  const image = ctx.createImageData(tile.width, tile.height);
  for (let i = 0; i < tile.width * tile.height; i += 1) {
    const offset = i * 4;
    image.data[offset] = 0;
    image.data[offset + 1] = 0;
    image.data[offset + 2] = 0;
    image.data[offset + 3] = tile.biomes[i] === biomeId ? 0 : 155;
  }
  ctx.putImageData(image, 0, 0);
  return canvas;
}

interface Props {
  notify?: (message: string) => void;
}

export function SeedMapPage({ notify }: Props) {
  const t = useT();

  const [versions, setVersions] = useState<SeedmapVersion[]>([]);
  const [mc, setMc] = useState("");
  const [dimension, setDimension] = useState<SeedmapDimension>(0);
  const [seed, setSeed] = useState("1234567890123");
  const [history, setHistory] = useState<string[]>(readHistory);
  const [historyOpen, setHistoryOpen] = useState(false);

  const [center, setCenter] = useState({ x: 0, z: 0 });
  const [posX, setPosX] = useState("0");
  const [posZ, setPosZ] = useState("0");
  const [zoom, setZoom] = useState(2);

  const [tile, setTile] = useState<SeedmapTile | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);

  const [overlays, setOverlays] = useState({ shade: true, hover: true, chunks: false, structures: true });
  const [structureKind, setStructureKind] = useState("");
  const [selectedStructures, setSelectedStructures] = useState<string[]>([]);
  const [structurePoints, setStructurePoints] = useState<SeedmapStructure[]>([]);

  const [biomeQuery, setBiomeQuery] = useState("");
  const [highlight, setHighlight] = useState<{ id: number; name: string } | null>(null);

  const [viewSize, setViewSize] = useState({ w: 0, h: 0 });
  const [mouse, setMouse] = useState<{ cx: number; cz: number; wx: number; wz: number } | null>(null);
  const [hoverBiome, setHoverBiome] = useState<string | null>(null);
  const [dragOffset, setDragOffset] = useState({ dx: 0, dy: 0 });
  const [dragging, setDragging] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);

  const wrapRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const dragRef = useRef({ active: false, x: 0, y: 0, dx: 0, dy: 0 });
  const tileSeq = useRef(0);
  const structureSeq = useRef(0);
  const biomeCache = useRef(new Map<string, string>());
  const biomeTimer = useRef<number | null>(null);
  const biomeLastAt = useRef(0);
  const shadeCache = useRef<{ key: string; canvas: HTMLCanvasElement | null } | null>(null);
  const maskCache = useRef<{ key: string; canvas: HTMLCanvasElement | null } | null>(null);
  const historySeed = useRef("");

  // ladderBpp 是档位（1/2/4/8/16/32/64），viewBpp 是这一屏真正每像素多少方块：
  // 后端 scale 顶到 64 时 viewBpp 会小于档位，画布靠拉伸铺满，读数与坐标换算都以 viewBpp 为准。
  const ladderBpp = ZOOM_LEVELS[zoom];
  const plan = useMemo(
    () => planTile(viewSize.w, viewSize.h, center.x, center.z, ladderBpp),
    [viewSize.w, viewSize.h, center.x, center.z, ladderBpp],
  );
  const viewBpp = plan.viewBpp;
  const ready = seed.trim().length > 0 && mc.length > 0 && viewSize.w > 8 && viewSize.h > 8;

  // ------------------------------------------------------------ 版本列表
  useEffect(() => {
    let alive = true;
    seedmapApi
      .versions()
      .then((list) => {
        if (!alive) return;
        setVersions(list);
        // 后端按「从新到旧」给列表：默认永远选第一项；当前值不在列表里也回落到第一项
        setMc((prev) => {
          const first = list[0]?.mc ?? "";
          return prev && list.some((item) => item.mc === prev) ? prev : first;
        });
      })
      .catch((err: unknown) => {
        if (!alive) return;
        setError(seedmapErrorText(err));
      });
    return () => {
      alive = false;
    };
  }, []);

  // ------------------------------------------------------- 视口尺寸跟踪
  useEffect(() => {
    const element = wrapRef.current;
    if (!element) return;
    const apply = () => setViewSize({ w: element.clientWidth, h: element.clientHeight });
    apply();
    const observer = new ResizeObserver(apply);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // ------------------------------------------------------------ 瓦片请求
  useEffect(() => {
    if (!ready) return;
    const seq = ++tileSeq.current;
    setLoading(true);
    setError(null);
    const request = {
      mc,
      dimension,
      seed: seed.trim(),
      scale: plan.scale,
      x0: plan.x0,
      z0: plan.z0,
      tilesX: plan.tilesX,
      tilesY: plan.tilesY,
    };
    loadTile(request)
      .then((next) => {
        // 序列号对不上就是过期响应（快速拖动 / 连点缩放），直接丢弃
        if (seq !== tileSeq.current) return;
        setTile(next);
        setLoading(false);
        if (historySeed.current !== request.seed) {
          historySeed.current = request.seed;
          setHistory((prev) => {
            const list = [request.seed, ...prev.filter((item) => item !== request.seed)].slice(0, HISTORY_MAX);
            writeHistory(list);
            return list;
          });
        }
      })
      .catch((err: unknown) => {
        if (seq !== tileSeq.current) return;
        setLoading(false);
        setError(seedmapErrorText(err));
      });
  }, [ready, mc, dimension, seed, plan.scale, plan.x0, plan.z0, plan.tilesX, plan.tilesY, refreshKey]);

  // ------------------------------------------------------------ 结构请求
  useEffect(() => {
    if (!ready || !overlays.structures || selectedStructures.length === 0) {
      setStructurePoints([]);
      return;
    }
    const seq = ++structureSeq.current;
    const radius = Math.max(256, Math.round(Math.max(viewSize.w, viewSize.h) * viewBpp * 0.8));
    Promise.all(
      selectedStructures.map((structure) =>
        seedmapApi.structures({ mc, seed: seed.trim(), structure, x: center.x, z: center.z, radius }),
      ),
    )
      .then((lists) => {
        if (seq !== structureSeq.current) return;
        setStructurePoints(lists.flat());
      })
      .catch(() => {
        if (seq !== structureSeq.current) return;
        setStructurePoints([]);
      });
  }, [ready, overlays.structures, selectedStructures, mc, seed, center.x, center.z, viewBpp, viewSize.w, viewSize.h, refreshKey]);

  // ------------------------------------------------------- 画布尺寸同步
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    canvas.width = Math.max(1, Math.round(viewSize.w));
    canvas.height = Math.max(1, Math.round(viewSize.h));
  }, [viewSize.w, viewSize.h]);

  // ------------------------------------------------------------- 绘制
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const width = canvas.width;
    const height = canvas.height;
    if (width < 2 || height < 2) return;

    const bpp = viewBpp;
    const viewLeft = center.x - (width * bpp) / 2;
    const viewTop = center.z - (height * bpp) / 2;
    const offsetX = dragOffset.dx;
    const offsetY = dragOffset.dy;
    const toCanvasX = (worldX: number) => (worldX - viewLeft) / bpp + offsetX;
    const toCanvasY = (worldZ: number) => (worldZ - viewTop) / bpp + offsetY;

    const host = window as unknown as Record<string, unknown>;
    const previousDraw = (host.__guguSeedmapDraw as { count?: number } | undefined) ?? {};
    host.__guguSeedmapDraw = {
      count: (previousDraw.count ?? 0) + 1,
      shade: overlays.shade,
      chunks: overlays.chunks,
      structures: structurePoints.length,
      tile: tile ? tile.originX + "," + tile.originZ : null,
    };

    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = cssVar("--surface-muted", "#eef1f6");
    ctx.fillRect(0, 0, width, height);
    if (!tile) return;

    const destX = (tile.originX - viewLeft) / bpp + offsetX;
    const destY = (tile.originZ - viewTop) / bpp + offsetY;
    const destW = (tile.width * tile.scale) / bpp;
    const destH = (tile.height * tile.scale) / bpp;
    ctx.imageSmoothingEnabled = true;
    ctx.drawImage(tile.canvas, destX, destY, destW, destH);

    const tileKey = tile.originX + ":" + tile.originZ + ":" + tile.width + ":" + tile.height + ":" + tile.scale;

    if (overlays.shade) {
      if (shadeCache.current?.key !== tileKey) {
        shadeCache.current = { key: tileKey, canvas: buildShadeLayer(tile) };
      }
      const shade = shadeCache.current.canvas;
      if (shade) ctx.drawImage(shade, destX, destY, destW, destH);
    }

    if (highlight) {
      const maskKey = tileKey + ":" + highlight.id;
      if (maskCache.current?.key !== maskKey) {
        maskCache.current = { key: maskKey, canvas: buildHighlightMask(tile, highlight.id) };
      }
      const mask = maskCache.current.canvas;
      if (mask) ctx.drawImage(mask, destX, destY, destW, destH);
    }

    if (overlays.chunks) {
      // 区块 = 16×16 方块；缩得太小时改画 16 个区块一格的粗网格，免得糊成一片
      let stepBlocks = 16;
      while (stepBlocks / bpp < 7) stepBlocks *= 16;
      ctx.save();
      ctx.strokeStyle = "rgba(255,255,255,0.34)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      const firstX = Math.ceil(viewLeft / stepBlocks) * stepBlocks;
      for (let worldX = firstX; toCanvasX(worldX) <= width; worldX += stepBlocks) {
        const x = Math.round(toCanvasX(worldX)) + 0.5;
        ctx.moveTo(x, 0);
        ctx.lineTo(x, height);
      }
      const firstZ = Math.ceil(viewTop / stepBlocks) * stepBlocks;
      for (let worldZ = firstZ; toCanvasY(worldZ) <= height; worldZ += stepBlocks) {
        const y = Math.round(toCanvasY(worldZ)) + 0.5;
        ctx.moveTo(0, y);
        ctx.lineTo(width, y);
      }
      ctx.stroke();
      ctx.restore();
    }

    if (overlays.structures && structurePoints.length > 0) {
      ctx.save();
      ctx.font = "11px 'Microsoft YaHei', system-ui, sans-serif";
      ctx.textBaseline = "middle";
      for (const point of structurePoints) {
        const x = toCanvasX(point.x);
        const y = toCanvasY(point.z);
        if (x < -24 || y < -24 || x > width + 24 || y > height + 24) continue;
        const color = STRUCTURE_COLOR.get(point.kind) ?? "#f5c845";
        ctx.beginPath();
        ctx.arc(x, y, 5, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        ctx.lineWidth = 2;
        ctx.strokeStyle = "rgba(0,0,0,0.55)";
        ctx.stroke();
        const label = t(STRUCTURES.find((item) => item.id === point.kind)?.labelKey ?? "seedmap.structures");
        ctx.fillStyle = "rgba(0,0,0,0.62)";
        ctx.fillText(label, x + 9, y + 1);
        ctx.fillStyle = "#ffffff";
        ctx.fillText(label, x + 8, y);
      }
      ctx.restore();
    }

    if (mouse && !dragging) {
      ctx.save();
      ctx.strokeStyle = "rgba(255,255,255,0.9)";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(mouse.cx, mouse.cz, 6, 0, Math.PI * 2);
      ctx.stroke();
      ctx.strokeStyle = "rgba(0,0,0,0.45)";
      ctx.beginPath();
      ctx.moveTo(mouse.cx - 11, mouse.cz);
      ctx.lineTo(mouse.cx - 8, mouse.cz);
      ctx.moveTo(mouse.cx + 8, mouse.cz);
      ctx.lineTo(mouse.cx + 11, mouse.cz);
      ctx.moveTo(mouse.cx, mouse.cz - 11);
      ctx.lineTo(mouse.cx, mouse.cz - 8);
      ctx.moveTo(mouse.cx, mouse.cz + 8);
      ctx.lineTo(mouse.cx, mouse.cz + 11);
      ctx.stroke();
      ctx.restore();
    }
  }, [
    tile,
    center.x,
    center.z,
    viewBpp,
    overlays.shade,
    overlays.chunks,
    overlays.structures,
    highlight,
    structurePoints,
    dragOffset.dx,
    dragOffset.dy,
    dragging,
    mouse,
    t,
  ]);

  // ------------------------------------------------------- 滚轮缩放（非 passive）
  useEffect(() => {
    const element = wrapRef.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const direction = event.deltaY > 0 ? 1 : -1;
      setZoom((prev) => clampIndex(prev + direction, ZOOM_LEVELS.length - 1));
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, []);

  // --------------------------------------------------------- 全屏状态跟踪
  useEffect(() => {
    const onFullscreenChange = () => setFullscreen(document.fullscreenElement === wrapRef.current);
    document.addEventListener("fullscreenchange", onFullscreenChange);
    return () => document.removeEventListener("fullscreenchange", onFullscreenChange);
  }, []);

  useEffect(
    () => () => {
      if (biomeTimer.current !== null) window.clearTimeout(biomeTimer.current);
    },
    [],
  );

  // ------------------------------------------------------------ 交互动作
  const jumpTo = useCallback((x: number, z: number) => {
    const nx = Math.round(Number.isFinite(x) ? x : 0);
    const nz = Math.round(Number.isFinite(z) ? z : 0);
    setCenter({ x: nx, z: nz });
    setPosX(String(nx));
    setPosZ(String(nz));
    setHistoryOpen(false);
  }, []);

  const applySeed = useCallback((value: string) => {
    setSeed(value);
    setHistoryOpen(false);
    jumpTo(0, 0);
  }, [jumpTo]);

  const lookupBiome = useCallback(
    (x: number, z: number) => {
      const key = x + "," + z;
      const cached = biomeCache.current.get(key);
      if (cached) {
        setHoverBiome(cached);
        return;
      }
      if (biomeTimer.current !== null) window.clearTimeout(biomeTimer.current);
      const wait = Math.max(0, BIOME_THROTTLE_MS - (Date.now() - biomeLastAt.current));
      biomeTimer.current = window.setTimeout(() => {
        biomeLastAt.current = Date.now();
        void seedmapApi
          .biomeAt({ mc, dimension, seed: seed.trim(), x, z })
          .then((result) => {
            const name = result.biomeName || biomeName(result.biomeId);
            if (biomeCache.current.size > 600) biomeCache.current.clear();
            biomeCache.current.set(key, name);
            setHoverBiome(name);
          })
          .catch(() => setHoverBiome(null));
      }, wait);
    },
    [mc, dimension, seed],
  );

  const onCanvasMouseDown = (event: React.MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    dragRef.current = { active: true, x: event.clientX, y: event.clientY, dx: 0, dy: 0 };
    setDragging(true);
    event.preventDefault();
  };

  const onCanvasMouseMove = (event: React.MouseEvent<HTMLDivElement>) => {
    const element = wrapRef.current;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const cx = event.clientX - rect.left;
    const cz = event.clientY - rect.top;
    if (dragRef.current.active) {
      const dx = event.clientX - dragRef.current.x;
      const dy = event.clientY - dragRef.current.y;
      dragRef.current.dx = dx;
      dragRef.current.dy = dy;
      setDragOffset({ dx, dy });
      return;
    }
    const wx = Math.round(center.x + (cx - rect.width / 2) * viewBpp);
    const wz = Math.round(center.z + (cz - rect.height / 2) * viewBpp);
    setMouse({ cx, cz, wx, wz });
    if (overlays.hover) lookupBiome(wx, wz);
  };

  const endDrag = () => {
    if (!dragRef.current.active) return;
    const { dx, dy } = dragRef.current;
    dragRef.current.active = false;
    setDragging(false);
    setDragOffset({ dx: 0, dy: 0 });
    // 平移去抖：拖动过程中只用位移显示旧瓦片，松手才请求新的一屏
    if (Math.abs(dx) + Math.abs(dy) >= 3) {
      jumpTo(center.x - dx * viewBpp, center.z - dy * viewBpp);
    }
  };

  const toggleFullscreen = useCallback(() => {
    const element = wrapRef.current;
    if (!element) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else void element.requestFullscreen().catch(() => undefined);
  }, []);

  const copyTp = () => {
    const x = parseCoord(posX, center.x);
    const z = parseCoord(posZ, center.z);
    void copyText("tp @s " + x + " ~ " + z).then((ok) => {
      notify?.(ok ? t("seedmap.tpCopied", { x, z }) : t("seedmap.tpFailed"));
    });
  };

  const fetchSpawn = () => {
    void seedmapApi
      .spawn({ mc, seed: seed.trim() })
      .then((result) => {
        jumpTo(result.x, result.z);
        notify?.(t("seedmap.spawnDone", { x: result.x, z: result.z }));
      })
      .catch((err: unknown) => notify?.(seedmapErrorText(err)));
  };

  const biomeMatches = useMemo(() => searchBiomes(biomeQuery), [biomeQuery]);

  const addStructure = (id: string) => {
    setSelectedStructures((prev) => (prev.includes(id) ? prev : [...prev, id]));
  };

  const scaleLabel = t("seedmap.scale", { n: Math.max(1, Math.round(viewBpp)) });
  const coords = mouse
    ? t("seedmap.coords", { x: mouse.wx, z: mouse.wz })
    : t("seedmap.coordsEmpty");

  return (
    <div className="seedmap-page" data-seedmap-root="seedmap">
      <div className="seedmap-bar">
        <label className="seedmap-field seedmap-field--seed">
          <span>{t("seedmap.seed")}</span>
          <input
            data-seedmap="seed"
            className="mono seedmap-seed"
            value={seed}
            spellCheck={false}
            onChange={(event) => setSeed(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") jumpTo(0, 0);
            }}
          />
        </label>
        <button type="button" data-seedmap="random" onClick={() => applySeed(randomSeed())}>
          {t("seedmap.random")}
        </button>
        <div className="seedmap-history">
          <button
            type="button"
            data-seedmap="history"
            aria-expanded={historyOpen}
            onClick={() => setHistoryOpen((open) => !open)}
          >
            {t("seedmap.history", { count: history.length })}
          </button>
          {historyOpen ? (
            <div className="seedmap-menu" data-seedmap="history-menu">
              {history.length === 0 ? (
                <p className="seedmap-menu__empty">{t("seedmap.historyEmpty")}</p>
              ) : (
                history.map((item) => (
                  <button key={item} type="button" className="seedmap-menu__item mono" onClick={() => applySeed(item)}>
                    {item}
                  </button>
                ))
              )}
            </div>
          ) : null}
        </div>
        <button type="button" data-seedmap="import" disabled title={t("seedmap.importHint")}>
          {t("seedmap.import")}
        </button>

        <span className="seedmap-bar__spacer" />

        <label className="seedmap-field">
          <span>{t("seedmap.version")}</span>
          <select data-seedmap="mc" value={mc} onChange={(event) => setMc(event.target.value)}>
            {versions.map((item) => (
              <option key={item.mc} value={item.mc}>
                {item.label}
              </option>
            ))}
          </select>
        </label>
        <label className="seedmap-field">
          <span>{t("seedmap.edition")}</span>
          <select data-seedmap="edition" value="java" onChange={() => undefined}>
            <option value="java">{t("seedmap.editionJava")}</option>
          </select>
        </label>
        <label className="seedmap-field">
          <span>{t("seedmap.dimension")}</span>
          <select
            data-seedmap="dimension"
            value={String(dimension)}
            onChange={(event) => setDimension(Number(event.target.value) === 1 ? 1 : Number(event.target.value) === 2 ? 2 : 0)}
          >
            {DIMENSIONS.map((item) => (
              <option key={item.value} value={String(item.value)}>
                {t(item.labelKey)}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="seedmap-bar seedmap-bar--sub">
        <label className="seedmap-field seedmap-field--coord">
          <span>X</span>
          <input data-seedmap="x" className="mono" value={posX} onChange={(event) => setPosX(event.target.value)} />
        </label>
        <label className="seedmap-field seedmap-field--coord">
          <span>Z</span>
          <input data-seedmap="z" className="mono" value={posZ} onChange={(event) => setPosZ(event.target.value)} />
        </label>
        <button type="button" data-seedmap="goto" onClick={() => jumpTo(parseCoord(posX, center.x), parseCoord(posZ, center.z))}>
          {t("seedmap.goto")}
        </button>
        <button type="button" data-seedmap="tp" onClick={copyTp}>
          {t("seedmap.tp")}
        </button>
        <button type="button" data-seedmap="spawn" onClick={fetchSpawn}>
          {t("seedmap.spawn")}
        </button>
        <button type="button" data-seedmap="refresh" onClick={() => setRefreshKey((key) => key + 1)}>
          {t("seedmap.refresh")}
        </button>

        <span className="seedmap-bar__spacer" />
        <span className="seedmap-scale" data-seedmap="scale">
          {scaleLabel}
        </span>
      </div>

      <div
        className={"seedmap-map" + (dragging ? " is-dragging" : "")}
        ref={wrapRef}
        data-seedmap-map
        data-seedmap-chunks={overlays.chunks ? "on" : "off"}
        data-seedmap-shade={overlays.shade ? "on" : "off"}
        onMouseDown={onCanvasMouseDown}
        onMouseMove={onCanvasMouseMove}
        onMouseUp={endDrag}
        onMouseLeave={() => {
          endDrag();
          setMouse(null);
          setHoverBiome(null);
        }}
      >
        <canvas className="seedmap-canvas" ref={canvasRef} data-seedmap="canvas" />

        <div className="seedmap-panel" data-seedmap="panel">
          <div className="seedmap-panel__title">{t("seedmap.layers")}</div>
          <label className="seedmap-check">
            <input
              type="checkbox"
              data-seedmap="layer-shade"
              checked={overlays.shade}
              onChange={(event) => setOverlays((prev) => ({ ...prev, shade: event.target.checked }))}
            />
            <span>{t("seedmap.layerShade")}</span>
          </label>
          <label className="seedmap-check">
            <input
              type="checkbox"
              data-seedmap="layer-hover"
              checked={overlays.hover}
              onChange={(event) => setOverlays((prev) => ({ ...prev, hover: event.target.checked }))}
            />
            <span>{t("seedmap.layerHover")}</span>
          </label>
          <label className="seedmap-check">
            <input
              type="checkbox"
              data-seedmap="layer-chunks"
              checked={overlays.chunks}
              onChange={(event) => setOverlays((prev) => ({ ...prev, chunks: event.target.checked }))}
            />
            <span>{t("seedmap.layerChunks")}</span>
          </label>
          <label className="seedmap-check">
            <input
              type="checkbox"
              data-seedmap="layer-structures"
              checked={overlays.structures}
              onChange={(event) => setOverlays((prev) => ({ ...prev, structures: event.target.checked }))}
            />
            <span>{t("seedmap.layerStructures")}</span>
          </label>

          <div className="seedmap-panel__divider" />

          <div className="seedmap-panel__sub">{t("seedmap.structures")}</div>
          <select
            className="seedmap-panel__select"
            data-seedmap="structure-select"
            value={structureKind}
            onChange={(event) => {
              const value = event.target.value;
              setStructureKind("");
              if (value) addStructure(value);
            }}
          >
            <option value="">{t("seedmap.structureAdd")}</option>
            {STRUCTURES.map((item) => (
              <option key={item.id} value={item.id}>
                {t(item.labelKey)}
              </option>
            ))}
          </select>
          <div className="seedmap-chips" data-seedmap="structure-chips">
            {selectedStructures.length === 0 ? (
              <span className="seedmap-chips__empty">{t("seedmap.structureEmpty")}</span>
            ) : (
              selectedStructures.map((id) => (
                <button
                  key={id}
                  type="button"
                  className="chip chip--button is-active"
                  data-seedmap-chip={id}
                  onClick={() => setSelectedStructures((prev) => prev.filter((item) => item !== id))}
                >
                  {t(STRUCTURES.find((item) => item.id === id)?.labelKey ?? "seedmap.structures")}
                  <span className="seedmap-chip__x">×</span>
                </button>
              ))
            )}
            {selectedStructures.length > 0 ? (
              <button type="button" className="link-button" data-seedmap="structure-clear" onClick={() => setSelectedStructures([])}>
                {t("seedmap.clear")}
              </button>
            ) : null}
          </div>

          <div className="seedmap-panel__divider" />

          <div className="seedmap-panel__sub">{t("seedmap.highlight")}</div>
          <input
            data-seedmap="biome-search"
            className="seedmap-panel__search"
            placeholder={t("seedmap.highlightHint")}
            value={biomeQuery}
            onChange={(event) => setBiomeQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && biomeMatches.length > 0) {
                setHighlight({ id: biomeMatches[0].id, name: biomeMatches[0].name });
              }
              if (event.key === "Escape") setBiomeQuery("");
            }}
          />
          {biomeMatches.length > 0 ? (
            <div className="seedmap-menu seedmap-menu--inline" data-seedmap="biome-list">
              {biomeMatches.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className="seedmap-menu__item"
                  data-seedmap-biome={item.id}
                  onClick={() => {
                    setHighlight({ id: item.id, name: item.name });
                    setBiomeQuery("");
                  }}
                >
                  <span className="seedmap-dot" style={{ background: item.color }} />
                  {item.name}
                </button>
              ))}
            </div>
          ) : null}
          {highlight ? (
            <div className="seedmap-highlight" data-seedmap="highlight-current">
              <span className="seedmap-dot" style={{ background: "#f5c845" }} />
              {t("seedmap.highlightOn", { name: highlight.name })}
              <button
                type="button"
                className="link-button"
                data-seedmap="highlight-clear"
                onClick={() => {
                  setHighlight(null);
                  setBiomeQuery("");
                }}
              >
                {t("seedmap.clear")}
              </button>
            </div>
          ) : null}
        </div>

        <div className="seedmap-coords mono" data-seedmap="coords">
          {coords}
        </div>

        <div className="seedmap-zoom">
          <button
            type="button"
            title={t("seedmap.zoomIn")}
            data-seedmap="zoom-in"
            onClick={() => setZoom((prev) => clampIndex(prev - 1, ZOOM_LEVELS.length - 1))}
          >
            +
          </button>
          <button
            type="button"
            title={t("seedmap.zoomOut")}
            data-seedmap="zoom-out"
            onClick={() => setZoom((prev) => clampIndex(prev + 1, ZOOM_LEVELS.length - 1))}
          >
            −
          </button>
          <button type="button" title={t("seedmap.fullscreen")} data-seedmap="fullscreen" onClick={toggleFullscreen}>
            {fullscreen ? "⤡" : "⤢"}
          </button>
        </div>

        {overlays.hover && mouse && !dragging ? (
          <div
            className="seedmap-tooltip"
            data-seedmap="tooltip"
            style={{
              left: Math.min(mouse.cx + 14, Math.max(8, viewSize.w - 190)),
              top: Math.min(mouse.cz + 16, Math.max(8, viewSize.h - 44)),
            }}
          >
            <strong>{hoverBiome ?? t("seedmap.biomePending")}</strong>
            <span className="mono">
              (x: {mouse.wx}, z: {mouse.wz})
            </span>
          </div>
        ) : null}

        {loading ? (
          <div className="seedmap-status" data-seedmap="loading">
            <span className="seedmap-skeleton" />
            <p>{t("seedmap.loading")}</p>
          </div>
        ) : null}

        {error && !loading ? (
          <div className="seedmap-status seedmap-status--error" data-seedmap="error">
            <p>{error}</p>
            <button type="button" data-seedmap="retry" onClick={() => setRefreshKey((key) => key + 1)}>
              {t("seedmap.retry")}
            </button>
          </div>
        ) : null}
      </div>

      <p className="seedmap-foot" data-seedmap="foot">
        {t("seedmap.foot")}
      </p>
    </div>
  );
}
