/**
 * 内置实例图标（21 张）：素材文件名已改成 ASCII id，避免打包/路径编码问题。
 * 内置资源不可删除，只能「改用别的图标」。
 */
import icon0 from "./bedrock.png";
import icon1 from "./bookshelf.png";
import icon2 from "./cherry-log.png";
import icon3 from "./command-block.png";
import icon4 from "./diamond-block.png";
import icon5 from "./emerald-block.png";
import icon6 from "./glowstone.png";
import icon7 from "./gold-block.png";
import icon8 from "./grass-block.png";
import icon9 from "./honey-block.png";
import icon10 from "./iron-block.png";
import icon11 from "./jack-o-lantern.png";
import icon12 from "./lapis-block.png";
import icon13 from "./netherite-block.png";
import icon14 from "./oak-log.png";
import icon15 from "./obsidian.png";
import icon16 from "./pumpkin.png";
import icon17 from "./quartz-block.png";
import icon18 from "./redstone-block.png";
import icon19 from "./sponge.png";
import icon20 from "./target.png";

export interface BuiltinInstanceIcon {
  id: string;
  label: string;
  url: string;
}

export const BUILTIN_INSTANCE_ICONS: BuiltinInstanceIcon[] = [
  { id: "bedrock", label: "基岩", url: icon0 },
  { id: "bookshelf", label: "书架", url: icon1 },
  { id: "cherry-log", label: "樱花木", url: icon2 },
  { id: "command-block", label: "命令方块", url: icon3 },
  { id: "diamond-block", label: "钻石块", url: icon4 },
  { id: "emerald-block", label: "绿宝石块", url: icon5 },
  { id: "glowstone", label: "萤火石", url: icon6 },
  { id: "gold-block", label: "黄金块", url: icon7 },
  { id: "grass-block", label: "草方块", url: icon8 },
  { id: "honey-block", label: "蜂蜜块", url: icon9 },
  { id: "iron-block", label: "铁块", url: icon10 },
  { id: "jack-o-lantern", label: "南瓜灯", url: icon11 },
  { id: "lapis-block", label: "青金石块", url: icon12 },
  { id: "netherite-block", label: "下界合金块", url: icon13 },
  { id: "oak-log", label: "橡木原木", url: icon14 },
  { id: "obsidian", label: "黑曜石", url: icon15 },
  { id: "pumpkin", label: "南瓜", url: icon16 },
  { id: "quartz-block", label: "石英块", url: icon17 },
  { id: "redstone-block", label: "红石块", url: icon18 },
  { id: "sponge", label: "海绵", url: icon19 },
  { id: "target", label: "标靶", url: icon20 },
];

/** 按 id 找内置图标；找不到返回 null（后端给了未知 id 也不崩）。 */
export function builtinIconById(id: string | undefined): BuiltinInstanceIcon | null {
  if (!id) return null;
  return BUILTIN_INSTANCE_ICONS.find((item) => item.id === id) ?? null;
}
