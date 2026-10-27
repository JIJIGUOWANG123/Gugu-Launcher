import { builtinSkins } from "../assets/skins";
import type { SkinItem } from "./types";
import steveSkin from "../assets/skins/steve-wide.png";
import alexSkin from "../assets/skins/alex-slim.png";
import ariSkin from "../assets/skins/ari-wide.png";
import efeSkin from "../assets/skins/efe-slim.png";
import kaiSkin from "../assets/skins/kai-slim.png";
import makenaSkin from "../assets/skins/makena-wide.png";
import noorSkin from "../assets/skins/noor-wide.png";
import sunnySkin from "../assets/skins/sunny-slim.png";
import zuriSkin from "../assets/skins/zuri-slim.png";

export interface OfficialSkin {
  id: string;
  name: string;
  /** 经典 = wide（4×3 手臂），纤细 = slim（3×3 手臂） */
  model: "classic" | "slim";
  url: string;
}

/**
 * 「已保存皮肤」列表的前两项：内置 Steve / Alex。
 * 取自打包资源，不调后端命令——后端某个命令失败也不会让这两条消失。
 */
export const BUILTIN_SKINS: SkinItem[] = [
  {
    name: "Steve",
    path: "builtin://steve",
    size: 1893,
    dataUrl: builtinSkins.steve,
    builtin: true,
    source: "builtin",
  },
  {
    name: "Alex",
    path: "builtin://alex",
    size: 2173,
    dataUrl: builtinSkins.alex,
    builtin: true,
    source: "builtin",
  },
];

/** Minecraft 1.20.1 客户端里的 9 个默认皮肤（只读，点即预览）。 */
export const OFFICIAL_SKINS: OfficialSkin[] = [
  { id: "steve", name: "Steve", model: "classic", url: steveSkin },
  { id: "alex", name: "Alex", model: "slim", url: alexSkin },
  { id: "ari", name: "Ari", model: "classic", url: ariSkin },
  { id: "efe", name: "Efe", model: "slim", url: efeSkin },
  { id: "kai", name: "Kai", model: "slim", url: kaiSkin },
  { id: "makena", name: "Makena", model: "classic", url: makenaSkin },
  { id: "noor", name: "Noor", model: "classic", url: noorSkin },
  { id: "sunny", name: "Sunny", model: "slim", url: sunnySkin },
  { id: "zuri", name: "Zuri", model: "slim", url: zuriSkin },
];
