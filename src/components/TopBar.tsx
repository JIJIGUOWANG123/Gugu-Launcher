import { useState } from "react";
import { brand } from "../assets/brand";
import { icons } from "../assets/icons";
import { navIcons } from "../assets/icons/nav";
import { DownloadCenter } from "./DownloadCenter";
import { useDownloads } from "../lib/downloads-store";
import { useT } from "../lib/i18n";
import { isTauri } from "../lib/tauri-api";
import type { ThemeMode } from "../lib/types";

export type PageKey = "home" | "instances" | "net" | "content" | "accounts" | "skins" | "about" | "settings";

interface NavItem {
  key: PageKey;
  labelKey: string;
  icon: string;
}

/**
 * 顶部导航项，顺序照参照图：主页 / 实例 / 多人 / 资源 / 账号 / 皮肤 / 关于。
 * 前六个用用户给的彩色图标（64×64），「关于」暂无素材，走内联 SVG（见 ABOUT_ICON）。
 */
export const NAV_ITEMS: NavItem[] = [
  { key: "home", labelKey: "nav.home", icon: navIcons.home },
  { key: "instances", labelKey: "nav.instances", icon: navIcons.instances },
  { key: "net", labelKey: "nav.net", icon: navIcons.multiplayer },
  { key: "content", labelKey: "nav.content", icon: navIcons.content },
  { key: "accounts", labelKey: "nav.accounts", icon: navIcons.account },
  { key: "skins", labelKey: "nav.skins", icon: navIcons.skin },
];

interface Props {
  active: PageKey;
  onSelect: (key: PageKey) => void;
  theme: ThemeMode;
  onToggleTheme: () => void;
}

/**
 * 顶部标题栏 + 导航（照参照图：左侧 Logo 与中英双标题，中间导航，右侧主题/下载/设置/窗口按钮）。
 * 下载图标点开就是下载中心悬浮面板；角标数字读的是 lib/downloads-store 的同一份任务快照。
 * 自绘窗口按钮的前提是 tauri.conf.json 里 decorations: false。
 */
export function TopBar({ active, onSelect, theme, onToggleTheme }: Props) {
  const t = useT();
  const { activeCount } = useDownloads();
  const [downloadsOpen, setDownloadsOpen] = useState(false);

  const windowAction = async (action: "minimize" | "toggleMaximize" | "close") => {
    if (!isTauri()) return;
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();
    if (action === "minimize") await win.minimize();
    else if (action === "toggleMaximize") await win.toggleMaximize();
    else await win.close();
  };

  return (
    <header className="topbar">
      <div className="topbar__brand" data-tauri-drag-region>
        <img className="topbar__logo" src={brand.logo} alt="" data-brand-logo data-tauri-drag-region />
        <div className="topbar__titles" data-tauri-drag-region>
          <span className="topbar__title" data-tauri-drag-region>
            {t("app.name")}
          </span>
          <span className="topbar__subtitle" data-tauri-drag-region>
            {t("app.nameEn")}
          </span>
        </div>
      </div>

      <nav className="topbar__nav" aria-label={t("nav.main")} data-nav>
        {NAV_ITEMS.map((item) => (
          <button
            key={item.key}
            type="button"
            className={"topnav-item" + (active === item.key ? " is-active" : "")}
            data-page={item.key}
            aria-current={active === item.key ? "page" : undefined}
            title={t(item.labelKey)}
            onClick={() => onSelect(item.key)}
          >
            <img className="topnav-item__icon" src={item.icon} alt="" width={18} height={18} />
            <span className="topnav-item__label">{t(item.labelKey)}</span>
          </button>
        ))}
        {/* 关于：暂无彩色素材，保留内联 SVG（README TODO：关于图标待补） */}
        <button
          type="button"
          className={"topnav-item" + (active === "about" ? " is-active" : "")}
          data-page="about"
          aria-current={active === "about" ? "page" : undefined}
          title={t("nav.about")}
          onClick={() => onSelect("about")}
        >
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 10.5v6" />
            <circle cx="12" cy="7.6" r="0.9" fill="currentColor" stroke="none" />
          </svg>
          <span className="topnav-item__label">{t("nav.about")}</span>
        </button>
      </nav>

      <div className="topbar__spacer" data-tauri-drag-region />

      <div className="topbar__tools">
        <button
          type="button"
          className="icon-button"
          data-action="theme-toggle"
          title={theme === "light" ? t("titlebar.themeToDark") : t("titlebar.themeToLight")}
          onClick={onToggleTheme}
        >
          <img src={theme === "light" ? icons.moon : icons.sun} alt="" />
        </button>
        <button
          type="button"
          className={"icon-button topbar__download" + (downloadsOpen ? " is-active" : "")}
          data-action="downloads"
          title={t("titlebar.download")}
          aria-expanded={downloadsOpen}
          aria-haspopup="dialog"
          onClick={() => setDownloadsOpen((open) => !open)}
        >
          <img src={icons.download} alt="" />
          {activeCount > 0 ? (
            <span className="topbar__badge" data-badge>
              {activeCount > 99 ? "99+" : activeCount}
            </span>
          ) : null}
        </button>
        <button
          type="button"
          className={"icon-button" + (active === "settings" ? " is-active" : "")}
          data-action="settings"
          aria-current={active === "settings" ? "page" : undefined}
          title={t("nav.settings")}
          onClick={() => onSelect("settings")}
        >
          <img src={icons.gear} alt="" />
        </button>
      </div>

      <div className="topbar__window">
        <button type="button" className="icon-button" data-action="minimize" title={t("titlebar.minimize")} onClick={() => void windowAction("minimize")}>
          <span className="window-dash" aria-hidden="true" />
        </button>
        <button type="button" className="icon-button" data-action="maximize" title={t("titlebar.maximize")} onClick={() => void windowAction("toggleMaximize")}>
          <img src={icons.maximize} alt="" />
        </button>
        <button type="button" className="icon-button" data-action="close" title={t("titlebar.close")} onClick={() => void windowAction("close")}>
          <img src={icons.close} alt="" />
        </button>
      </div>

      {downloadsOpen ? <DownloadCenter variant="popover" onClose={() => setDownloadsOpen(false)} /> : null}
    </header>
  );
}
