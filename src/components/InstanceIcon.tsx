import { iconFileUrl } from "../lib/tauri-api";

interface Props {
  /** 实例目录下的 icon.png 绝对路径；空字符串＝还没设置 */
  icon: string;
  className?: string;
  alt?: string;
}

/**
 * 实例图标：有 icon.png 就用文件，没有就用内联 SVG 草方块占位。
 * TODO(M2-B): 素材目录里没有草方块 png，先用 SVG 顶着；补素材后换成 <img>。
 */
export function InstanceIcon({ icon, className, alt = "" }: Props) {
  const url = iconFileUrl(icon);
  if (url) {
    return <img className={className} src={url} alt={alt} />;
  }
  return (
    <svg className={className} viewBox="0 0 32 32" role="img" aria-label={alt || "实例图标"} shapeRendering="crispEdges">
      <rect width="32" height="32" fill="#8d6141" />
      <rect width="32" height="10" fill="#63a83c" />
      <rect y="10" width="32" height="3" fill="#4d8a2c" />
      <rect x="4" y="14" width="4" height="4" fill="#7a5236" />
      <rect x="14" y="18" width="4" height="4" fill="#9c6f4a" />
      <rect x="24" y="15" width="4" height="4" fill="#7a5236" />
      <rect x="8" y="24" width="4" height="4" fill="#9c6f4a" />
      <rect x="20" y="26" width="4" height="4" fill="#6d4a30" />
      <rect x="2" y="2" width="3" height="3" fill="#7cbe52" />
      <rect x="18" y="3" width="3" height="3" fill="#7cbe52" />
      <rect x="27" y="6" width="3" height="3" fill="#4d8a2c" />
    </svg>
  );
}
