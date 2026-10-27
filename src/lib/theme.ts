/**
 * 主题落地：`data-theme` 控制浅/深两套 CSS 变量，`--accent` 由用户主题色覆盖。
 * 只做 DOM 写入，不持有状态（状态在 App 的 settings 里）。
 */
import type { ThemeMode } from "./types";

export const ACCENT_PRESETS: Array<{ id: string; label: string; value: string }> = [
  { id: "yellow", label: "默认黄", value: "#f5c845" },
  { id: "blue", label: "天空蓝", value: "#4c9aff" },
  { id: "green", label: "草绿", value: "#3ecf8e" },
  { id: "purple", label: "紫罗兰", value: "#9b7bff" },
  { id: "pink", label: "樱粉", value: "#f27ba6" },
  { id: "orange", label: "南瓜橙", value: "#fb923c" },
];

export const ACCENT_STORAGE_KEY = "gugu.accent";
export const THEME_STORAGE_KEY = "gugu.theme";
export const FONT_STORAGE_KEY = "gugu.font";

/** 默认字体「微软雅黑」，与 Rust 侧 fonts::DEFAULT_FONT 保持一致。 */
export const DEFAULT_UI_FONT = "Microsoft YaHei";

/** #rgb / #rrggbb 均接受，其他一律回退默认黄。 */
export function normalizeAccent(value: string | undefined | null): string {
  const fallback = ACCENT_PRESETS[0].value;
  if (!value) return fallback;
  return /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.test(value) ? value : fallback;
}

/** 当前主题色是不是某个预设（自定义态时预设圈都不高亮）。 */
export function isPresetAccent(value: string | undefined | null): boolean {
  if (!value) return false;
  const target = expandHex(value).toLowerCase();
  return ACCENT_PRESETS.some((preset) => preset.value.toLowerCase() === target);
}

/** #rgb → #rrggbb，其他原样返回。 */
function expandHex(value: string): string {
  const hex = value.trim();
  if (/^#[0-9a-fA-F]{3}$/.test(hex)) {
    return "#" + hex[1] + hex[1] + hex[2] + hex[2] + hex[3] + hex[3];
  }
  return hex;
}

function hexToRgb(value: string): [number, number, number] {
  const hex = expandHex(value).replace("#", "");
  if (hex.length !== 6) return [245, 200, 69];
  return [parseInt(hex.slice(0, 2), 16), parseInt(hex.slice(2, 4), 16), parseInt(hex.slice(4, 6), 16)];
}

function rgbToHex(rgb: [number, number, number]): string {
  return (
    "#" +
    rgb
      .map((channel) => Math.max(0, Math.min(255, Math.round(channel))).toString(16).padStart(2, "0"))
      .join("")
  );
}

/** 感知亮度（0–1），用来判断该加深还是提亮。 */
function luminance(rgb: [number, number, number]): number {
  const [r, g, b] = rgb.map((channel) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : Math.pow((value + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/**
 * 派生 --accent-strong：浅色主题往深里混（自定义色太亮时混得更多），
 * 深色主题往白里混，保证文字在两种主题下都有对比度。
 */
export function accentStrong(accent: string, theme: ThemeMode): string {
  const rgb = hexToRgb(normalizeAccent(accent));
  const lum = luminance(rgb);
  const dark = theme === "dark";
  const amount = dark ? (lum < 0.35 ? 0.55 : 0.3) : lum > 0.62 ? 0.55 : 0.32;
  const target: [number, number, number] = dark ? [255, 255, 255] : [26, 18, 0];
  return rgbToHex([
    rgb[0] + (target[0] - rgb[0]) * amount,
    rgb[1] + (target[1] - rgb[1]) * amount,
    rgb[2] + (target[2] - rgb[2]) * amount,
  ]);
}

/** 主色上的文字颜色：亮底用深字，暗底用白字（自定义色也安全）。 */
export function accentContrast(accent: string): string {
  return luminance(hexToRgb(normalizeAccent(accent))) > 0.55 ? "#1a1400" : "#ffffff";
}

/**
 * 界面字体：把 `--font-ui` 写到 :root，全局 CSS（含表单控件）都用这个变量。
 * 名字里可能带空格，一律加引号；别名（SimSun/宋体 这种）按顺序拼进回退链，
 * 某些 WebView2 版本只认其中一个名字，多写几个才能真的换过来。
 */
export function fontStack(family: string | undefined, aliases?: string[]): string {
  const name = (family ?? "").trim() || DEFAULT_UI_FONT;
  const names = [name, ...(aliases ?? []).map((alias) => alias.trim()).filter((alias) => alias.length > 0 && alias !== name)];
  const quoted = names.map((item) => '"' + item + '"');
  return quoted.join(", ") + ", system-ui, sans-serif";
}

export function applyFont(family: string | undefined, aliases?: string[]): void {
  const name = (family ?? "").trim() || DEFAULT_UI_FONT;
  document.documentElement.style.setProperty("--font-ui", fontStack(name, aliases));
  try {
    window.localStorage.setItem(FONT_STORAGE_KEY, name);
  } catch {
    /* 忽略隐私模式写入失败 */
  }
  // 个别 WebView2 会缓存字体，读一次布局强制重排
  void document.body.offsetHeight;
}

/**
 * 本机是否真的有这个字体。
 * document.fonts.check 对「不存在的 family」也常常返回 true，所以再量一次宽度：
 * 用 fallback 和不用 fallback 量同一串字，宽度完全一样就说明根本没换上。
 */
export function isFontAvailable(family: string): boolean {
  const name = (family ?? "").trim();
  if (!name) return false;
  try {
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d");
    if (!ctx) return true;
    const sample = "咕咕启动器永字八法ABC123";
    ctx.font = '20px "' + name + '", monospace';
    const withFamily = ctx.measureText(sample).width;
    ctx.font = "20px monospace";
    const fallback = ctx.measureText(sample).width;
    if (Math.abs(withFamily - fallback) < 0.01) return false;
    if (typeof document.fonts !== "undefined" && typeof document.fonts.check === "function") {
      const known = document.fonts.check('20px "' + name + '"');
      // 宽度不同但 fonts.check 说没有 → 大概率是别的字序命中，按可用处理
      return known || withFamily !== fallback;
    }
    return true;
  } catch {
    return true;
  }
}

export function readCachedFont(): string {
  try {
    return window.localStorage.getItem(FONT_STORAGE_KEY) || DEFAULT_UI_FONT;
  } catch {
    return DEFAULT_UI_FONT;
  }
}

export function applyTheme(theme: ThemeMode, accent: string): void {
  const root = document.documentElement;
  const value = normalizeAccent(accent);
  root.dataset.theme = theme;
  root.style.setProperty("--accent", value);
  // 派生色以内联变量写死：自定义色不在预设里，CSS 的 color-mix 基线仍然是默认黄
  root.style.setProperty("--accent-strong", accentStrong(value, theme));
  root.style.setProperty("--accent-contrast", accentContrast(value));
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    window.localStorage.setItem(ACCENT_STORAGE_KEY, value);
  } catch {
    /* 忽略隐私模式写入失败 */
  }
}

/** 浏览器预览时没有后端 settings，用 localStorage 记住用户的主题选择。 */
export function readCachedTheme(): { theme: ThemeMode; accent: string } {
  try {
    const theme = window.localStorage.getItem(THEME_STORAGE_KEY);
    const accent = window.localStorage.getItem(ACCENT_STORAGE_KEY);
    return {
      theme: theme === "dark" ? "dark" : "light",
      accent: normalizeAccent(accent),
    };
  } catch {
    return { theme: "light", accent: ACCENT_PRESETS[0].value };
  }
}
