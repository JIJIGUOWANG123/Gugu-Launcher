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
