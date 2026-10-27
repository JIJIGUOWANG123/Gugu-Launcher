/**
 * 群系色板：biomeId → 颜色 + 中文名（M12 种子地图）。
 *
 * biomeId 沿用 cubiomes 的枚举值（等于游戏内数字 ID），后端 seedmap_biome_at 回的就是这一套。
 * 没登记的 id 一律回落到「灰绿」，所以哪怕后端加了新群系、前端还没跟上，地图也不会画成透明块。
 * 页面里所有上色都从这里取，别在页面里写死颜色。
 */

export interface BiomeEntry {
  id: number;
  /** 中文名：地图 tooltip、高亮地形搜索框都用它 */
  name: string;
  /** 地图上的填充色（接近原版地图配色，压过一点饱和度，叠阴影后仍然分得清） */
  color: string;
}

/** 未登记群系的兜底色（灰绿）。 */
export const BIOME_FALLBACK_COLOR = "#7d8f6a";

/** 主世界 / 下界 / 末地 三套常用群系。 */
export const BIOMES: BiomeEntry[] = [
  // ---- 主世界：海洋系 ----
  { id: 0, name: "海洋", color: "#2f4f8f" },
  { id: 24, name: "深海", color: "#1f3a6e" },
  { id: 10, name: "冻洋", color: "#4f7fb0" },
  { id: 44, name: "暖水海洋", color: "#2f8fbd" },
  { id: 45, name: "温水海洋", color: "#3a86bb" },
  { id: 46, name: "冷水海洋", color: "#37628f" },
  { id: 47, name: "暖水深海", color: "#27719b" },
  { id: 48, name: "温水深海", color: "#2f6c96" },
  { id: 50, name: "冷水深海", color: "#2a5279" },
  { id: 7, name: "河流", color: "#3c6cb0" },
  { id: 11, name: "冻河", color: "#5b86bf" },
  { id: 25, name: "石岸", color: "#8d8d8d" },
  { id: 16, name: "沙滩", color: "#e8dfa8" },
  { id: 26, name: "雪沙滩", color: "#eef3f6" },

  // ---- 主世界：温带 ----
  { id: 1, name: "平原", color: "#7fb238" },
  { id: 129, name: "向日葵平原", color: "#a3c74a" },
  { id: 4, name: "森林", color: "#3f7a20" },
  { id: 18, name: "森林丘陵", color: "#376d1c" },
  { id: 27, name: "白桦森林", color: "#6fa049" },
  { id: 28, name: "白桦森林丘陵", color: "#628f41" },
  { id: 29, name: "黑森林", color: "#1f4a1c" },
  { id: 131, name: "繁花森林", color: "#4f9e3b" },
  { id: 6, name: "沼泽", color: "#4f5b3a" },
  { id: 5, name: "针叶林", color: "#2f6b4f" },
  { id: 19, name: "针叶林丘陵", color: "#2a5f47" },
  { id: 32, name: "巨型针叶林", color: "#2b5c3a" },
  { id: 33, name: "巨型针叶林丘陵", color: "#265234" },
  { id: 3, name: "山地", color: "#9aa0a6" },
  { id: 34, name: "山地边缘", color: "#8a9096" },
  { id: 35, name: "热带草原", color: "#bfb755" },
  { id: 36, name: "热带高原", color: "#c8c06a" },
  { id: 21, name: "丛林", color: "#2f9b1e" },
  { id: 22, name: "丛林丘陵", color: "#278814" },
  { id: 23, name: "丛林边缘", color: "#45a92f" },
  { id: 2, name: "沙漠", color: "#f0d78a" },
  { id: 17, name: "沙漠丘陵", color: "#e5c983" },
  { id: 37, name: "恶地", color: "#c4622d" },
  { id: 38, name: "恶地高原", color: "#a8541f" },
  { id: 39, name: "破碎恶地", color: "#b5642c" },
  { id: 14, name: "蘑菇岛", color: "#b48a8a" },
  { id: 15, name: "蘑菇岛岸", color: "#a58787" },

  // ---- 主世界：寒带 / 山地变种 ----
  { id: 12, name: "雪原", color: "#f2f7fb" },
  { id: 13, name: "雪山", color: "#e6eef5" },
  { id: 30, name: "积雪针叶林", color: "#345b52" },
  { id: 31, name: "积雪针叶林丘陵", color: "#2f5349" },
  { id: 140, name: "冰刺之地", color: "#dbe7ef" },
  { id: 177, name: "草甸", color: "#8fbf5a" },
  { id: 178, name: "雪林", color: "#6f8f7f" },
  { id: 179, name: "积雪山坡", color: "#eaf1f6" },
  { id: 180, name: "尖峭山峰", color: "#c9d2d8" },
  { id: 181, name: "冰封山峰", color: "#dfe8ee" },
  { id: 182, name: "积石山峰", color: "#8d9298" },
  { id: 183, name: "樱花树林", color: "#e8a5bd" },
  { id: 185, name: "樱花树林", color: "#e8a5bd" },
  { id: 155, name: "繁茂洞穴", color: "#5f7a3f" },

  // ---- 下界 ----
  { id: 8, name: "下界荒地", color: "#b02e26" },
  { id: 170, name: "灵魂沙峡谷", color: "#5a4a55" },
  { id: 171, name: "绯红森林", color: "#8f2b2b" },
  { id: 172, name: "诡异森林", color: "#1f6f6a" },
  { id: 173, name: "玄武岩三角洲", color: "#5f5a5f" },

  // ---- 末地 ----
  { id: 9, name: "末地", color: "#dcd3a0" },
  { id: 40, name: "末地高地", color: "#d8d0a4" },
  { id: 41, name: "末地内陆", color: "#ded7b4" },
  { id: 42, name: "末地小岛", color: "#cfc79c" },
  { id: 43, name: "末地荒地", color: "#d3cb9f" },
];

const COLOR_BY_ID = new Map<number, string>(BIOMES.map((biome) => [biome.id, biome.color]));
const NAME_BY_ID = new Map<number, string>(BIOMES.map((biome) => [biome.id, biome.name]));

/** #rrggbb / #rgb → [r, g, b]；解析失败回落到兜底色。 */
const rgbCache = new Map<string, [number, number, number]>();

function parseHex(color: string): [number, number, number] {
  const hit = rgbCache.get(color);
  if (hit) return hit;
  let hex = color.replace("#", "");
  if (hex.length === 3) hex = hex[0] + hex[0] + hex[1] + hex[1] + hex[2] + hex[2];
  const value = Number.parseInt(hex, 16);
  const rgb: [number, number, number] = Number.isFinite(value)
    ? [(value >> 16) & 255, (value >> 8) & 255, value & 255]
    : [125, 143, 106];
  rgbCache.set(color, rgb);
  return rgb;
}

/** 未登记 id → 灰绿。 */
export function biomeColor(id: number): string {
  return COLOR_BY_ID.get(id) ?? BIOME_FALLBACK_COLOR;
}

/** 未登记 id → 「群系 #id」，至少 tooltip 不会空着。 */
export function biomeName(id: number): string {
  return NAME_BY_ID.get(id) ?? "群系 " + id;
}

/** 直接给 ImageData 写像素用（避免每帧解析十六进制）。 */
export function biomeRgb(id: number): [number, number, number] {
  return parseHex(biomeColor(id));
}

/**
 * 高亮地形搜索：按中文名/英文 id/数字 id 模糊匹配。
 * 名字里带关键词就算命中，最多返回 limit 条（默认 8），够下拉列表用。
 */
export function searchBiomes(keyword: string, limit = 8): BiomeEntry[] {
  const query = keyword.trim().toLowerCase();
  if (!query) return [];
  const out: BiomeEntry[] = [];
  for (const biome of BIOMES) {
    if (biome.name.toLowerCase().includes(query) || String(biome.id) === query) {
      out.push(biome);
      if (out.length >= limit) break;
    }
  }
  return out;
}
