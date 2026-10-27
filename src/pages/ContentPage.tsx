import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { Modal } from "../components/Modal";
import { formatBytes } from "../lib/format";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type {
  ContentItem,
  ContentKindTab,
  ContentProvider,
  ContentSearchQuery,
  ContentSort,
  Instance,
  ProjectDetail,
  VersionItem,
  VersionManifest,
} from "../lib/types";

interface Props {
  instances: Instance[];
  versions: VersionManifest | null;
  notify: (message: string) => void;
  onInstalled: () => void;
}

const PROVIDERS: Array<{ id: ContentProvider; labelKey: string }> = [
  { id: "modrinth", labelKey: "settings.contentSourceModrinth" },
  { id: "curseforge", labelKey: "settings.contentSourceCurseforge" },
  { id: "mcim", labelKey: "settings.contentSourceMcim" },
];

const KIND_TABS: ContentKindTab[] = ["mod", "modpack", "resourcepack", "shader", "datapack"];
const SORTS: Array<{ id: ContentSort; labelKey: string }> = [
  { id: "downloads", labelKey: "content.sort.downloads" },
  { id: "updated", labelKey: "content.sort.updated" },
  { id: "relevance", labelKey: "content.sort.relevance" },
];
const PAGE_SIZES = [20, 40, 60];
const LOADERS = ["Fabric", "Forge", "NeoForge", "Quilt", "原版"];
const CATEGORIES = ["优化", "科技", "实用", "装饰", "冒险", "光影", "材质", "数据包", "轻量"];

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
  versionId?: string;
}

/** 资源页：类型 tab + 搜索/实例版本 + 工具行 + 筛选 chips + 网格/列表 + 分页器。 */
export function ContentPage({ instances, versions, notify, onInstalled }: Props) {
  const t = useT();

  const [provider, setProvider] = useState<ContentProvider>("modrinth");
  const [kind, setKind] = useState<ContentKindTab>("mod");
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const [sort, setSort] = useState<ContentSort>("downloads");
  const [pageSize, setPageSize] = useState(20);
  const [view, setView] = useState<"grid" | "list">("grid");
  const [page, setPage] = useState(1);
  const [loader, setLoader] = useState("");
  const [categories, setCategories] = useState<string[]>([]);
  const [filtersOpen, setFiltersOpen] = useState(false);
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

  useEffect(() => {
    let alive = true;
    void api
      .settingsGet()
      .then((loaded) => {
        if (!alive) return;

        if (loaded.resourceSource === "curseforge" || loaded.resourceSource === "mcim" || loaded.resourceSource === "modrinth") {
          setProvider(loaded.resourceSource);
        }
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
      mcVersion: undefined,
      loader: loader || undefined,
      categories: categories.length > 0 ? categories : undefined,
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
  }, [provider, kind, submitted, loader, categories, sort, page, pageSize, translateOn, nonce]);

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

  const resetFilters = () => {
    setLoader("");
    setCategories([]);
    setSubmitted("");
    setQuery("");
    setPage(1);
  };

  const chips = useMemo(() => {
    const out: Array<{ key: string; label: string; clear: () => void }> = [];
    if (loader) out.push({ key: "loader", label: t("content.chip.loader", { value: loader }), clear: () => setLoader("") });
    for (const category of categories) {
      out.push({
        key: "cat-" + category,
        label: t("content.chip.category", { value: category }),
        clear: () => setCategories((prev) => prev.filter((item) => item !== category)),
      });
    }
    if (submitted) out.push({ key: "q", label: t("content.chip.query", { value: submitted }), clear: () => { setSubmitted(""); setQuery(""); } });
    return out;
  }, [loader, categories, submitted, t]);

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
    const translated = translations[item.id];
    const title = translated?.title ?? (item.translated ? item.title : item.title) ?? item.titleOriginal ?? "";
    const summary = translated?.summary ?? item.summary ?? item.summaryOriginal ?? "";
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
            onClick={() => setFiltersOpen((open) => !open)}
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
            onClick={() => setTranslateOn((on) => !on)}
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
          <div className="content-filters-panel" data-content-filters>
            <div className="content-filter-block">
              <span className="content-filter__label">{t("content.loader")}</span>
              <div className="chip-row">
                {LOADERS.map((item) => (
                  <button
                    key={item}
                    type="button"
                    className={"chip chip--button" + (loader === item ? " is-active" : "")}
                    data-filter-loader={item}
                    onClick={() => {
                      setLoader(loader === item ? "" : item);
                      setPage(1);
                    }}
                  >
                    {item}
                  </button>
                ))}
              </div>
            </div>
            <div className="content-filter-block">
              <span className="content-filter__label">{t("content.categories")}</span>
              <div className="chip-row">
                {CATEGORIES.map((item) => (
                  <button
                    key={item}
                    type="button"
                    className={"chip chip--button" + (categories.includes(item) ? " is-active" : "")}
                    data-filter-category={item}
                    onClick={() => {
                      setCategories((prev) => (prev.includes(item) ? prev.filter((value) => value !== item) : [...prev, item]));
                      setPage(1);
                    }}
                  >
                    {item}
                  </button>
                ))}
              </div>
            </div>
          </div>
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
            {/不支持|unsupported/i.test(errorMessage) ? t("content.unsupportedKind") : t("content.loadFailed")}
          </strong>
          {errorMessage ? <span className="row__hint" data-content-error-message>{errorMessage}</span> : null}
          <div className="content-error__actions">
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
                    onClick={() => setInstall({ title: data.title, projectId: item.id })}
                  >
                    {t("content.installShort")}
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
        <ContentDetailDrawer item={detailItem} onClose={() => setDetailItem(null)} onInstall={(versionId) => setInstall({ title: cardFor(detailItem).title, projectId: detailItem.id, versionId })} notify={notify} />
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
          {instances.length === 0 ? (
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

interface DrawerProps {
  item: ContentItem;
  onClose: () => void;
  onInstall: (versionId?: string) => void;
  notify: (message: string) => void;
}

/** 详情抽屉：项目详情 + 版本列表（保留原有能力，id 换成新的 ContentItem.id）。 */
function ContentDetailDrawer({ item, onClose, onInstall, notify }: DrawerProps) {
  const t = useT();
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [versions, setVersions] = useState<VersionItem[] | null>(null);
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setDetail(null);
    setVersions(null);
    void (async () => {
      try {
        const [project, versionList] = await Promise.all([
          api.contentDetail(item.provider, item.id),
          api.contentVersions(item.provider, item.id),
        ]);
        if (!alive) return;
        setDetail(project);
        setVersions(versionList);
        setSelectedVersion(versionList[0]?.versionId ?? null);
      } catch (error) {
        if (alive) notify(String(error));
      }
    })();
    return () => {
      alive = false;
    };
  }, [item.provider, item.id, notify]);

  return (
    <div className="drawer-overlay" role="dialog" aria-modal="true" aria-label={t("content.detail")} onClick={onClose}>
      <aside className="drawer" data-content-drawer onClick={(event) => event.stopPropagation()}>
        <header className="drawer__head">
          <div className="drawer__title">
            {item.iconUrl ? <img className="drawer__icon" src={item.iconUrl} alt="" /> : null}
            <div>
              <h3>{detail?.title ?? item.title}</h3>
              <span className="row__hint">
                {item.author || "—"} · {t("content.source." + item.provider)}
                {detail?.license ? " · " + detail.license : ""}
              </span>
            </div>
          </div>
          <button type="button" className="icon-button" data-action="drawer-close" title={t("content.close")} onClick={onClose}>
            <img src={icons.close} alt={t("content.close")} />
          </button>
        </header>
        <div className="drawer__body">
          <section className="drawer__section">
            <div className="drawer__section-head">
              <span className="detail-card__title">{t("content.description")}</span>
            </div>
            <pre className="drawer__text" data-drawer-description>
              {detail === null ? t("common.loading") : detail.description}
            </pre>
          </section>
          <section className="drawer__section">
            <div className="drawer__section-head">
              <span className="detail-card__title">{t("content.versions")}</span>
              {versions !== null ? <span className="row__hint">{versions.length}</span> : null}
            </div>
            {versions === null ? (
              <div className="row__hint">{t("common.loading")}</div>
            ) : versions.length === 0 ? (
              <div className="empty-hint">{t("content.noVersions")}</div>
            ) : (
              <div className="version-rows">
                {versions.map((version) => (
                  <button
                    key={version.versionId}
                    type="button"
                    className={"version-row" + (selectedVersion === version.versionId ? " is-active" : "")}
                    data-version={version.versionId}
                    onClick={() => setSelectedVersion(version.versionId)}
                  >
                    <span className="version-row__name">{version.name}</span>
                    <span className="version-row__meta">
                      {version.gameVersions.join(" / ")} · {version.loaders.join(" / ")} · {formatBytes(version.fileSize)}
                    </span>
                  </button>
                ))}
              </div>
            )}
          </section>
        </div>
        <footer className="drawer__foot">
          <button type="button" className="pill-outline" onClick={onClose}>
            {t("common.close")}
          </button>
          <button type="button" className="pill-button" data-action="drawer-install" onClick={() => onInstall(selectedVersion ?? undefined)}>
            {t("content.installVersion")}
          </button>
        </footer>
      </aside>
    </div>
  );
}
