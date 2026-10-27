import { useEffect, useState } from "react";
import { useDownloads } from "../lib/downloads-store";
import { formatPercent, formatSpeed } from "../lib/format";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { GithubAccelerator, Settings } from "../lib/types";

/** 本机种子：第一次进页面生成一次，之后一直用它（清掉等于重新抽一个）。 */
const SEED_KEY = "gugu.fun.seed";
/** 今日人品：{ date: "YYYY-MM-DD", value: n }；日期不是今天就当作没抽过。 */
const FORTUNE_KEY = "gugu.fun.fortune";

/** 运势小结按 20 一档，共 5 句。 */
const LUCK_BANDS = 5;

/** 本地日期：不用 toISOString（那是 UTC，东八区过了 16 点会跨到「明天」）。 */
export function dateKey(date = new Date()): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return date.getFullYear() + "-" + month + "-" + day;
}

/** 今日人品：FNV-1a 32 位哈希（日期 + 种子）落到 1–100。纯函数，同一天同一设备恒定。 */
export function fortuneFor(date: string, seed: string): number {
  let hash = 0x811c9dc5;
  const input = date + "|" + seed;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return (hash % 100) + 1;
}

/** 基调按十位分段（tone.1 最好），修饰按个位，运势小结按 20 一档。 */
export function fortuneTextKeys(value: number): { tone: string; mood: string; luck: string } {
  const index = Math.min(100, Math.max(1, Math.round(value))) - 1;
  return {
    tone: "fun.tone." + (10 - Math.floor(index / 10)),
    mood: "fun.mood." + ((index % 10) + 1),
    luck: "fun.luck." + Math.min(LUCK_BANDS, Math.floor(index / 20) + 1),
  };
}

function readSeed(): string {
  try {
    const existing = window.localStorage.getItem(SEED_KEY);
    if (existing) return existing;
    const created = Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 10);
    window.localStorage.setItem(SEED_KEY, created);
    return created;
  } catch {
    return "gugu-fun-fallback";
  }
}

function readFortune(today: string): number | null {
  try {
    const raw = window.localStorage.getItem(FORTUNE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { date?: unknown; value?: unknown };
    if (parsed.date !== today || typeof parsed.value !== "number") return null;
    return Math.min(100, Math.max(1, Math.round(parsed.value)));
  } catch {
    return null;
  }
}

function writeFortune(entry: { date: string; value: number }): void {
  try {
    window.localStorage.setItem(FORTUNE_KEY, JSON.stringify(entry));
  } catch {
    /* 存不下也不影响这一次显示 */
  }
}

/** 加速节点展示名（id 由 GithubAccelerator 约束，多一个少一个 tsc 都会报错）。 */
const ACCELERATORS: Array<{ id: GithubAccelerator; label: string }> = [
  { id: "ghf", label: "ghf.无名氏.top" },
  { id: "ghfast", label: "ghfast.top" },
  { id: "cnxiaobai", label: "github.cnxiaobai.com" },
  { id: "tangbai", label: "git.tangbai.cc" },
  { id: "chenc", label: "github.chenc.dev" },
  { id: "xyz", label: "30006000.xyz" },
];

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

interface Props {
  settings: Settings;
  dataRoot: string;
  onChange: (patch: Partial<Settings>) => void;
  onNotify: (message: string) => void;
  /** 搜索过滤：true 表示该分组命中关键词，需要渲染 */
  visible: (groupId: string) => boolean;
}

/** 娱乐页三块：今日人品 / 每日一言 / GitHub 下载器（挂设置页左侧导航的「娱乐」下）。 */
export function FunGroups({ settings, dataRoot, onChange, onNotify, visible }: Props) {
  const t = useT();

  // ---- 今日人品 ----
  const [seed] = useState(() => readSeed());
  const [today] = useState(() => dateKey());
  const [fortune, setFortune] = useState<number | null>(() => readFortune(today));

  const roll = () => {
    const next = fortuneFor(today, seed);
    writeFortune({ date: today, value: next });
    setFortune(next);
  };

  const texts = fortune === null ? null : fortuneTextKeys(fortune);

  // ---- 每日一言 ----
  const [saying, setSaying] = useState<string | null>(null);
  const [sayingSource, setSayingSource] = useState<string>("");
  const [sayingLoading, setSayingLoading] = useState(false);

  const drawSaying = async () => {
    setSayingLoading(true);
    try {
      const result = await api.funSaying();
      setSaying(result.ok && result.text ? result.text : t("fun.sayingFallback"));
      setSayingSource(result.source || settings.sayingApiUrl);
    } catch {
      // 接口失败不弹错误框：给一句兜底文案即可
      setSaying(t("fun.sayingFallback"));
      setSayingSource("");
    } finally {
      setSayingLoading(false);
    }
  };

  // ---- GitHub 下载器 ----
  const { tasks, cancel } = useDownloads();
  const githubTasks = tasks.filter((task) => task.kind === "github");
  const latest = githubTasks[0] ?? null;

  const [url, setUrl] = useState("");
  const [fileName, setFileName] = useState("");
  const [dirDraft, setDirDraft] = useState(settings.githubDownloadDir);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDirDraft(settings.githubDownloadDir);
  }, [settings.githubDownloadDir]);

  const defaultDir = dataRoot ? dataRoot.replace(/[\\/]+$/, "") + "\\DL" : "";
  const effectiveDir = dirDraft.trim() || defaultDir;

  const pickDir = async () => {
    try {
      const picked = await api.pickDirectory();
      if (picked) {
        setDirDraft(picked);
        onChange({ githubDownloadDir: picked });
      }
    } catch (caught) {
      onNotify(errorMessage(caught));
    }
  };

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.githubDownload({ url: url.trim(), dir: dirDraft.trim(), fileName: fileName.trim() });
      onNotify(t("fun.githubStarted", { name: fileName.trim() || t("fun.githubFileNameAuto") }));
      setUrl("");
      setFileName("");
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      {visible("fortune") ? (
        <section className="card settings-card" data-group="fortune">
          <div className="settings-card__title">{t("fun.fortune")}</div>
          <div className="row__hint">{t("fun.fortuneSub")}</div>

          <div className="fortune">
            <div className="fortune__value" data-fortune={fortune ?? "none"}>
              {fortune ?? "?"}
            </div>
            <div className="fortune__body">
              {fortune === null ? (
                <button type="button" className="pill-button" data-roll="1" onClick={roll}>
                  {t("fun.roll")}
                </button>
              ) : (
                <>
                  <div className="fortune__texts">
                    <span className="fortune__tone" data-fortune-tone>
                      {texts ? t(texts.tone) : ""}
                    </span>
                    <span className="fortune__mood" data-fortune-mood>
                      {texts ? t(texts.mood) : ""}
                    </span>
                    <p className="fortune__luck" data-fortune-luck>
                      {texts ? t(texts.luck) : ""}
                    </p>
                  </div>
                  <div className="fortune__meta">
                    <span className="chip" data-fortune-rolled>
                      {t("fun.rolled")}
                    </span>
                    <span className="mono">{t("fun.fortuneStored", { date: today })}</span>
                  </div>
                </>
              )}
            </div>
          </div>

          <div className="row__hint">{t("fun.fortuneNote")}</div>
        </section>
      ) : null}

      {visible("saying") ? (
        <section className="card settings-card" data-group="saying">
          <div className="settings-card__title">{t("fun.saying")}</div>
          <div className="row__hint">{t("fun.sayingSub")}</div>

          <button type="button" className="saying" data-saying="1" disabled={sayingLoading} onClick={() => void drawSaying()}>
            <span className="saying__text" data-saying-text>
              {sayingLoading ? t("fun.sayingLoading") : saying ?? t("fun.sayingInitial")}
            </span>
            {saying ? <span className="saying__hint">{t("fun.sayingAgain")}</span> : null}
          </button>

          <div className="row__hint">
            {sayingSource ? t("fun.sayingFrom", { url: sayingSource }) + " · " : ""}
            {t("fun.sayingHint")}
          </div>
        </section>
      ) : null}

      {visible("github") ? (
        <section className="card settings-card" data-group="github">
          <div className="settings-card__title">{t("fun.github")}</div>
          <div className="row__hint">{t("fun.githubSub")}</div>

          <div className="field">
            <label htmlFor="github-url">{t("fun.githubUrl")}</label>
            <input
              id="github-url"
              type="text"
              data-github="url"
              spellCheck={false}
              value={url}
              placeholder={t("fun.githubUrlPlaceholder")}
              onChange={(event) => setUrl(event.target.value)}
            />
          </div>

          <div className="field-pair">
            <div className="field">
              <label htmlFor="github-dir">{t("fun.githubDir")}</label>
              <div className="secret-field">
                <input
                  id="github-dir"
                  type="text"
                  data-github="dir"
                  spellCheck={false}
                  value={dirDraft}
                  placeholder={effectiveDir}
                  onChange={(event) => {
                    setDirDraft(event.target.value);
                    onChange({ githubDownloadDir: event.target.value });
                  }}
                />
                <button type="button" data-github="pickDir" onClick={() => void pickDir()}>
                  {t("fun.githubPickDir")}
                </button>
              </div>
              <div className="row__hint">{t("fun.githubDirDefault", { dir: effectiveDir || "—" })}</div>
            </div>

            <div className="field">
              <label htmlFor="github-name">{t("fun.githubFileName")}</label>
              <input
                id="github-name"
                type="text"
                data-github="name"
                spellCheck={false}
                value={fileName}
                placeholder={t("fun.githubFileNameAuto")}
                onChange={(event) => setFileName(event.target.value)}
              />
            </div>
          </div>

          <div className="field">
            <label htmlFor="github-accel">{t("fun.githubAccelerator")}</label>
            <select
              id="github-accel"
              data-github="accelerator"
              value={settings.githubAccelerator}
              onChange={(event) => onChange({ githubAccelerator: event.target.value as Settings["githubAccelerator"] })}
            >
              <option value="auto">{t("fun.githubNodeAuto")}</option>
              {ACCELERATORS.map((node) => (
                <option key={node.id} value={node.id}>
                  {node.label}
                </option>
              ))}
            </select>
          </div>

          <div className="field-actions">
            <button
              type="button"
              className="pill-button"
              data-github="start"
              disabled={busy || url.trim().length === 0}
              onClick={() => void start()}
            >
              {busy ? t("fun.githubStarting") : t("fun.githubStart")}
            </button>
            <button type="button" data-github="openDir" onClick={() => void api.githubOpenFolder().catch((caught) => onNotify(errorMessage(caught)))}>
              {t("fun.githubOpenDir")}
            </button>
          </div>

          {error ? (
            <div className="test-result is-off" role="alert" data-github-error>
              {error}
            </div>
          ) : null}

          {latest ? (
            <div className="dl-item" data-github-task>
              <div className="dl-item__top">
                <span className="dl-item__title">{latest.title}</span>
                {latest.phase === "finished" ? (
                  <button type="button" className="dl-item__link" onClick={() => void api.githubOpenFolder()}>
                    {t("fun.githubOpenDir")}
                  </button>
                ) : (
                  <button type="button" className="dl-item__icon" title={t("fun.githubCancel")} onClick={() => cancel(latest.id)}>
                    ×
                  </button>
                )}
              </div>
              <div className="dl-bar">
                <div className="dl-bar__fill" style={{ width: formatPercent(latest.progress) }} />
              </div>
              <div className="dl-item__meta">
                <span className="dl-item__stage">{latest.stage || t("fun.githubQueued")}</span>
                <span className="dl-item__speed">
                  {formatPercent(latest.progress)} · {formatSpeed(latest.bytesPerSec)}
                </span>
              </div>
            </div>
          ) : null}
        </section>
      ) : null}
    </>
  );
}
