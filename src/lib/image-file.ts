/** 浏览器侧的小工具：把用户选的图片读成 data URL 并量出尺寸（皮肤 PNG 校验用）。 */

export function fileToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(new Error("读取文件失败"));
    reader.readAsDataURL(file);
  });
}

export interface ImageSize {
  width: number;
  height: number;
}

export function readImageSize(dataUrl: string): Promise<ImageSize> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve({ width: image.naturalWidth, height: image.naturalHeight });
    image.onerror = () => reject(new Error("不是有效的图片"));
    image.src = dataUrl;
  });
}

/** 皮肤只接受 64×32（旧版）与 64×64（新版）两种尺寸。 */
export function isSkinSize(size: ImageSize): boolean {
  return size.width === 64 && (size.height === 32 || size.height === 64);
}

/** 从 data URL 里取纯 base64（给 skin_upload 的 pngBase64 用）。 */
export function stripDataUrlPrefix(dataUrl: string): string {
  const comma = dataUrl.indexOf(",");
  return comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
}

/**
 * 把任意图片来源读成纯 base64：
 * - data URL（用户刚拖入的文件、后端返回的 dataUrl）→ 直接切掉前缀；
 * - 打包资源 URL（vite 产物 / 内置 Steve、Alex、官方 9 套）→ fetch 成 ArrayBuffer 再 base64。
 * 内置皮肤运行时没有磁盘路径，ms_skin_upload 只能靠这条。
 */
export async function imageUrlToBase64(src: string): Promise<string> {
  if (!src) throw new Error("没有可上传的皮肤图片");
  if (src.startsWith("data:")) return stripDataUrlPrefix(src);
  const response = await fetch(src);
  if (!response.ok) throw new Error("读取皮肤图片失败：" + response.status);
  const bytes = new Uint8Array(await response.arrayBuffer());
  let binary = "";
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

/** 这个皮肤条目在磁盘上有真实文件吗（官方 / 内置资源都是 URL，没有路径）。 */
export function hasDiskPath(path: string | undefined | null): boolean {
  if (!path) return false;
  if (path.startsWith("official://") || path.startsWith("builtin://") || path.startsWith("http")) return false;
  // Windows 盘符或 *nix 绝对路径才算
  return /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("/");
}
