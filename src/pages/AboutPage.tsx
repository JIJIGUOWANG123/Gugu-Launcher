import { useCallback, useMemo, useState } from "react";
import { brand } from "../assets/brand";
import { icons } from "../assets/icons";
import { CHANGELOG, CHANGELOG_LIMIT } from "../lib/changelog";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { AppInfo, Settings, UpdateCheckResult } from "../lib/types";

interface Props {
  appInfo: AppInfo | null;
  settings: Settings | null;
  /** 手动检查更新；失败也要返回带 error 的结果，不抛异常 */
  onCheck: () => Promise<UpdateCheckResult>;
  onInstall: () => Promise<void>;
  busy: boolean;
  notify: (message: string) => void;
}

const REPO_URL = "https://github.com/JIJIGUOWANG123/Gugu-Launcher";
const CHANGELOG_URL = "https://github.com/JIJIGUOWANG123/Gugu-Launcher/releases";
const LICENSE_URL = "https://github.com/JIJIGUOWANG123/Gugu-Launcher/blob/main/LICENSE";

/**
 * 关于页：Logo + 版本 + 三个外链入口 + 更新日志（最近 5 条，默认只展开最新一条）+ 检查更新 + 许可。
 * 自动检查模式下不放手点按钮，只显示状态文字。
 */
export function AboutPage({ appInfo, settings, onCheck, onInstall, busy, notify }: Props) {
  const t = useT();
  const [expanded, setExpanded] = useState<string | null>(CHANGELOG[0]?.version ?? null);
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<UpdateCheckResult | null>(null);

  const entries = useMemo(() => CHANGELOG.slice(0, CHANGELOG_LIMIT), []);
  const auto = (settings?.updateMode ?? "manual") === "auto";

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
      setResult(await onCheck());
    } catch (error) {
      setResult({ hasUpdate: false, latest: null, current: appInfo?.version ?? "1.0.0", error: String(error) });
    } finally {
      setChecking(false);
    }
  };

  const checkText = () => {
    if (checking) return t("about.checking");
    if (!result) return "";
    if (result.error) return t("about.checkFailed", { message: result.error });
    if (result.hasUpdate && result.latest) return t("about.available", { version: result.latest });
    return t("about.uptodate", { version: result.current });
  };

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

      <section className="card about-changelog">
        <div className="about-changelog__head">
          <span className="detail-card__title">{t("about.changelog")}</span>
          <span className="row__hint">{t("about.changelogHint")}</span>
        </div>
        <div className="changelog-list" data-changelog>
          {entries.map((entry, index) => {
            const open = expanded === entry.version;
            return (
              <article className={"changelog-entry" + (open ? " is-open" : "")} key={entry.version} data-changelog-entry={entry.version}>
                <button
                  type="button"
                  className="changelog-entry__head"
                  data-action="changelog-toggle"
                  aria-expanded={open}
                  onClick={() => setExpanded(open ? null : entry.version)}
                >
                  <span className="changelog-entry__version">{entry.version}</span>
                  {index === 0 ? <span className="chip chip--tiny">{t("about.latest")}</span> : null}
                  <span className="changelog-entry__date">{entry.date}</span>
                  <span className="changelog-entry__toggle">{open ? t("about.collapse") : t("about.expand")}</span>
                </button>
                {open ? (
                  <div className="changelog-entry__body">
                    {entry.sections.map((section) => (
                      <div className="changelog-section" key={section.title}>
                        <div className="changelog-section__title">{section.title}</div>
                        <ul>
                          {section.items.map((item) => (
                            <li key={item}>{item}</li>
                          ))}
                        </ul>
                      </div>
                    ))}
                  </div>
                ) : null}
              </article>
            );
          })}
        </div>
      </section>

      <section className="card about-update">
        <div className="about-update__row">
          <div>
            <div className="detail-card__title">{t("about.update")}</div>
            {auto ? (
              <div className="row__hint" data-update-auto>
                {t("about.autoState")} · {t("about.autoHint")}
              </div>
            ) : (
              <div className="row__hint" data-update-status>
                {checkText() || t("about.checkReady")}
              </div>
            )}
          </div>
          <div className="about-update__actions">
            {auto ? null : (
              <button type="button" className="pill-outline" data-action="about-check" disabled={busy || checking} onClick={() => void runCheck()}>
                {checking ? t("about.checking") : t("about.check")}
              </button>
            )}
            {result?.hasUpdate && result.latest ? (
              <button type="button" className="pill-button" data-action="about-install" disabled={busy} onClick={() => void onInstall()}>
                {t("about.installNow")}
              </button>
            ) : null}
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
