import { useEffect, useRef } from "react";

interface Props {
  /** 皮肤 PNG 的 data URL；空值画一个占位方块 */
  skin: string | null;
  /** CSS 像素尺寸 */
  size?: number;
  className?: string;
}

/**
 * 2D 头像：从同一张皮肤里裁出头部（底面 8,8 与帽子层 40,8），
 * 关掉插值放大，账号卡与皮肤列表都用它，不再单独存头像图。
 */
export function HeadAvatar({ skin, size = 36, className }: Props) {
  const ref = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const scale = 2;
    canvas.width = size * scale;
    canvas.height = size * scale;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    if (!skin) {
      ctx.fillStyle = "#9aa2b1";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      return;
    }
    let alive = true;
    const image = new Image();
    image.onload = () => {
      if (!alive) return;
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(image, 8, 8, 8, 8, 0, 0, canvas.width, canvas.height);
      ctx.drawImage(image, 40, 8, 8, 8, 0, 0, canvas.width, canvas.height);
    };
    image.onerror = () => {
      if (!alive) return;
      ctx.fillStyle = "#9aa2b1";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
    };
    image.src = skin;
    return () => {
      alive = false;
    };
  }, [skin, size]);

  return <canvas ref={ref} className={className} style={{ width: size, height: size }} aria-hidden="true" />;
}
