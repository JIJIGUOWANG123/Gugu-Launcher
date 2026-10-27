import { useCallback, useEffect, useState } from "react";
import { InstanceIcon } from "../components/InstanceIcon";
import { formatStamp } from "../lib/format";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { Instance, NewsItem, VersionManifest } from "../lib/types";

interface Props {
  versions: VersionManifest | null;
  instance: Instance | null;
  onLaunch: () => void;
  onOpenInstances: () => void;
  notify: (message: string) => void;
}

// 只显示最新一条：请求也只取一条，省掉无谓的翻译与流量
const NEWS_LIMIT = 1;
const MINECRAFT_SITE = "https://www.minecraft.net/";

/**
 * 主页：官网新闻卡（只显示最新一条 + 原文切换）+ 版本信息 + 一键启动。
 * 新闻取不到时只显示「暂时取不到新闻」和一个打开官网的按钮，不阻塞其它卡片。
 */
export function HomePage({ versions, instance, onLaunch, onOpenInstances, notify }: Props) {
  const t = useT();
  const [news, setNews] = useState<NewsItem[] | null>(null);
  const [recent, setRecent] = useState<Instance[]>([]);
  const [failed, setFailed] = useState(false);
  const [showOriginal, setShowOriginal] = useState(false);

  const load = useCallback(async () => {
    setFailed(false);
    try {
      const list = await api.newsList({ limit: NEWS_LIMIT });
      // 自检用：记下这次新闻请求的参数（界面不显示）
      if (typeof window !== "undefined") {
        const host = window as unknown as Record<string, unknown>;
        const seen = Array.isArray(host.__guguNewsReq) ? (host.__guguNewsReq as unknown[]) : [];
        host.__guguNewsReq = [...seen, { limit: NEWS_LIMIT }].slice(-20);
      }
      setNews(list);
      setShowOriginal(false);
    } catch {
      setNews(null);
      setFailed(true);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let alive = true;
    void api
      .instancesRecent({ limit: 3 })
      .then((list) => {
        if (alive) setRecent(list);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const openExternal = async (url: string) => {
    try {
      await api.openExternal(url);
    } catch (error) {
      notify(String(error));
    }
  };

  // 只显示最新一条，不再翻页
  const current = news && news.length > 0 ? news[0] : null;
  // 自检用：gugu.mock.newsCover = off 时把封面当成没有，方便截「NEWS COVER 占位」那张图
  const hideCover =
    typeof window !== "undefined" && window.localStorage.getItem("gugu.mock.newsCover") === "off";

  const title = current ? (showOriginal ? current.titleOriginal ?? current.title : current.title) : "";
  const summary = current ? (showOriginal ? current.summaryOriginal ?? current.summary : current.summary) : "";

  return (
    <div className="home">
      <div className="home__scroll">
        <div className="card-grid">
      <section className="card news-card" data-news-card>
        <header className="news-card__head">
          <span className="loader-badge">{t("home.news")}</span>
        </header>

        {failed ? (
          <div className="news-empty" data-news-error>
            <span>{t("home.newsError")}</span>
            <button type="button" className="pill-outline" data-action="news-open-site" onClick={() => void openExternal(MINECRAFT_SITE)}>
              {t("home.newsOpenSite")}
            </button>
          </div>
        ) : news === null ? (
          <div className="news-skeleton" data-news-skeleton aria-hidden="true">
            <div className="news-skeleton__cover" />
            <div className="news-skeleton__lines">
              <span className="news-skeleton__bar" style={{ width: "40%" }} />
              <span className="news-skeleton__bar news-skeleton__bar--title" style={{ width: "82%" }} />
              <span className="news-skeleton__bar" style={{ width: "70%" }} />
              <span className="news-skeleton__bar" style={{ width: "55%" }} />
            </div>
          </div>
        ) : current ? (
          <div className="news-body" key={current.id}>
            <div className="news-cover">
              {current.coverUrl && !hideCover ? (
                <img className="news-cover__img" src={current.coverUrl} alt="" data-news-cover />
              ) : (
                <div className="news-cover__placeholder" data-news-cover-empty>
                  NEWS COVER
                </div>
              )}
            </div>
            <div className="news-text">
              <div className="news-text__head">
                <span className="chip" data-news-category>
                  {current.category}
                </span>
                {current.translated && current.titleOriginal ? (
                  <button
                    type="button"
                    className="link-button"
                    data-action="news-toggle-original"
                    onClick={() => setShowOriginal((value) => !value)}
                  >
                    {showOriginal ? t("content.showTranslated") : t("content.showOriginal")}
                  </button>
                ) : null}
              </div>
              <h2 className="news-title" data-news-title>
                {title}
              </h2>
              <p className="news-summary" data-news-summary>
                {summary}
              </p>
              <div className="news-foot">
                <span className="row__hint" data-news-date>
                  {current.date}
                </span>
                <button type="button" className="pill-button" data-action="news-read" onClick={() => void openExternal(current.readMoreUrl)}>
                  {t("home.newsRead")}
                </button>
              </div>
            </div>
          </div>
        ) : (
          <div className="news-empty" data-news-error>
            <span>{t("home.newsError")}</span>
          </div>
        )}
      </section>

      <section className="stat-pair">
        <div className="card stat">
          <span>{t("home.latestRelease")}：</span>
          <span>{versions ? versions.latestRelease : "—"}</span>
        </div>
        <div className="card stat">
          <span>{t("home.latestSnapshot")}：</span>
          <span>{versions ? versions.latestSnapshot : "—"}</span>
        </div>
      </section>

      <section className="card home-recent" data-home-recent>
        <div className="skin-library__head">
          <span className="detail-card__title">{t("home.recent")}</span>
          <button type="button" className="link-button" data-action="home-go-instances" onClick={onOpenInstances}>
            {t("home.recentGo")}
          </button>
        </div>
        {recent.length === 0 ? (
          <div className="row__hint" data-home-recent-empty>
            {t("home.recentEmpty")}
          </div>
        ) : (
          <div className="home-recent__grid">
            {recent.slice(0, 3).map((item, index) => (
              <button
                key={item.id}
                type="button"
                className="home-recent__card"
                data-home-recent-item={item.id}
                onClick={() => (index === 0 ? onLaunch() : onOpenInstances())}
              >
                <InstanceIcon className="home-recent__icon" icon={item.icon} alt={item.name} />
                <span className="home-recent__meta">
                  <span className="home-recent__name">{item.name}</span>
                  <span className="row__hint">
                    {item.mcVersion} · {item.loader || "Vanilla"}
                  </span>
                  <span className="row__hint">{t("home.lastPlayed", { time: formatStamp(item.lastPlayedAt) })}</span>
                </span>
              </button>
            ))}
          </div>
        )}
      </section>

        </div>
      </div>

      <section className="home__dock card" data-home-dock>
        <div className="home__dock-inner">
          <div className="home__dock-meta">
            <div className="home-instance__name" data-home-dock-name>
              {instance ? instance.name : t("home.recentEmpty")}
            </div>
            <div className="row__hint" data-home-current>
              {instance ? t("home.currentInstance", { name: instance.name, version: instance.mcVersion }) : ""}
            </div>
          </div>
          <button className="launch-button" data-action="home-launch" onClick={instance ? onLaunch : onOpenInstances}>
            <span className="play-triangle" aria-hidden="true" />
            {instance ? t("home.launchGame") : t("home.createInstance")}
          </button>
        </div>
      </section>

    </div>
  );
}
