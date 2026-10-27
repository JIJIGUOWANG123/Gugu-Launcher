import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BackgroundLayer } from "./components/BackgroundLayer";
import { type PageKey, TopBar } from "./components/TopBar";
import type { NewInstancePayload } from "./components/NewInstanceModal";
import { useDownloads } from "./lib/downloads-store";
import { useT } from "./lib/i18n";
import { api, normalizeAppearance, RUNTIME_MODE, subscribeLauncherEvents } from "./lib/tauri-api";
import { applyFont, applyTheme, normalizeAccent, readCachedFont, readCachedTheme } from "./lib/theme";
import type { Account, AppInfo, FontChoice, Instance, Settings, UpdateCheckResult, VersionManifest } from "./lib/types";
import { AboutPage } from "./pages/AboutPage";
import { AccountsPage } from "./pages/AccountsPage";
import { ContentPage } from "./pages/ContentPage";
import { HomePage } from "./pages/HomePage";
import { InstancesPage } from "./pages/InstancesPage";
import { MultiplayerPage } from "./pages/MultiplayerPage";
import { PlaceholderPage } from "./pages/PlaceholderPage";
import { SettingsPage } from "./pages/SettingsPage";
import { SkinsPage } from "./pages/SkinsPage";

const PAGE_TITLE_KEY: Record<PageKey, string> = {
  home: "nav.home",
  instances: "nav.instances",
  content: "nav.content",
  accounts: "nav.accounts",
  skins: "nav.skins",
  net: "nav.net",
  about: "nav.about",
  settings: "nav.settings",
};

export default function App() {
  const t = useT();
  const cached = useMemo(() => readCachedTheme(), []);
  const [page, setPage] = useState<PageKey>("home");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [versions, setVersions] = useState<VersionManifest | null>(null);
  const [instances, setInstances] = useState<Instance[]>([]);
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  const [fonts, setFonts] = useState<FontChoice[]>([]);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  // 别的页面点「去 设置 → 内容服务 填写」时，用它指定设置页落在哪一节
  const [settingsSection, setSettingsSection] = useState<string | null>(null);
  const { refresh: refreshDownloads } = useDownloads();

  // 连着弹两条提示时，旧的那条定时器不能把新的这条提前清掉
  const toastTimer = useRef<number | null>(null);
  const notify = useCallback((message: string) => {
    setToast(message);
    if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => {
      setToast(null);
      toastTimer.current = null;
    }, 3200);
  }, []);

  // 首次进入：拉设置 + 应用信息 + 版本清单 + 实例列表。单个失败不拖垮整个壳。
  useEffect(() => {
    let alive = true;
    (async () => {
      const [info, loaded] = await Promise.all([
        api.appInfo().catch(() => null),
        api.settingsGet().catch(() => null),
      ]);
      if (!alive) return;
      if (info) setAppInfo(info);
      if (loaded) {
        setSettings(normalizeAppearance(loaded));
        applyTheme(loaded.theme, loaded.accent);
        applyFont(loaded.uiFont);
      } else {
        applyTheme(cached.theme, cached.accent);
        applyFont(readCachedFont());
      }
      const [manifest, list, fontList, accountList] = await Promise.all([
        api.versionsList().catch(() => null),
        api.instancesList().catch(() => [] as Instance[]),
        api.fontsList().catch(() => [] as FontChoice[]),
        api.accountsList().catch(() => null),
      ]);
      if (!alive) return;
      if (manifest) setVersions(manifest);
      setInstances(list);
      setFonts(fontList);
      if (accountList) setAccounts(accountList);
    })();
    return () => {
      alive = false;
    };
  }, [cached.accent, cached.theme]);

  // 单实例：再点一次图标时后端发 app://second-instance，这里只弹提示，不动当前页面
  useEffect(() => {
    let off: (() => void) | null = null;
    let alive = true;
    void subscribeLauncherEvents((event) => {
      if (event.kind !== "second-instance") return;
      notify(event.payload.message || t("app.secondInstance"));
    })
      .then((unsubscribe) => {
        if (alive) off = unsubscribe;
        else unsubscribe();
      })
      .catch(() => undefined);
    return () => {
      alive = false;
      off?.();
    };
  }, [notify, t]);

  // 连接状态只留在控制台，界面上不再显示任何运行时徽标
  useEffect(() => {
    console.debug("[gugu] runtime mode:", RUNTIME_MODE);
  }, []);

  // 设置变化立刻落地到 DOM（不必等后端回包）；字体带上后端给的别名链
  useEffect(() => {
    if (!settings) return;
    applyTheme(settings.theme, normalizeAccent(settings.accent));
    const aliases = fonts.find((font) => font.family === settings.uiFont)?.aliases;
    applyFont(settings.uiFont, aliases);
  }, [settings, fonts]);

  // 选的字体已经不在系统里 → 提示一次「已回落到默认字体」
  const fontMissing = Boolean(settings && fonts.length > 0 && !fonts.some((font) => font.family.toLowerCase() === settings.uiFont.toLowerCase()));

  const updateSettings = useCallback(
    (patch: Partial<Settings>) => {
      setSettings((prev) => {
        if (!prev) return prev;
        const next = { ...prev, ...patch };
        api.settingsSet(next).catch((error: unknown) => {
          notify(String(error));
        });
        return next;
      });
    },
    [notify],
  );

  const refreshAccounts = useCallback(async () => {
    try {
      setAccounts(await api.accountsList());
    } catch (error) {
      notify(String(error));
    }
  }, [notify]);

  const refreshInstances = useCallback(async () => {
    try {
      setInstances(await api.instancesList());
    } catch (error) {
      notify(String(error));
    }
  }, [notify]);

  const launch = useCallback(
    async (instance: Instance) => {
      try {
        await api.instanceLaunch(instance.id);
      } catch (error) {
        notify(String(error));
      }
    },
    [notify],
  );

  const openFolder = useCallback(
    async (instance: Instance) => {
      try {
        await api.instanceOpenFolder(instance.id);
      } catch (error) {
        notify(String(error));
      }
    },
    [notify],
  );

  /** 新建实例：instance_create → instances_list 刷新 → version_install（进度走下载中心）。 */
  const createInstance = useCallback(
    async (payload: NewInstancePayload) => {
      const created = await api.instanceCreate({
        name: payload.name,
        mcVersion: payload.mcVersion,
        loader: payload.loader,
        memoryMb: payload.memoryMb,
      });
      await refreshInstances();
      await api.versionInstall(created.id, payload.mcVersion);
      notify(created.name + " · " + t("downloads.running"));
    },
    [notify, refreshInstances, t],
  );

  const deleteInstance = useCallback(
    async (instance: Instance) => {
      await api.instanceDelete(instance.id);
      await refreshInstances();
    },
    [refreshInstances],
  );

  /** 手动检查更新：结果交给关于页显示三种文案，异常也折成带 error 的结果。 */
  const checkUpdate = useCallback(async (): Promise<UpdateCheckResult> => {
    setUpdateBusy(true);
    try {
      return await api.updateCheck();
    } catch (error) {
      return {
        hasUpdate: false,
        latest: null,
        current: appInfo?.version ?? "1.0.0",
        error: String(error),
      };
    } finally {
      setUpdateBusy(false);
    }
  }, [appInfo?.version]);

  const installUpdate = useCallback(async () => {
    setUpdateBusy(true);
    try {
      const status = await api.updateInstall();
      notify(status.message);
    } catch (error) {
      notify(String(error));
    } finally {
      setUpdateBusy(false);
    }
  }, [notify]);

  const mainPage = () => {
    if (page === "home") {
      return (
        <HomePage
          versions={versions}
          instance={instances[0] ?? null}
          onLaunch={() => {
            const first = instances[0];
            if (first) void launch(first);
          }}
          onOpenInstances={() => setPage("instances")}
          notify={notify}
        />
      );
    }
    if (page === "instances") {
      return (
        <InstancesPage
          instances={instances}
          versions={versions}
          onCreate={createInstance}
          onDelete={deleteInstance}
          onLaunch={(instance) => void launch(instance)}
          onOpenFolder={(instance) => void openFolder(instance)}
          onRefresh={refreshInstances}
          notify={notify}
          onOpenJavaSettings={() => {
            setSettingsSection("java");
            setPage("settings");
          }}
        />
      );
    }
    if (page === "skins") {
      return (
        <SkinsPage notify={notify} settings={settings} onChangeSettings={updateSettings} />
      );
    }
    if (page === "accounts") {
      return (
        <AccountsPage
          accounts={accounts}
          onRefresh={refreshAccounts}
          notify={notify}
          onOpenSettings={() => {
            setSettingsSection("content");
            setPage("settings");
          }}
        />
      );
    }
    if (page === "content") {
      return (
        <ContentPage
          instances={instances}
          versions={versions}
          notify={notify}
          onInstalled={() => refreshDownloads()}
        />
      );
    }
    if (page === "net") {
      return <MultiplayerPage notify={notify} />;
    }
    if (page === "about") {
      return (
        <AboutPage
          appInfo={appInfo}
          settings={settings}
          onCheck={checkUpdate}
          onInstall={installUpdate}
          busy={updateBusy}
          notify={notify}
        />
      );
    }
    if (page === "settings" && settings) {
      return (
        <SettingsPage
          settings={settings}
          appInfo={appInfo}
          dataRoot={appInfo?.dataRoot ?? ""}
          fonts={fonts}
          fontMissing={fontMissing}
          onChange={updateSettings}
          onNotify={notify}
          initialSection={settingsSection}
        />
      );
    }
    return <PlaceholderPage titleKey={PAGE_TITLE_KEY[page]} />;
  };

  return (
    <div className={"app" + (settings?.appearance ? " app--has-bg" : "")}>
      {settings?.appearance ? <BackgroundLayer appearance={settings.appearance} /> : null}
      <TopBar
        active={page}
        onSelect={(next) => {
          setSettingsSection(null);
          setPage(next);
        }}
        theme={settings?.theme ?? cached.theme}
        onToggleTheme={() => {
          setSettingsSection(null);
          updateSettings({ theme: (settings?.theme ?? "light") === "dark" ? "light" : "dark" });
        }}
      />
      <div className="app-body">
        <main className={"app-content" + (page === "home" ? " app-content--home" : "")}>
          {page !== "home" && page !== "instances" && page !== "settings" ? (
            <h1 className="page-title">{t(PAGE_TITLE_KEY[page])}</h1>
          ) : null}
          {page === "instances" ? <h1 className="page-title">{t("nav.instances")}</h1> : null}
          {page === "settings" ? <h1 className="page-title">{t("settings.title")}</h1> : null}
          {import.meta.env.DEV ? (
            <p className="page-subtitle">
              <span className="chip">DEV · {RUNTIME_MODE}</span>
            </p>
          ) : null}
          {mainPage()}
        </main>
      </div>

      {toast ? (
        <div
          style={{
            position: "fixed",
            bottom: 20,
            left: "50%",
            transform: "translateX(-50%)",
            zIndex: 80,
            padding: "10px 18px",
            borderRadius: "var(--radius-pill)",
            background: "var(--surface)",
            border: "1px solid var(--accent)",
            color: "var(--accent-strong)",
            boxShadow: "var(--shadow-md)",
            fontSize: 13,
          }}
        >
          {toast}
        </div>
      ) : null}
    </div>
  );
}
