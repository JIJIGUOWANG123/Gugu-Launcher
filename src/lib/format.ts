/**
 * 显示层格式化：时长、字节、速度、时间。文案全部走 i18n，别在这里写死中文。
 */
import { t } from "./i18n";

/** 500 → 「8 分 20 秒」；不足 1 分钟按秒显示，超过 1 小时显示「小时 + 分」。 */
export function formatDuration(totalSecs: number): string {
  const secs = Math.max(0, Math.round(totalSecs));
  if (secs < 60) return t("instances.durationSecond", { s: secs });
  if (secs < 3600) {
    return t("instances.durationMinute", { m: Math.floor(secs / 60), s: secs % 60 });
  }
  const hours = Math.floor(secs / 3600);
  const minutes = Math.floor((secs % 3600) / 60);
  return t("instances.durationHour", { h: hours, m: minutes });
}

/** 字节数 → 「12.3 MB」；未知（null）显示占位符。 */
export function formatBytes(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return (unit === 0 ? Math.round(value).toString() : value.toFixed(1)) + " " + units[unit];
}

/** 速度：「3.2 MB/s」 */
export function formatSpeed(bytesPerSec: number): string {
  return formatBytes(bytesPerSec) + "/s";
}

/** ISO 时间 → 「02-01 18:20」 */
export function formatStamp(iso: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  const pad = (value: number) => value.toString().padStart(2, "0");
  return pad(date.getMonth() + 1) + "-" + pad(date.getDate()) + " " + pad(date.getHours()) + ":" + pad(date.getMinutes());
}

/** 0..1 → 「42%」 */
export function formatPercent(ratio: number): string {
  return Math.round(Math.min(1, Math.max(0, ratio)) * 100) + "%";
}
