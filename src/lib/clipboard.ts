/**
 * 复制文本：优先 navigator.clipboard，失败回退 textarea + execCommand。
 * 顺手把结果记到 window.__guguClipboard（只给自检用，界面不显示）。
 */
export async function copyText(text: string): Promise<boolean> {
  if (!text) return false;
  let ok = false;
  try {
    await navigator.clipboard.writeText(text);
    ok = true;
  } catch {
    ok = fallbackCopy(text);
  }
  try {
    const host = window as unknown as Record<string, unknown>;
    const seen = Array.isArray(host.__guguClipboard) ? (host.__guguClipboard as string[]) : [];
    host.__guguClipboard = [...seen, (ok ? "ok:" : "fail:") + text].slice(-40);
  } catch {
    /* 自检记录失败不影响复制本身 */
  }
  return ok;
}

function fallbackCopy(text: string): boolean {
  try {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.top = "-1000px";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(area);
    return ok;
  } catch {
    return false;
  }
}
