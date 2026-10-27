import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FilterDrawer } from "../components/FilterDrawer";
import { Modal } from "../components/Modal";
import { ALL_VERSIONS, FILTERS_BY_KIND, labelToCategoryId } from "../lib/contentFilters";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type {
  ContentDependency,
  ContentItem,
  ContentKindTab,
  ContentProvider,
  ContentSearchQuery,
  ContentSort,
  Instance,
  ProjectDetail,
  ServerInstance,
  VersionChannel,
  VersionItem,
  VersionManifest,
} from "../lib/types";

interface Props {
  instances: Instance[];
  versions: VersionManifest | null;
  notify: (message: string) => void;
  onInstalled: () => void;
  /** 整合包创建完成后跳到实例页 */
  onOpenInstances: () => void;
  /** 插件没有服务端时提供跳转（多人 → 服务器） */
  onOpenServers: () => void;
}

/** 用户可选来源只有两个：MCIM 是 CurseForge 的镜像通道，不作为来源选项。 */
const PROVIDERS: Array<{ id: Extract<ContentProvider, "modrinth" | "curseforge">; labelKey: string }> = [
  { id: "modrinth", labelKey: "settings.contentSourceModrinth" },
  { id: "curseforge", labelKey: "settings.contentSourceCurseforge" },
];

/** CurseForge 没填 Key / 镜像不可用时后端会回 403 一类的错误，这里翻成人话。 */
function curseforgeKeyIssue(message: string): boolean {
  return /403|401|api[\s_-]?key|unauthor|forbidden|缺少.*key|需要.*key/i.test(message);
}

/**
 * 打开设置页：App 的回调这轮不在改动范围内（App 由其它分支在改），
 * 直接点顶栏那颗齿轮，等价于用户自己点「设置」。
 */
function openSettingsPage(): void {
  document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();
}

const KIND_TABS: ContentKindTab[] = ["mod", "modpack", "resourcepack", "shader", "datapack", "plugin"];
const SORTS: Array<{ id: ContentSort; labelKey: string }> = [
  { id: "downloads", labelKey: "content.sort.downloads" },
  { id: "updated", labelKey: "content.sort.updated" },
  { id: "relevance", labelKey: "content.sort.relevance" },
];
const PAGE_SIZES = [20, 40, 60];

/** 下载量缩写：291_500_000 → 291.5M */
function formatCount(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0";
  if (value >= 1_000_000_000) return (value / 1_000_000_000).toFixed(1) + "B";
  if (value >= 1_000_000) return (value / 1_000_000).toFixed(1) + "M";
  if (value >= 1_000) return (value / 1_000).toFixed(1) + "K";
  return String(Math.round(value));
}

/** 相对时间：4 天前 / 上个月 */
function relativeTime(iso: string): string {
  const time = new Date(iso).getTime();
  if (!Number.isFinite(time)) return "";
  const diff = Date.now() - time;
  const day = 86_400_000;
  if (diff < 3_600_000) return Math.max(1, Math.round(diff / 60_000)) + " 分钟前";
  if (diff < day) return Math.round(diff / 3_600_000) + " 小时前";
  if (diff < 30 * day) return Math.round(diff / day) + " 天前";
  if (diff < 365 * day) return Math.round(diff / (30 * day)) + " 个月前";
  return Math.round(diff / (365 * day)) + " 年前";
}

/** 页码序列：1 2 3 4 5 … 435 */
function pageNumbers(current: number, total: number): Array<number | "…"> {
  if (total <= 7) return Array.from({ length: total }, (_, index) => index + 1);
  const out: Array<number | "…"> = [];
  const push = (value: number | "…") => out.push(value);
  push(1);
  const start = Math.max(2, current - 2);
  const end = Math.min(total - 1, current + 2);
  if (start > 2) push("…");
  for (let page = start; page <= end; page += 1) push(page);
  if (end < total - 1) push("…");
  push(total);
  return out;
}

/** 来源不支持的类型：返回应该切过去的来源（null 表示兼容）。 */
function incompatibleProvider(provider: ContentProvider, kind: ContentKindTab): ContentProvider | null {
  // 已知不支持的组合：直接切源；后端还会拒的其它组合交给错误态（显示后端原文）
  const unsupported: Partial<Record<ContentProvider, ContentKindTab[]>> = {
    curseforge: ["datapack"],
  };
  const list = unsupported[provider];
  if (!list || !list.includes(kind)) return null;
  return "modrinth";
}

interface InstallTarget {
  title: string;
  projectId: string;
  /** 插件走服务端安装，其它类型走实例 */
  kind: ContentKindTab;
  versionId?: string;
}

/** 资源页：类型 tab + 搜索/实例版本 + 工具行 + 筛选 chips + 网格/列表 + 分页器。 */
export function ContentPage({ instances, versions, notify, onInstalled, onOpenInstances, onOpenServers }: Props) {
  const t = useT();

  const [provider, setProvider] = useState<ContentProvider>("modrinth");
  const [kind, setKind] = useState<ContentKindTab>("mod");
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const [sort, setSort] = useState<ContentSort>("downloads");
  const [pageSize, setPageSize] = useState(20);
  const [view, setView] = useState<"grid" | "list">("grid");
  const [page, setPage] = useState(1);
  // 已应用的筛选（chips 显示的就是这几项）
  const [loader, setLoader] = useState("");
  const [categoryLabels, setCategoryLabels] = useState<string[]>([]);
  const [mcVersion, setMcVersion] = useState("");
  const [filtersOpen, setFiltersOpen] = useState(false);
  // 抽屉里的草稿：点「完成」才写回上面三项并发请求
  const [draftLoader, setDraftLoader] = useState("");
  const [draftCategories, setDraftCategories] = useState<string[]>([]);
  const [draftVersion, setDraftVersion] = useState("");
  const [translateOn, setTranslateOn] = useState(true);
  const [instanceId, setInstanceId] = useState<number | null>(instances[0]?.id ?? null);

  const [items, setItems] = useState<ContentItem[] | null>(null);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [errorMessage, setErrorMessage] = useState("");
  const [translating, setTranslating] = useState(false);
  const [translations, setTranslations] = useState<Record<string, { title?: string; summary?: string }>>({});
  const [limited, setLimited] = useState(false);
  const [install, setInstall] = useState<InstallTarget | null>(null);
  const [detailItem, setDetailItem] = useState<ContentItem | null>(null);
  const seqRef = useRef(0);
  // 参数没变但用户又点了一次「搜索」/「重试」时，用它强制重新请求
  const [nonce, setNonce] = useState(0);

  const activeInstance = useMemo(
    () => instances.find((item) => item.id === instanceId) ?? instances[0] ?? null,
    [instances, instanceId],
  );

  // 插件要装到服务端：进来拉一次服务端列表（页面切换会重新挂载，够用）
  const [servers, setServers] = useState<ServerInstance[]>([]);
  useEffect(() => {
    let alive = true;
    void api
      .serverList()
      .then((list) => {
        if (alive) setServers(list);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  // 类别标签是界面文案，请求要用 Modrinth 的类别 id（一个标签可能拆成多个）
  const categoryIds = useMemo(() => categoryLabels.flatMap(labelToCategoryId), [categoryLabels]);
  const kindFilters = FILTERS_BY_KIND[kind];
  const versionOptions = useMemo(
    () => (versions?.versions ?? []).filter((item) => item.versionType === "release"),
    [versions],
  );

  useEffect(() => {
    let alive = true;
    void api
      .settingsGet()
      .then((loaded) => {
        if (!alive) return;

        // 旧设置里可能存着 mcim：它不是可选来源，直接回落 Modrinth
        if (loaded.resourceSource === "curseforge" || loaded.resourceSource === "modrinth") {
          setProvider(loaded.resourceSource);
        }
        // 翻译开关跟着设置走（老设置没有这个字段时按开启处理）
        setTranslateOn(loaded.translateEnabled !== false);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const load = useCallback(async () => {
    seqRef.current += 1;
    const seq = seqRef.current;
    setLoading(true);
    setFailed(false);
    setErrorMessage("");
    const request: ContentSearchQuery = {
      provider,
      kind,
      query: submitted,
      mcVersion: mcVersion || undefined,
      loader: loader || undefined,
      categories: categoryIds.length > 0 ? categoryIds : undefined,
      sort,
      page,
      pageSize,
    };
    try {
      const result = await api.contentSearch(request);
      if (seq !== seqRef.current) return;
      setItems(result.items);
      setTotal(result.total);
      setLoading(false);
      if (translateOn && result.items.length > 0) {
        setTranslating(true);
        try {
          const texts = result.items.map((item) => item.summaryOriginal ?? item.summary);
          const translated = await api.translateTexts(texts);
          if (seq !== seqRef.current) return;
          setLimited(translated.limited);
          const next: Record<string, { title?: string; summary?: string }> = {};
          result.items.forEach((item, index) => {
            const value = translated.results[index];
            if (value) next[item.id] = { summary: value };
          });
          setTranslations((prev) => ({ ...prev, ...next }));
        } catch {
          /* 翻译失败就用原文，不打扰用户 */
        } finally {
          if (seq === seqRef.current) setTranslating(false);
        }
      }
    } catch (error) {
      if (seq !== seqRef.current) return;
      const message = error instanceof Error ? error.message : String(error);
      setItems(null);
      setFailed(true);
      setErrorMessage(message);
      setLoading(false);
    }
  }, [provider, kind, submitted, loader, categoryIds, mcVersion, sort, page, pageSize, translateOn, nonce]);

  useEffect(() => {
    void load();
  }, [load]);

  const goPage = (next: number) => {
    if (next < 1) return;
    setPage(next);
    const node = document.querySelector(".app-content");
    if (node && typeof node.scrollTo === "function") node.scrollTo({ top: 0 });
    if (typeof window !== "undefined") window.scrollTo({ top: 0 });
  };

  /** 清掉三组筛选（草稿与应用值一起清），搜索词留着 */
  const resetPanel = () => {
    setDraftLoader("");
    setDraftCategories([]);
    setDraftVersion("");
    setLoader("");
    setCategoryLabels([]);
    setMcVersion("");
    setPage(1);
  };

  /** 翻译开关：立刻写设置并重新请求，卡片随之在译文 / 原文之间切换。 */
  const toggleTranslate = async () => {
    const next = !translateOn;
    setTranslateOn(next);
    setPage(1);
    try {
      const current = await api.settingsGet();
      await api.settingsSet({ ...current, translateEnabled: next });
    } catch (error) {
      notify(String(error));
    }
  };

  /** 工具行的「清除全部」：筛选与搜索词一起清 */
  const resetFilters = () => {
    resetPanel();
    setSubmitted("");
    setQuery("");
    setPage(1);
  };

  const openFilters = () => {
    setDraftLoader(loader);
    setDraftCategories(categoryLabels);
    setDraftVersion(mcVersion);
    setFiltersOpen(true);
  };

  /** 「完成」：草稿落地并发一次请求，然后关抽屉 */
  const applyFilters = () => {
    setLoader(draftLoader);
    setCategoryLabels(draftCategories);
    setMcVersion(draftVersion);
    setPage(1);
    setNonce((value) => value + 1);
    setFiltersOpen(false);
  };

  const chips = useMemo(() => {
    const out: Array<{ key: string; label: string; clear: () => void }> = [];
    if (loader) out.push({ key: "loader", label: t("content.chip.loader", { value: loader }), clear: () => setLoader("") });
    if (mcVersion) out.push({ key: "version", label: t("content.chip.version", { value: mcVersion }), clear: () => setMcVersion("") });
    for (const category of categoryLabels) {
      out.push({
        key: "cat-" + category,
        label: category,
        clear: () => setCategoryLabels((prev) => prev.filter((item) => item !== category)),
      });
    }
    if (submitted) out.push({ key: "q", label: t("content.chip.query", { value: submitted }), clear: () => { setSubmitted(""); setQuery(""); } });
    return out;
  }, [loader, mcVersion, categoryLabels, submitted, t]);

  /** 插件：装到选中的服务端（进度走下载中心）。 */
  const installToServer = async (serverId: string) => {
    if (!install) return;
    try {
      await api.serverContentInstall({ serverId, provider, projectId: install.projectId, versionId: install.versionId });
      notify(t("content.installedServer", { name: servers.find((server) => server.id === serverId)?.name ?? serverId }));
      setInstall(null);
    } catch (error) {
      notify(String(error));
    }
  };

  const installTo = async (id: number) => {
    if (!install) return;
    try {
      await api.modInstall({ provider, projectId: install.projectId, versionId: install.versionId, instanceId: id });
      notify(t("content.installed", { name: install.title }));
      setInstall(null);
      onInstalled();
    } catch (error) {
      notify(String(error));
    }
  };

  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  const cardFor = (item: ContentItem) => {
    // 关掉自动翻译 → 一律用后端给的原始字段，绝不显示译文
    const translated = translateOn ? translations[item.id] : undefined;
    const title = translateOn
      ? (translated?.title ?? item.title ?? item.titleOriginal ?? "")
      : (item.titleOriginal ?? item.title ?? "");
    const summary = translateOn
      ? (translated?.summary ?? item.summary ?? item.summaryOriginal ?? "")
      : (item.summaryOriginal ?? item.summary ?? "");
    const author = item.author || "";
    const tags = [...(item.loaders ?? []), ...(item.categories ?? [])].filter(Boolean).slice(0, 4);
    return { title, summary, author, tags };
  };

  return (
    <div className="content-page">
      <nav className="content-tabs" role="tablist">
        {KIND_TABS.map((item) => (
          <button
            key={item}
            type="button"
            role="tab"
            aria-selected={kind === item}
            className={"content-tab" + (kind === item ? " is-active" : "")}
            data-content-tab={item}
            onClick={() => {
              // 来源与类型不兼容时就地切到兼容的来源，别让用户撞一个「空列表」
              const fallback = incompatibleProvider(provider, item);
              if (fallback) {
                setProvider(fallback);
                notify(t("content.providerSwitched", { from: t("content.source." + provider), to: t("content.source." + fallback), kind: t("content.kind." + item) }));
              }
              setKind(item);
              // 每种类型的筛选项不同，换 tab 时把三组选择清干净，避免带着不存在的类别去请求
              resetPanel();
              setPage(1);
            }}
          >
            {t("content.kind." + item)}
          </button>
        ))}
      </nav>

      <section className="card content-toolbar">
        <div className="content-search">
          <input
            type="text"
            value={query}
            placeholder={t("content.searchPlaceholder")}
            data-content-search
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                setSubmitted(query.trim());
                setPage(1);
              }
            }}
          />
          <select
            className="content-instance"
            data-content-instance
            value={instanceId ?? ""}
            onChange={(event) => setInstanceId(Number(event.target.value) || null)}
          >
            {instances.length === 0 ? <option value="">{t("content.noInstance")}</option> : null}
            {instances.map((instance) => (
              <option key={instance.id} value={instance.id}>
                {instance.name} ({instance.mcVersion} {instance.loader.toLowerCase()})
              </option>
            ))}
          </select>
          <button
            type="button"
            className="pill-button"
            data-action="content-search"
            onClick={() => {
              setSubmitted(query.trim());
              setPage(1);
              setNonce((value) => value + 1);
            }}
          >
            {t("content.search")}
          </button>
        </div>

        <div className="content-tools">
          <label className="content-tool">
            <span>{t("content.sort")}</span>
            <select data-content-sort value={sort} onChange={(event) => { setSort(event.target.value as ContentSort); setPage(1); }}>
              {SORTS.map((item) => (
                <option key={item.id} value={item.id}>
                  {t(item.labelKey)}
                </option>
              ))}
            </select>
          </label>
          <label className="content-tool">
            <span>{t("content.pageSize")}</span>
            <select data-content-pagesize value={pageSize} onChange={(event) => { setPageSize(Number(event.target.value)); setPage(1); }}>
              {PAGE_SIZES.map((size) => (
                <option key={size} value={size}>
                  {size}
                </option>
              ))}
            </select>
          </label>
          <div className="content-view">
            <button
              type="button"
              className={"icon-button" + (view === "grid" ? " is-active" : "")}
              data-content-view="grid"
              title={t("content.view.grid")}
              aria-label={t("content.view.grid")}
              onClick={() => setView("grid")}
            >
              <span className="view-glyph view-glyph--grid" aria-hidden="true" />
            </button>
            <button
              type="button"
              className={"icon-button" + (view === "list" ? " is-active" : "")}
              data-content-view="list"
              title={t("content.view.list")}
              aria-label={t("content.view.list")}
              onClick={() => setView("list")}
            >
              <span className="view-glyph view-glyph--list" aria-hidden="true" />
            </button>
          </div>
          <button
            type="button"
            className={"pill-outline" + (filtersOpen ? " is-active" : "")}
            data-action="content-filters"
            onClick={() => {
              if (filtersOpen) setFiltersOpen(false);
              else openFilters();
            }}
          >
            {t("content.filters")}
          </button>
          <label className="content-tool">
            <span>{t("content.provider")}</span>
            <select data-provider-select value={provider} onChange={(event) => { setProvider(event.target.value as ContentProvider); setPage(1); }}>
              {PROVIDERS.map((item) => (
                <option key={item.id} value={item.id}>
                  {t(item.labelKey)}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            className={"pill-outline" + (translateOn ? " is-active" : "")}
            data-action="content-translate-toggle"
            data-translate-on={translateOn ? "1" : "0"}
            onClick={() => void toggleTranslate()}
          >
            {translateOn ? t("content.translateOn") : t("content.translateOff")}
          </button>
          {translating ? (
            <span className="row__hint" data-content-translating>
              {t("content.translating")}
            </span>
          ) : null}
        </div>

        {filtersOpen ? (
          <FilterDrawer
            title={t("content.filters")}
            onClose={() => setFiltersOpen(false)}
            footer={
              <>
                <button type="button" className="pill-outline" data-action="filter-reset" onClick={resetPanel}>
                  {t("content.filtersReset")}
                </button>
                <button type="button" className="pill-button" data-action="filter-apply" onClick={applyFilters}>
                  {t("content.filtersDone")}
                </button>
              </>
            }
          >
            <div className="filter-group" data-filter-group="version">
              <span className="filter-group__label">{t("content.gameVersion")}</span>
              <select data-filter-version value={draftVersion} onChange={(event) => setDraftVersion(event.target.value)}>
                <option value="">{ALL_VERSIONS}</option>
                {versionOptions.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.id}
                  </option>
                ))}
              </select>
            </div>

            {kindFilters.loaders ? (
              <div className="filter-group" data-filter-group="loader">
                <span className="filter-group__label">{t("content.loader")}</span>
                <div className="chip-row">
                  {(kindFilters.loaders ?? []).map((option) => {
                    const isAll = option.ids.length === 0;
                    const active = isAll ? draftLoader === "" : draftLoader === option.label;
                    return (
                      <button
                        key={option.label}
                        type="button"
                        className={"chip chip--button" + (active ? " is-active" : "")}
                        data-filter-loader={option.label}
                        data-filter-all={isAll ? "1" : undefined}
                        onClick={() => setDraftLoader(isAll ? "" : option.label)}
                      >
                        {option.label}
                      </button>
                    );
                  })}
                </div>
              </div>
            ) : null}

            <div className="filter-group" data-filter-group="category">
              <span className="filter-group__label">{t("content.categories")}</span>
              <div className="chip-row">
                {kindFilters.categories.map((option) => {
                  const isAll = option.ids.length === 0;
                  const active = isAll ? draftCategories.length === 0 : draftCategories.includes(option.label);
                  return (
                    <button
                      key={option.label}
                      type="button"
                      className={"chip chip--button" + (active ? " is-active" : "")}
                      data-filter-category={option.label}
                      data-filter-all={isAll ? "1" : undefined}
                      onClick={() => {
                        if (isAll) {
                          setDraftCategories([]);
                          return;
                        }
                        setDraftCategories((prev) =>
                          prev.includes(option.label) ? prev.filter((value) => value !== option.label) : [...prev, option.label],
                        );
                      }}
                    >
                      {option.label}
                    </button>
                  );
                })}
              </div>
            </div>
          </FilterDrawer>
        ) : null}

        {chips.length > 0 ? (
          <div className="content-chips" data-content-chips>
            {chips.map((chip) => (
              <span className="chip" key={chip.key}>
                {chip.label}
                <button type="button" className="chip__close" data-chip-clear={chip.key} onClick={chip.clear} aria-label={t("common.delete")}>
                  ×
                </button>
              </span>
            ))}
            <button type="button" className="link-button" data-action="content-clear-filters" onClick={resetFilters}>
              {t("content.clearAll")}
            </button>
          </div>
        ) : null}
      </section>

      {loading ? (
        <div className={"content-grid" + (view === "list" ? " content-grid--list" : "")} data-content-skeleton>
          {Array.from({ length: Math.min(pageSize, 8) }, (_, index) => (
            <div className="card content-card content-card--skeleton" key={index}>
              <span className="skel skel--icon" />
              <div className="content-card__body">
                <span className="skel skel--title" />
                <span className="skel skel--line" />
                <span className="skel skel--line skel--short" />
              </div>
            </div>
          ))}
        </div>
      ) : failed ? (
        <div className="empty-hint content-error" data-content-error>
          <strong data-content-error-title>
            {curseforgeKeyIssue(errorMessage)
              ? t("content.cfKeyRequired")
              : /不支持|unsupported/i.test(errorMessage)
                ? t("content.unsupportedKind")
                : t("content.loadFailed")}
          </strong>
          {errorMessage ? (
            // 403 / 缺 Key 时不要把后端原文甩给用户，换成一句人话
            <span className="row__hint" data-content-error-message>
              {curseforgeKeyIssue(errorMessage) ? t("content.cfKeyRequiredHint") : errorMessage}
            </span>
          ) : null}
          <div className="content-error__actions">
            {curseforgeKeyIssue(errorMessage) ? (
              <button type="button" className="pill-button" data-action="content-go-settings" onClick={openSettingsPage}>
                {t("content.goSettings")}
              </button>
            ) : null}
            <button
              type="button"
              className="pill-outline"
              data-action="content-switch-provider"
              onClick={() => {
                setProvider((prev) => (prev === "modrinth" ? "curseforge" : "modrinth"));
                setPage(1);
              }}
            >
              {t("content.switchProvider")}
            </button>
            <button type="button" className="pill-outline" data-action="content-retry" onClick={() => void load()}>
              {t("content.retry")}
            </button>
          </div>
        </div>
      ) : items === null || items.length === 0 ? (
        <div className="empty-hint" data-content-empty>
          {t("content.empty")}
          <button type="button" className="pill-outline" data-action="content-clear-filters" onClick={resetFilters}>
            {t("content.clearFilters")}
          </button>
        </div>
      ) : (
        <div className={"content-grid" + (view === "list" ? " content-grid--list" : "")} data-content-grid data-content-view-mode={view}>
          {items.map((item) => {
            const data = cardFor(item);
            return (
              <article className="card content-card" key={item.id} data-content-card={item.id}>
                {item.iconUrl ? (
                  <img className="content-card__icon" src={item.iconUrl} alt="" />
                ) : (
                  <span className="content-card__icon content-card__icon--letter">{data.title.slice(0, 1)}</span>
                )}
                <div className="content-card__body">
                  <div className="content-card__head">
                    <button type="button" className="content-card__title" data-action="content-detail" onClick={() => setDetailItem(item)}>
                      {data.title}
                    </button>
                    {data.author ? <span className="content-card__author">{data.author}</span> : null}
                    {item.updatedAt ? <span className="content-card__time">{relativeTime(item.updatedAt)}</span> : null}
                  </div>
                  <p className="content-card__summary" data-content-summary>
                    {data.summary}
                  </p>
                  {data.tags.length > 0 ? (
                    <div className="content-card__tags">
                      {data.tags.map((tag) => (
                        <span className="chip chip--tiny" key={tag}>
                          {tag}
                        </span>
                      ))}
                    </div>
                  ) : null}
                  <div className="content-card__meta">
                    <span className={"source-badge source-badge--" + item.provider}>{t("content.source." + item.provider)}</span>
                    <span>↓ {formatCount(item.downloads)}</span>
                    {item.follows && item.follows > 0 ? <span>♥ {formatCount(item.follows)}</span> : null}
                  </div>
                </div>
                <div className="content-card__actions">
                  <button
                    type="button"
                    className="pill-button"
                    data-action="content-install"
                    onClick={() => setInstall({ title: data.title, projectId: item.id, kind: item.kind })}
                  >
                    {t(item.kind === "plugin" ? "content.installToServer" : "content.installShort")}
                  </button>
                </div>
              </article>
            );
          })}
        </div>
      )}

      {!loading && !failed && total > 0 ? (
        <div className="content-pager" data-content-pager>
          <span className="row__hint" data-content-total>
            {t("content.totalCount", { total })}
          </span>
          <div className="content-pager__pages">
            <button type="button" className="icon-button" data-pager-prev disabled={page <= 1} onClick={() => goPage(page - 1)} aria-label={t("content.prevPage")}>
              ‹
            </button>
            {pageNumbers(page, totalPages).map((value, index) =>
              value === "…" ? (
                <span className="content-pager__gap" key={"gap-" + index}>
                  …
                </span>
              ) : (
                <button
                  type="button"
                  key={value}
                  className={"content-pager__page" + (value === page ? " is-active" : "")}
                  data-pager-page={value}
                  onClick={() => goPage(value)}
                >
                  {value}
                </button>
              ),
            )}
            <button type="button" className="icon-button" data-pager-next disabled={page >= totalPages} onClick={() => goPage(page + 1)} aria-label={t("content.nextPage")}>
              ›
            </button>
          </div>
        </div>
      ) : null}

      {limited ? (
        <span className="notice notice--inline" data-translate-limited>
          {t("content.translateLimited")}
        </span>
      ) : null}

      {detailItem ? (
        <ContentDetailModal
          item={detailItem}
          instances={instances}
          servers={servers}
          onClose={() => setDetailItem(null)}
          onInstalled={onInstalled}
          onGoInstances={() => {
            setDetailItem(null);
            onOpenInstances();
          }}
          onGoServers={() => {
            setDetailItem(null);
            onOpenServers();
          }}
          notify={notify}
        />
      ) : null}

      {install ? (
        <Modal
          title={t("content.installPick")}
          onClose={() => setInstall(null)}
          footer={
            <button type="button" className="pill-outline" onClick={() => setInstall(null)}>
              {t("common.cancel")}
            </button>
          }
        >
          <p className="row__hint">{install.title}</p>
          {install.kind === "plugin" ? (
            servers.length === 0 ? (
              <div className="empty-hint" data-install-no-server>
                {t("content.noServer")}
                <button
                  type="button"
                  className="pill-outline"
                  data-action="install-go-servers"
                  onClick={() => {
                    setInstall(null);
                    onOpenServers();
                  }}
                >
                  {t("content.goServers")}
                </button>
              </div>
            ) : (
              <div className="install-list">
                {servers.map((server) => (
                  <button
                    key={server.id}
                    type="button"
                    className="install-row"
                    data-install-server={server.id}
                    onClick={() => void installToServer(server.id)}
                  >
                    <span className="install-row__name">{server.name}</span>
                    <span className="install-row__meta">
                      {t("server.core." + server.core)} {server.mcVersion}
                    </span>
                  </button>
                ))}
              </div>
            )
          ) : instances.length === 0 ? (
            <div className="empty-hint">{t("content.noInstance")}</div>
          ) : (
            <div className="install-list">
              {instances.map((instance) => (
                <button key={instance.id} type="button" className="install-row" data-install-instance={instance.id} onClick={() => void installTo(instance.id)}>
                  <span className="install-row__name">{instance.name}</span>
                  <span className="install-row__meta">
                    {instance.mcVersion} · {instance.loader || "Vanilla"}
                  </span>
                </button>
              ))}
            </div>
          )}
        </Modal>
      ) : null}

      {activeInstance ? null : null}
      {versions ? null : null}
    </div>
  );
}

interface DetailProps {
  item: ContentItem;
  instances: Instance[];
  servers: ServerInstance[];
  onClose: () => void;
  onInstalled: () => void;
  onGoInstances: () => void;
  onGoServers: () => void;
  notify: (message: string) => void;
}

const CHANNELS: Array<{ id: VersionChannel | "all"; labelKey: string }> = [
  { id: "all", labelKey: "content.channel.all" },
  { id: "release", labelKey: "content.channel.release" },
  { id: "beta", labelKey: "content.channel.beta" },
  { id: "alpha", labelKey: "content.channel.alpha" },
];

/** 资源详情：居中大弹窗（图标 + 信息 + 快捷操作 + 实例 + 版本页签 + 依赖 + 一键安装）。 */
function ContentDetailModal({ item, instances, servers, onClose, onInstalled, onGoInstances, onGoServers, notify }: DetailProps) {
  const t = useT();
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [versions, setVersions] = useState<VersionItem[] | null>(null);
  const [deps, setDeps] = useState<ContentDependency[]>([]);
  const [channel, setChannel] = useState<VersionChannel | "all">("all");
  const [selected, setSelected] = useState<string | null>(null);
  const [instanceId, setInstanceId] = useState<number | null>(instances[0]?.id ?? null);
  const [installing, setInstalling] = useState(false);
  const [installed, setInstalled] = useState(false);
  // 整合包：直接建实例
  const isModpack = item.kind === "modpack";
  // 插件：装到服务端
  const isPlugin = item.kind === "plugin";
  const [serverId, setServerId] = useState<string>(servers[0]?.id ?? "");
  const [packName, setPackName] = useState(item.title);
  const [packMemory, setPackMemory] = useState(2048);
  const [packError, setPackError] = useState("");
  const [packDone, setPackDone] = useState<{ name: string } | null>(null);
  // CurseForge 缺 Key / 镜像不可用：不改报错原文，弹窗里给友好提示 + 去设置
  const [keyIssue, setKeyIssue] = useState("");

  /** 统一的失败处理：缺 Key 类错误进弹窗提示，其它照旧走 toast。 */
  const reportError = (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    if (curseforgeKeyIssue(message)) {
      setKeyIssue(message);
      return;
    }
    notify(message);
  };

  useEffect(() => {
    let alive = true;
    void api
      .contentDetail(item.provider, item.id)
      .then((value) => {
        if (alive) setDetail(value);
      })
      .catch((error) => notify(String(error)));
    void api
      .contentDependencies(item.provider, item.id)
      .then((list) => {
        if (alive) setDeps(list);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [item.provider, item.id, notify]);

  useEffect(() => {
    let alive = true;
    setVersions(null);
    void api
      .contentVersions(item.provider, item.id, undefined, undefined, channel === "all" ? undefined : channel)
      .then((list) => {
        if (!alive) return;
        setVersions(list);
        setSelected(list[0]?.versionId ?? null);
      })
      .catch((error) => {
        if (alive) notify(String(error));
      });
    return () => {
      alive = false;
    };
  }, [item.provider, item.id, channel, notify]);

  const install = async () => {
    if (!instanceId) return;
    setInstalling(true);
    try {
      await api.modInstall({ provider: item.provider, projectId: item.id, versionId: selected ?? undefined, instanceId, withDependencies: true });
      setInstalled(true);
      onInstalled();
      notify(t("content.installed", { name: item.title }));
    } catch (error) {
      reportError(error);
    } finally {
      setInstalling(false);
    }
  };

  const installToServer = async () => {
    if (!serverId) return;
    setInstalling(true);
    try {
      await api.serverContentInstall({ serverId, provider: item.provider, projectId: item.id, versionId: selected ?? undefined });
      setInstalled(true);
      onInstalled();
      notify(t("content.installedServer", { name: servers.find((server) => server.id === serverId)?.name ?? serverId }));
    } catch (error) {
      reportError(error);
    } finally {
      setInstalling(false);
    }
  };

  const copyName = async () => {
    try {
      await navigator.clipboard.writeText(item.title);
      notify(t("content.copied"));
    } catch {
      notify(t("content.copyFailed"));
    }
  };

  const requiredCount = deps.filter((dep) => dep.kind === "required").length;

  /** 整合包：创建实例并安装（进度进下载中心）。 */
  const createPackInstance = async () => {
    const name = packName.trim() || item.title;
    setInstalling(true);
    setPackError("");
    try {
      await api.modpackInstall({ provider: item.provider, projectId: item.id, versionId: selected ?? undefined, instanceName: name, memoryMb: packMemory });
      setPackDone({ name });
      setInstalled(true);
      onInstalled();
      notify(t("content.modpackCreated", { name }));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setPackError(curseforgeKeyIssue(message) ? t("content.cfKeyRequiredHint") : message);
      // 实例可能已经建出来了（只是没装完），刷新列表让「整合包未装完」小标出现
      onInstalled();
      notify(t("content.modpackCreateFailed"));
    } finally {
      setInstalling(false);
    }
  };

  const chosenVersion = versions?.find((version) => version.versionId === selected) ?? null;

  return (
    <Modal
      title={item.title}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="pill-outline" onClick={onClose}>
            {t("common.close")}
          </button>
          {packDone ? (
            <button type="button" className="pill-outline" data-action="detail-go-instances" onClick={onGoInstances}>
              {t("content.goInstances")}
            </button>
          ) : null}
          <button
            type="button"
            className="pill-button"
            data-action="detail-install"
            disabled={installing || installed || (isModpack ? false : isPlugin ? !serverId : !instanceId)}
            onClick={() => void (isModpack ? createPackInstance() : isPlugin ? installToServer() : install())}
          >
            {installed
              ? t(isModpack ? "content.packDone" : "content.installedBadge")
              : installing
                ? t("content.installing")
                : t(isModpack ? "content.createAndInstall" : isPlugin ? "content.installToServer" : "content.oneClickInstall")}
          </button>
        </>
      }
    >
      <div className="cdetail" data-content-detail-modal>
        {keyIssue ? (
          <div className="notice" data-detail-key-issue>
            <strong>{t("content.cfKeyRequired")}</strong>
            <span>{t("content.cfKeyRequiredHint")}</span>
            <button type="button" className="link-button" data-action="detail-go-settings" onClick={openSettingsPage}>
              {t("content.goSettings")}
            </button>
          </div>
        ) : null}
        <div className="cdetail__head">
          {item.iconUrl ? <img className="cdetail__icon" src={item.iconUrl} alt="" /> : null}
          <div className="cdetail__info">
            <div className="cdetail__title-row">
              <span className="cdetail__title">{detail?.title ?? item.title}</span>
              <span className="chip chip--tiny" data-detail-kind>{t("content.kind." + item.kind)}</span>
            </div>
            <div className="row__hint" data-detail-meta>
              {item.author || "—"} · ↓ {formatCount(item.downloads)} · {relativeTime(item.updatedAt)}
            </div>
            <div className="row__hint cdetail__summary">{(detail?.summary ?? item.summary).slice(0, 120)}</div>
          </div>
        </div>

        <div className="cdetail__actions">
          <button type="button" className="pill-outline pill-outline--tiny" data-action="detail-wiki" onClick={() => void api.openExternal(detail?.url ?? item.iconUrl ?? "").catch((error) => notify(String(error)))}>
            {t("content.wiki")}
          </button>
          <button type="button" className="pill-outline pill-outline--tiny" data-action="detail-open" onClick={() => void api.openExternal(detail?.url ?? "").catch((error) => notify(String(error)))}>
            {t("content.openInBrowser")}
          </button>
          <button type="button" className="pill-outline pill-outline--tiny" data-action="detail-copy" onClick={() => void copyName()}>
            {t("content.copyName")}
          </button>
        </div>

        {isModpack ? (
          <div className="cdetail__pack" data-detail-pack>
            <div className="field">
              <label htmlFor="pack-name">{t("content.packName")}</label>
              <input id="pack-name" type="text" data-detail-pack-name value={packName} onChange={(event) => setPackName(event.target.value)} />
            </div>
            <div className="field">
              <label htmlFor="pack-memory">
                {t("content.packMemory")}
                <span className="field__value">{packMemory} MB</span>
              </label>
              <input
                id="pack-memory"
                type="range"
                min={1024}
                max={8192}
                step={512}
                value={packMemory}
                data-detail-pack-memory
                onChange={(event) => setPackMemory(Number(event.target.value))}
              />
            </div>
            <div className="field">
              <span className="row__hint">{t("content.packVersion")}</span>
              <div className="mono" data-detail-pack-version>
                {(chosenVersion?.gameVersions[0] ?? item.title) + " · " + (chosenVersion?.loaders.join(" / ") ?? "—")}
              </div>
            </div>
          </div>
        ) : isPlugin ? (
          <div className="field">
            <label htmlFor="detail-server">{t("content.installServerPick")}</label>
            {servers.length === 0 ? (
              <div className="empty-hint" data-detail-no-server>
                {t("content.noServer")}
                <button type="button" className="pill-outline" data-action="detail-go-servers" onClick={onGoServers}>
                  {t("content.goServers")}
                </button>
              </div>
            ) : (
              <select id="detail-server" data-detail-server value={serverId} onChange={(event) => setServerId(event.target.value)}>
                {servers.map((server) => (
                  <option key={server.id} value={server.id}>
                    {server.name} · {t("server.core." + server.core)} {server.mcVersion}
                  </option>
                ))}
              </select>
            )}
          </div>
        ) : (
          <div className="field">
            <label htmlFor="detail-instance">{t("content.installPick")}</label>
            <select id="detail-instance" data-detail-instance value={instanceId ?? ""} onChange={(event) => setInstanceId(Number(event.target.value) || null)}>
              {instances.length === 0 ? <option value="">{t("content.noInstance")}</option> : null}
              {instances.map((instance) => (
                <option key={instance.id} value={instance.id}>
                  {instance.name} ({instance.mcVersion} {instance.loader.toLowerCase()})
                </option>
              ))}
            </select>
          </div>
        )}

        <div className="cdetail__versions">
          <div className="cdetail__versions-head">
            <span className="detail-card__title">{t("content.versions")}</span>
            <div className="chip-row">
              {CHANNELS.map((entry) => (
                <button
                  key={entry.id}
                  type="button"
                  className={"chip chip--button" + (channel === entry.id ? " is-active" : "")}
                  data-detail-channel={entry.id}
                  onClick={() => setChannel(entry.id)}
                >
                  {t(entry.labelKey)}
                </button>
              ))}
            </div>
          </div>
          {versions === null ? (
            <div className="row__hint">{t("common.loading")}</div>
          ) : versions.length === 0 ? (
            <div className="empty-hint" data-detail-no-version>
              {t("content.noVersions")}
            </div>
          ) : (
            <div className="cdetail__version-list">
              {versions.map((version) => {
                const kind = version.channel ?? "release";
                return (
                  <button
                    key={version.versionId}
                    type="button"
                    className={"cdetail__version" + (selected === version.versionId ? " is-active" : "")}
                    data-detail-version={version.versionId}
                    onClick={() => setSelected(version.versionId)}
                  >
                    <span className="cdetail__version-name">
                      {version.name} for {version.loaders.join(" / ")} {version.gameVersions.join(" / ")}
                    </span>
                    <span className={"version-kind version-kind--" + kind} data-detail-version-kind={kind}>
                      {t("content.channel." + kind)}
                    </span>
                    <span className="row__hint">{version.serverLabel ?? "Server, " + version.gameVersions[0]}</span>
                    <span className="row__hint">{version.publishedAt.slice(0, 10)}</span>
                  </button>
                );
              })}
            </div>
          )}
        </div>

        <div className="cdetail__deps" hidden={isModpack}>
          <span className="detail-card__title">{t("content.dependencies")}</span>
          {isModpack ? null : deps.length === 0 ? (
            <span className="row__hint" data-detail-no-deps>
              {t("content.noDependencies")}
            </span>
          ) : (
            <>
              <span className="row__hint" data-detail-deps-hint>
                {t("content.depsHint", { count: requiredCount })}
              </span>
              <div className="chip-row">
                {deps.map((dep) => (
                  <span key={dep.projectId} className={"chip dep-chip dep-chip--" + dep.kind} data-detail-dep={dep.name}>
                    {dep.kind === "required" ? t("content.dep.required") : t("content.dep.optional")} {dep.name}
                  </span>
                ))}
              </div>
            </>
          )}
        </div>
        {packError ? (
          <div className="field-error" data-detail-pack-error>
            {packError}
          </div>
        ) : null}
      </div>
    </Modal>
  );
}
