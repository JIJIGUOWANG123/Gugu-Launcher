import { useEffect, useRef } from "react";

interface Props {
  /** 贴图 PNG 的 data URL / http 地址 */
  src: string | null;
  /** 从贴图上裁的矩形（贴图坐标） */
  sx: number;
  sy: number;
  sw: number;
  sh: number;
  /** 画出来的 CSS 尺寸 */
  width: number;
  height: number;
  className?: string;
}

/**
 * 从一张贴图里裁一块放大显示（关掉插值，保持像素感）。
 * 皮肤列表用头部、披风列表用披风正面，都走这一个组件，不用另存缩略图。
 */
export function TextureCrop({ src, sx, sy, sw, sh, width, height, className }: Props) {
  const ref = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const scale = 2;
    canvas.width = width * scale;
    canvas.height = height * scale;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    if (!src) return;
    let alive = true;
    const image = new Image();
    image.onload = () => {
      if (!alive) return;
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(image, sx, sy, sw, sh, 0, 0, canvas.width, canvas.height);
    };
    image.src = src;
    return () => {
      alive = false;
    };
  }, [src, sx, sy, sw, sh, width, height]);

  return <canvas ref={ref} className={className} style={{ width, height }} aria-hidden="true" />;
}
