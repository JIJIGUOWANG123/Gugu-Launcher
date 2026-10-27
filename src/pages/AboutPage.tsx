import { useCallback, useEffect, useState, type ReactNode } from "react";
import { brand } from "../assets/brand";
import { icons } from "../assets/icons";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { AppChangelog, AppInfo, Settings, UpdateStatus } from "../lib/types";

interface Props {
  appInfo: AppInfo | null;
  settings: Settings | null;
  /** 检查更新；失败也要返回 phase=failed 的状态，不抛异常 */
  onCheck: () => Promise<UpdateStatus>;
  onInstall: () => Promise<void>;
  busy: boolean;
  notify: (message: string) => void;
}

/** 后端字段可能缺（老版本 / 异常包）：缺就用应用版本兜底，界面绝不出现 undefined。 */
function normalizeStatus(raw: UpdateStatus | null, fallbackVersion: string): UpdateStatus {
  return {
    phase: raw?.phase ?? "idle",
    currentVersion: raw?.currentVersion || fallbackVersion,
    remoteVersion: raw?.remoteVersion || null,
    message: raw?.message || "",
    checkedAt: raw?.checkedAt ?? Date.now(),
    canInstall: raw?.canInstall === true,
  };
}

const REPO_URL = "https://github.com/JIJIGUOWANG123/Gugu-Launcher";
const CHANGELOG_URL = "https://github.com/JIJIGUOWANG123/Gugu-Launcher/releases/latest";
const LICENSE_URL = "https://github.com/JIJIGUOWANG123/Gugu-Launcher/blob/main/LICENSE";

/** 极简 markdown：只认 #/## 标题、- 列表、**加粗**（不引第三方库，也不做 HTML 注入）。 */
function renderMarkdown(markdown: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let listBuffer: string[] = [];
  const flushList = () => {
    if (listBuffer.length === 0) return;
    nodes.push(
      <ul key={"ul-" + nodes.length}>
        {listBuffer.map((item, index) => (
          <li key={index}>{renderInline(item)}</li>
        ))}
      </ul>,
    );
    listBuffer = [];
  };
  for (const rawLine of markdown.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) {
      flushList();
      continue;
    }
    if (line.startsWith("#")) {
      flushList();
      const title = line.replace(/^#+\s*/, "");
      nodes.push(
        <div className="changelog-md__head" key={"h-" + nodes.length}>
          {renderInline(title)}
        </div>,
      );
      continue;
    }
    if (/^[-*]\s+/.test(line)) {
      listBuffer.push(line.replace(/^[-*]\s+/, ""));
      continue;
    }
    flushList();
    nodes.push(
      <p className="changelog-md__p" key={"p-" + nodes.length}>
        {renderInline(line)}
      </p>,
    );
  }
  flushList();
  return nodes;
}

/** **加粗** → <strong>；其余原样当文本（不解析 HTML）。 */
function renderInline(text: string): ReactNode[] {
  return text.split(/(\*\*[^*]+\*\*)/g).filter(Boolean).map((part, index) => {
    const bold = /^\*\*.+\*\*$/.test(part);
    return bold ? <strong key={index}>{part.slice(2, -2)}</strong> : <span key={index}>{part}</span>;
  });
}

/**
 * 关于页：Logo + 版本 + 三个外链入口 + 更新日志（最近 5 条，默认只展开最新一条）+ 检查更新 + 许可。
 * 自动检查模式下不放手点按钮，只显示状态文字。
 */
export function AboutPage({ appInfo, settings, onCheck, onInstall, busy, notify }: Props) {
  const t = useT();
  const [checking, setChecking] = useState(false);
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  // 更新日志：只显示后端给的最新一个版本；后端挂了就只用当前版本兜底一条
  const [changelog, setChangelog] = useState<AppChangelog | null>(null);
  const [changelogError, setChangelogError] = useState("");

  const auto = (settings?.updateMode ?? "manual") === "auto";
  // 应用版本兜底：后端没给 currentVersion 也不渲染 undefined
  const version = appInfo?.version || status?.currentVersion || "1.0.0";
  const phase = checking ? "checking" : status?.phase ?? "idle";

  const openUrl = useCallback(
    async (url: string) => {
      try {
        await api.openExternal(url);
      } catch (error) {
        notify(t("about.openFailed", { message: String(error) }));
      }
    },
    [notify, t],
  );

  const runCheck = async () => {
    setChecking(true);
    try {
      setStatus(normalizeStatus(await onCheck(), version));
    } catch (error) {
      // 失败就是 failed + 可重试，绝不当成「已是最新」
      setStatus({
        phase: "failed",
        currentVersion: version,
        remoteVersion: null,
        message: t("about.checkFailed", { message: error instanceof Error ? error.message : String(error) }),
        checkedAt: Date.now(),
        canInstall: false,
      });
    } finally {
      setChecking(false);
    }
  };

  // 更新日志：只取最新一个版本；失败时用当前版本兜底（也只显示这一条）
  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const result = await api.appChangelog();
        if (!alive) return;
        if (result && typeof result === "object" && typeof result.markdown === "string") {
          setChangelog({ version: result.version || appInfo?.version || "1.0.0", markdown: result.markdown });
        } else {
          setChangelogError(t("about.changelogUnavailable"));
        }
      } catch (error) {
        if (!alive) return;
        setChangelogError(error instanceof Error ? error.message : String(error));
      }
    })();
    return () => {
      alive = false;
    };
    // 只在挂载时取一次；版本号兜底用当下的 appInfo
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 进页面自动检查一次（点「检查更新」再强制查一次）
  useEffect(() => {
    void runCheck();
    // 只在挂载时跑一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** 状态行：优先直显后端 message，缺了才用 i18n 兜底（任何分支都不会出现 undefined）。 */
  const statusLine = (() => {
    if (checking) return t("about.checking");
    if (!status) return t("about.checkReady");
    switch (status.phase) {
      case "idle":
        return t("about.checkIdle");
      case "checking":
        return t("about.checking");
      case "available":
        return status.message || t("about.statusAvailable", { version: status.remoteVersion ?? version });
      case "downloading":
        return status.message || t("about.downloading");
      case "installing":
        return status.message || t("about.installing");
      case "failed":
        return status.message || t("about.checkFailedRetry");
      default:
        return status.message || t("about.uptodate", { version: status.currentVersion || version });
    }
  })();

  return (
    <div className="about-page">
      <section className="card about-hero">
        <img className="about-hero__logo" src={brand.logo} alt="" data-about-logo />
        <div className="about-hero__meta">
          <div className="about-hero__name">
            {t("app.name")}
            <span className="chip" data-about-version>
              v{appInfo?.version ?? "1.0.0"}
            </span>
          </div>
          <div className="row__hint">{t("about.tagline")}</div>
        </div>
        <div className="about-hero__entries">
          <button type="button" className="pill-outline" data-action="about-open-repo" onClick={() => void openUrl(REPO_URL)}>
            <img src={icons.shareExport} alt="" />
            {t("about.entry.repo")}
          </button>
          <button type="button" className="pill-outline" data-action="about-open-changelog" onClick={() => void openUrl(CHANGELOG_URL)}>
            <img src={icons.log} alt="" />
            {t("about.entry.changelog")}
          </button>
          <button type="button" className="pill-outline" data-action="about-open-license" onClick={() => void openUrl(LICENSE_URL)}>
            <img src={icons.alert} alt="" />
            {t("about.entry.license")}
          </button>
        </div>
      </section>

      <section className="card about-changelog" data-app-changelog>
        <div className="about-changelog__head">
          <span className="detail-card__title">{t("about.changelog")}</span>
          <span className="chip chip--tiny" data-changelog-version>
            {changelog?.version ?? version}
          </span>
          <button
            type="button"
            className="link-button"
            data-action="about-open-changelog-full"
            onClick={() => void openUrl(CHANGELOG_URL)}
          >
            {t("about.changelogFull")}
          </button>
        </div>
        {changelog ? (
          <div className="changelog-md" data-changelog>
            {renderMarkdown(changelog.markdown)}
          </div>
        ) : changelogError ? (
          <div className="changelog-md" data-changelog data-changelog-fallback>
            <div className="changelog-md__head">{version}</div>
            <p className="changelog-md__p">{t("about.changelogFallback", { reason: changelogError })}</p>
          </div>
        ) : (
          <div className="row__hint">{t("common.loading")}</div>
        )}
      </section>

      <section className="card about-update" data-update-card>
        <div className="about-update__row">
          <div>
            <div className="detail-card__title">{t("about.update")}</div>
            <div
              className={"row__hint" + (phase === "failed" ? " about-update__error" : "")}
              data-update-status
              data-update-phase={phase}
            >
              {statusLine}
            </div>
            <div className="row__hint" data-update-current>
              {t("about.currentVersion", { version })}
            </div>
            {auto ? (
              <div className="row__hint" data-update-auto>
                {t("about.autoState")} · {t("about.autoHint")}
              </div>
            ) : null}
          </div>
          <div className="about-update__actions">
            {status?.phase === "failed" ? (
              <button
                type="button"
                className="pill-outline"
                data-action="about-retry"
                disabled={busy || checking}
                onClick={() => void runCheck()}
              >
                <img src={icons.refresh} alt="" />
                {t("about.retry")}
              </button>
            ) : null}
            {status?.phase === "available" && status.canInstall ? (
              <button type="button" className="pill-button" data-action="about-install" disabled={busy} onClick={() => void onInstall()}>
                {t("about.installNow")}
              </button>
            ) : null}
            <button
              type="button"
              className={"pill-outline" + (checking ? " is-busy" : "")}
              data-action="about-check"
              data-checking={checking ? "1" : "0"}
              disabled={busy || checking}
              onClick={() => void runCheck()}
            >
              {checking ? t("about.checking") : t("about.check")}
            </button>
          </div>
        </div>
      </section>

      <section className="card about-license">
        <div className="about-license__row">
          <span className="detail-card__title">{t("about.license")}</span>
          <span className="mono">{t("about.licenseValue")}</span>
        </div>
        <div className="about-license__row">
          <span className="row__hint">{t("about.thirdParty")}</span>
          <span className="row__hint" data-third-party>
            {t("about.thirdPartyBody")}
          </span>
        </div>
      </section>
    </div>
  );
}
