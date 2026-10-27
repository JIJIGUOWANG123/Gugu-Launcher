/**
 * 内容服务页的筛选数据：按内容类型给出不同的筛选项，并把中文标签映射到 Modrinth 类别名。
 * 标签是界面文案（工程只做简体中文），ids 才是发给后端的值；「全部xxx」用空 ids 表示不筛选。
 */
import type { ContentKindTab } from "./types";

/** 三个「全部」胶囊：选中它等于清空该组。 */
export const ALL_VERSIONS = "全部版本";
export const ALL_LOADERS = "全部加载器";
export const ALL_CATEGORIES = "全部分类";

export interface FilterOption {
  label: string;
  /** 发给后端的类别 id；空数组＝不筛选 */
  ids: string[];
}

export interface KindFilterSpec {
  /** 加载器组；null 表示这个类型没有加载器筛选（资源包 / 数据包） */
  loaders: FilterOption[] | null;
  categories: FilterOption[];
}

/** 中文标签 → Modrinth 类别名。一个标签可能对应多个 id（例如「库·API」）。 */
const CATEGORY_IDS: Record<string, string[]> = {
  Fabric: ["fabric"],
  Forge: ["forge"],
  Quilt: ["quilt"],
  NeoForge: ["neoforge"],
  "优化": ["optimization"],
  "库·API": ["library", "api"],
  "实用": ["utility"],
  "冒险": ["adventure"],
  "魔法": ["magic"],
  "科技": ["technology"],
  "装饰": ["decoration"],
  "装备": ["equipment"],
  "食物": ["food"],
  "杂项": ["misc"],
  "生物": ["mobs"],
  "存储": ["storage"],
  "世界生成": ["worldgen"],
  "16x": ["16x"],
  "32x": ["32x"],
  "64x": ["64x"],
  "128x": ["128x"],
  "256x": ["256x"],
  "512x": ["512x"],
  Faithful: ["faithful"],
  Cursed: ["cursed"],
  "现代": ["modern"],
  "半写实": ["semi-realistic"],
  "简约": ["simplistic"],
  "主题": ["themed"],
  "挑战": ["challenge"],
  "战斗": ["combat"],
  "硬核": ["hardcore"],
  "小游戏": ["minigame"],
  "多人联机": ["multiplayer"],
  PvP: ["pvp"],
  "原版增强": ["vanilla-like"],
  "经济": ["economy"],
  "写实": ["realistic"],
  "幻想": ["fantasy"],
  "卡通": ["cartoon"],
  "恐怖": ["horror"],
};

/** 中文标签 → 类别 id 列表；没登记的标签按「英文小写 + 连字符」兜底，不丢信息。 */
export function labelToCategoryId(label: string): string[] {
  const key = label.trim();
  if (!key) return [];
  const mapped = CATEGORY_IDS[key];
  if (mapped) return mapped;
  return [key.toLowerCase().replace(/\s+/g, "-")];
}

const loaderOptions = (loaders: string[] = ["Fabric", "Forge", "NeoForge", "Quilt"]): FilterOption[] => [
  { label: ALL_LOADERS, ids: [] },
  ...loaders.map((label) => ({ label, ids: labelToCategoryId(label) })),
];

/** 插件的服务端平台：和模组的加载器不是同一批。 */
const PLUGIN_LOADERS = ["Bukkit", "Spigot", "Paper", "Folia", "Purpur", "Sponge", "Pufferfish"];

const categoryOptions = (labels: string[]): FilterOption[] => [
  { label: ALL_CATEGORIES, ids: [] },
  ...labels.map((label) => ({ label, ids: labelToCategoryId(label) })),
];

const MOD_CATEGORIES = [
  "Fabric",
  "Forge",
  "Quilt",
  "NeoForge",
  "优化",
  "库·API",
  "实用",
  "冒险",
  "魔法",
  "科技",
  "装饰",
  "装备",
  "食物",
  "杂项",
  "生物",
  "存储",
  "世界生成",
];

const RESOURCEPACK_CATEGORIES = [
  "16x",
  "32x",
  "64x",
  "128x",
  "256x",
  "512x",
  "Faithful",
  "Cursed",
  "现代",
  "半写实",
  "简约",
  "主题",
];

const MODPACK_CATEGORIES = [
  "Fabric",
  "Forge",
  "Quilt",
  "NeoForge",
  "冒险",
  "挑战",
  "战斗",
  "硬核",
  "魔法",
  "小游戏",
  "多人联机",
  "优化",
  "PvP",
  "科技",
  "原版增强",
];

const DATAPACK_CATEGORIES = ["冒险", "挑战", "装饰", "魔法", "小游戏", "生物", "优化", "科技", "实用", "世界生成"];

const SHADER_CATEGORIES = ["优化", "写实", "幻想", "卡通", "恐怖", "半写实", "主题"];

const PLUGIN_CATEGORIES = ["冒险", "挑战", "装饰", "魔法", "小游戏", "优化", "科技", "实用", "世界生成", "装备", "食物", "经济", "生物"];

/** 五种内容类型各自的筛选组（照参照图逐项对应）。 */
export const FILTERS_BY_KIND: Record<ContentKindTab, KindFilterSpec> = {
  mod: { loaders: loaderOptions(), categories: categoryOptions(MOD_CATEGORIES) },
  modpack: { loaders: loaderOptions(), categories: categoryOptions(MODPACK_CATEGORIES) },
  resourcepack: { loaders: null, categories: categoryOptions(RESOURCEPACK_CATEGORIES) },
  datapack: { loaders: null, categories: categoryOptions(DATAPACK_CATEGORIES) },
  shader: { loaders: loaderOptions(), categories: categoryOptions(SHADER_CATEGORIES) },
  plugin: { loaders: loaderOptions(PLUGIN_LOADERS), categories: categoryOptions(PLUGIN_CATEGORIES) },
};
