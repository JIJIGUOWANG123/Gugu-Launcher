import { builtinIconById } from "../assets/instance-icons";
import { iconFileUrl } from "../lib/tauri-api";
import type { InstanceIconSpec } from "../lib/types";

interface Props {
  /** 图标描述（内置 / 自定义 / none） */
  spec?: InstanceIconSpec | null;
  /** 后端 icon.png 路径（老字段，和 spec 二选一） */
  icon?: string;
  /** 不选图标时显示名称首字 */
  name?: string;
  className?: string;
  alt?: string;
}

/** 没选渐变时的默认底板（浅色系，不抢内容） */
export const DEFAULT_ICON_GRADIENT = "linear-gradient(135deg, #cfe3ff, #a9c8f5)";

/**
 * 实例图标 = 底板（渐变）+ 前景（内置图 / 自定义图 / 名称首字）。
 * spec 与后端 icon 路径都缺时退回名称首字，不会渲染空方块。
 */
export function InstanceIcon({ spec, icon, name, className, alt = "" }: Props) {
  const kind = spec?.kind ?? (icon ? "custom" : "none");
  const builtin = kind === "builtin" ? builtinIconById(spec?.id) : null;
  const customUrl = kind === "custom" ? spec?.dataUrl || iconFileUrl(spec?.path || icon || "") || "" : "";
  const src = kind === "builtin" ? (builtin?.url ?? "") : customUrl;
  const gradient = spec?.gradient || DEFAULT_ICON_GRADIENT;
  const initial = (name ?? "").trim().slice(0, 1) || "?";

  return (
    <span
      className={"instance-icon" + (className ? " " + className : "")}
      data-instance-icon={kind}
      data-instance-icon-id={spec?.id ?? ""}
      style={{ background: gradient }}
      role="img"
      aria-label={alt || name || "实例图标"}
    >
      {src ? (
        <img className="instance-icon__img" src={src} alt="" />
      ) : (
        <span className="instance-icon__initial" data-instance-icon-initial>
          {initial}
        </span>
      )}
    </span>
  );
}
