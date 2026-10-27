import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { InstanceIcon } from "../components/InstanceIcon";
import { Modal } from "../components/Modal";
import { formatBytes, formatStamp } from "../lib/format";
import { useT } from "../lib/i18n";
import { useDownloads } from "../lib/downloads-store";
import { api } from "../lib/tauri-api";
import type { ContentFileKind, Instance, InstanceSettings, JavaRuntime, LogFile, ModFileItem, SaveItem } from "../lib/types";

type Tab = "settings" | "logs" | "mods" | "resourcepacks" | "shader" | "saves";

interface Props {
  instance: Instance;
  onBack: () => void;
  onLaunch: (instance: Instance) => void;
  /** 保存设置后让上层重新拉一次 instances_list()（改名要同步卡片） */
  onSaved: () => void | Promise<void>;
  notify: (message: string) => void;
  /** 跳到 设置 → Java（缺对应大版本时用） */
  onOpenJavaSettings?: () => void;
}

const MIN_MEMORY = 1024;
const MAX_MEMORY = 8192;

/** 实例详情：设置 / 日志 / 模组 / 资源包 / 光影 / 存档（文案与顺序照参照图）。 */
/**
 * 按游戏版本推一个最低可行的 Java 大版本：
 * 1.21+ 与 1.20.5+ → 21；1.17–1.20.4 → 17；1.16 及更早 → 8。
 */
function requiredJavaFor(mcVersion: string): number {
  const match = /^(\d+)\.(\d+)(?:\.(\d+))?/.exec(mcVersion);
  if (!match) return 0;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  const patch = Number(match[3] ?? "0");
  if (major > 1) return 21;
  if (minor >= 21) return 21;
  if (minor === 20) return patch >= 5 ? 21 : 17;
  if (minor >= 17) return 17;
  return 8;
}

export function InstanceDetailPage({ instance, onBack, onLaunch, onSaved, notify, onOpenJavaSettings }: Props) {
  const t = useT();
  const [tab, setTab] = useState<Tab>("settings");

  const [settings, setSettings] = useState<InstanceSettings | null>(null);
  const [java, setJava] = useState<JavaRuntime[]>([]);
  const [saving, setSaving] = useState(false);

  const [logs, setLogs] = useState<LogFile[] | null>(null);
  const [logName, setLogName] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [logLoading, setLogLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [autoScroll, setAutoScroll] = useState(true);
  const viewRef = useRef<HTMLPreElement | null>(null);

  const requiredJavaMajor = requiredJavaFor(instance.mcVersion);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportTarget, setExportTarget] = useState("");
  const [exporting, setExporting] = useState(false);

  const { refresh: refreshDownloads } = useDownloads();

  useEffect(() => {
    let alive = true;
    void (async () => {
      const [loadedSettings, javaList] = await Promise.all([
        api.instanceSettingsGet(instance.id).catch(() => null),
        api.javaList().catch(() => [] as JavaRuntime[]),
      ]);
      if (!alive) return;
      if (loadedSettings) setSettings(loadedSettings);
      setJava(javaList);
    })();
    return () => {
      alive = false;
    };
  }, [instance.id]);

  // 日志文件列表：第一次进「日志」标签时才拉
  useEffect(() => {
    if (tab !== "logs" || logs !== null) return;
    let alive = true;
    void (async () => {
      try {
        const list = await api.logsList(instance.id);
        if (!alive) return;
        setLogs(list);
        setLogName(list[0]?.name ?? null);
      } catch (error) {
        if (alive) notify(String(error));
      }
    })();
    return () => {
      alive = false;
    };
  }, [tab, logs, instance.id, notify]);

  useEffect(() => {
    if (tab !== "logs" || !logName) return;
    let alive = true;
    setLogLoading(true);
    void (async () => {
      try {
        const text = await api.logRead(instance.id, logName);
        if (!alive) return;
        setLogText(text);
        setLogLoading(false);
      } catch (error) {
        if (!alive) return;
        setLogLoading(false);
        notify(String(error));
      }
    })();
    return () => {
      alive = false;
    };
  }, [tab, logName, instance.id, notify]);

  const lines = useMemo(() => {
    const all = logText.length > 0 ? logText.split("\n") : [];
    const needle = query.trim().toLowerCase();
    return needle.length === 0 ? all : all.filter((line) => line.toLowerCase().includes(needle));
  }, [logText, query]);

  // 自动滚动到底：内容变化或切回自动滚动时贴底
  useEffect(() => {
    if (!autoScroll) return;
    const node = viewRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [lines.length, autoScroll, logLoading]);

  const patch = (next: Partial<InstanceSettings>) => {
    setSettings((prev) => (prev ? { ...prev, ...next } : prev));
  };

  const save = async () => {
    if (!settings) return;
    setSaving(true);
    try {
      setSettings(await api.instanceSettingsSet(instance.id, settings));
      await onSaved();
      notify(t("detail.saved"));
    } catch (error) {
      notify(t("detail.saveFailed") + "：" + String(error));
    } finally {
      setSaving(false);
    }
  };

  const pickIcon = async () => {
    try {
      const path = await api.instancePickIcon(instance.id);
      if (path) patch({ iconPath: path });
    } catch (error) {
      notify(String(error));
    }
  };

  const startExport = async () => {
    setExporting(true);
    try {
      await api.instanceExport(instance.id, exportTarget.trim());
      refreshDownloads();
      setExportOpen(false);
      notify(t("detail.exportStarted", { name: (settings?.name ?? instance.name) + ".zip" }));
    } catch (error) {
      notify(String(error));
    } finally {
      setExporting(false);
    }
  };

  const tabs: Array<{ key: Tab; labelKey: string }> = [
    { key: "mods", labelKey: "detail.tab.mods" },
    { key: "shader", labelKey: "detail.tab.shader" },
    { key: "resourcepacks", labelKey: "detail.tab.resourcepacks" },
    { key: "saves", labelKey: "detail.tab.saves" },
    { key: "logs", labelKey: "detail.tab.logs" },
    { key: "settings", labelKey: "detail.tab.settings" },
  ];

  return (
    <div className="detail">
      <section className="card detail-head">
        <button className="icon-button detail-head__back" title={t("detail.back")} aria-label={t("detail.back")} onClick={onBack}>
          {/* TODO(M2-B): 图标素材里没有左箭头，先用内联 SVG 顶上 */}
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M15 5l-7 7 7 7" />
          </svg>
        </button>
        <InstanceIcon className="detail-head__icon" icon={settings?.iconPath || instance.icon} alt={instance.name} />
        <div className="detail-head__meta">
          <div className="detail-head__name">{settings?.name ?? instance.name}</div>
          <div className="instance-card__version">
            <span className="version-badge">{instance.mcVersion}</span>
            <span className="loader-badge">{instance.loader || "Vanilla"}</span>
          </div>
        </div>
        <button className="launch-button detail-head__launch" onClick={() => onLaunch(instance)}>
          <span className="play-triangle" aria-hidden="true" />
          {t("home.launch")}
        </button>
        <button
          type="button"
          className="pill-outline detail-head__export"
          data-action="instance-export"
          title={t("detail.export")}
          onClick={() => setExportOpen(true)}
        >
          <img src={icons.shareExport} alt="" />
          {t("detail.export")}
        </button>
        <button className="icon-button" title={t("instances.folder")} onClick={() => void api.instanceOpenFolder(instance.id).catch((error) => notify(String(error)))}>
          <img src={icons.folder} alt={t("instances.folder")} />
        </button>
      </section>

      <nav className="detail-tabs" role="tablist">
        {tabs.map((item) => (
          <button
            key={item.key}
            role="tab"
            aria-selected={tab === item.key}
            className={"detail-tab" + (tab === item.key ? " is-active" : "")}
            data-tab={item.key}
            onClick={() => setTab(item.key)}
          >
            {t(item.labelKey)}
          </button>
        ))}
      </nav>

      {tab === "settings" ? (
        settings ? (
          <div className="detail-grid">
            <section className="card">
              <div className="detail-card__title">{t("detail.sectionBasic")}</div>
              <div className="field">
                <label htmlFor="detail-name">{t("detail.name")}</label>
                <input
                  id="detail-name"
                  type="text"
                  value={settings.name}
                  onChange={(event) => patch({ name: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="detail-note">{t("detail.note")}</label>
                <textarea
                  id="detail-note"
                  rows={3}
                  value={settings.note}
                  placeholder={t("detail.notePlaceholder")}
                  onChange={(event) => patch({ note: event.target.value })}
                />
              </div>
            </section>

            <section className="card">
              <div className="detail-card__title">{t("detail.sectionMemory")}</div>
              <div className="field">
                <label htmlFor="detail-memory">
                  {t("detail.memory")}
                  <span className="field__value">{t("instances.memoryValue", { value: settings.memoryMb })}</span>
                </label>
                <input
                  id="detail-memory"
                  type="range"
                  min={MIN_MEMORY}
                  max={MAX_MEMORY}
                  step={256}
                  value={settings.memoryMb}
                  onChange={(event) => patch({ memoryMb: Number(event.target.value) })}
                />
              </div>
              <div className="detail-card__title">{t("detail.window")}</div>
              <div className="field field--inline">
                <input
                  type="number"
                  aria-label={t("detail.windowWidth")}
                  value={settings.windowWidth}
                  onChange={(event) => patch({ windowWidth: Number(event.target.value) || 0 })}
                />
                <span className="mono">×</span>
                <input
                  type="number"
                  aria-label={t("detail.windowHeight")}
                  value={settings.windowHeight}
                  onChange={(event) => patch({ windowHeight: Number(event.target.value) || 0 })}
                />
              </div>
            </section>

            <section className="card">
              <div className="detail-card__title">{t("detail.java")}</div>
              <div className="field">
                {requiredJavaMajor && !java.some((runtime) => runtime.major === requiredJavaMajor) ? (
                  <div className="notice" data-java-missing>
                    <strong>{t("settings.java.missing", { major: requiredJavaMajor })}</strong>
                    {onOpenJavaSettings ? (
                      <button type="button" className="link-button" data-action="java-go-settings" onClick={onOpenJavaSettings}>
                        {t("settings.java.goJava")}
                      </button>
                    ) : null}
                  </div>
                ) : null}
                <select
                  value={settings.javaPath}
                  data-java-select
                  onChange={(event) => patch({ javaPath: event.target.value })}
                >
                  <option value="">{t("detail.javaAuto")}</option>
                  {java.map((runtime) => (
                    <option key={runtime.javaExe} value={runtime.javaExe}>
                      {"Java " + runtime.major + " (" + runtime.version + ") · " + runtime.vendor}
                    </option>
                  ))}
                </select>
                <div className="row__hint">{t("detail.javaHint")}</div>
              </div>
            </section>

            <section className="card">
              <div className="detail-card__title">{t("detail.icon")}</div>
              <div className="row">
                <InstanceIcon className="detail-icon__preview" icon={settings.iconPath} alt={t("detail.icon")} />
                <div>
                  <button onClick={() => void pickIcon()}>{t("detail.iconPick")}</button>
                  <div className="row__hint">{t("detail.iconHint")}</div>
                </div>
              </div>
            </section>

            <div className="detail-actions">
              <button className="pill-button" disabled={saving} onClick={() => void save()}>
                {t("common.save")}
              </button>
            </div>
          </div>
        ) : (
          <div className="empty-hint">{t("common.loading")}</div>
        )
      ) : null}

      {tab === "mods" ? (
        <FilesPane
          instanceId={instance.id}
          kind="mod"
          notify={notify}
          importLabelKey="detail.mods.import"
          openLabelKey="detail.mods.openFolder"
          emptyKey="detail.mods.empty"
        />
      ) : null}

      {tab === "resourcepacks" ? (
        <FilesPane
          instanceId={instance.id}
          kind="resourcepack"
          notify={notify}
          importLabelKey="detail.packs.import"
          openLabelKey="detail.packs.openFolder"
          emptyKey="detail.content.empty"
        />
      ) : null}

      {tab === "shader" ? (
        <FilesPane
          instanceId={instance.id}
          kind="shader"
          notify={notify}
          importLabelKey="detail.packs.import"
          openLabelKey="detail.packs.openFolder"
          emptyKey="detail.content.empty"
        />
      ) : null}

      {tab === "saves" ? <SavesPane instanceId={instance.id} notify={notify} /> : null}

      {tab === "logs" ? (
        <div className="logs">
          <aside className="card logs__files">
            <div className="logs__files-head">
              <span>{t("detail.logs.files")}</span>
              <button className="icon-button" title={t("detail.logs.openFolder")} onClick={() => void api.logOpenFolder(instance.id).catch((error) => notify(String(error)))}>
                <img src={icons.folder} alt={t("detail.logs.openFolder")} />
              </button>
            </div>
            {logs === null ? (
              <div className="row__hint">{t("common.loading")}</div>
            ) : logs.length === 0 ? (
              <div className="row__hint">{t("detail.logs.empty")}</div>
            ) : (
              <div className="logs__file-list">
                {logs.map((file) => (
                  <button
                    key={file.name}
                    className={"logs__file" + (logName === file.name ? " is-active" : "")}
                    aria-current={logName === file.name ? "true" : undefined}
                    onClick={() => setLogName(file.name)}
                  >
                    <span className="logs__file-name">{file.name}</span>
                    <span className="logs__file-meta">
                      {formatBytes(file.size)} · {formatStamp(file.modifiedAt)}
                    </span>
                  </button>
                ))}
              </div>
            )}
          </aside>

          <section className="card logs__viewer">
            <div className="logs__toolbar">
              <input
                type="text"
                className="logs__search"
                value={query}
                placeholder={t("detail.logs.search")}
                onChange={(event) => setQuery(event.target.value)}
              />
              {query.trim().length > 0 ? <span className="chip">{t("detail.logs.matches", { count: lines.length })}</span> : null}
              <label className="logs__toggle">
                <input type="checkbox" checked={autoScroll} onChange={(event) => setAutoScroll(event.target.checked)} />
                {t("detail.logs.autoscroll")}
              </label>
            </div>
            {!logName ? (
              <div className="empty-hint">{t("detail.logs.placeholder")}</div>
            ) : logLoading ? (
              <div className="empty-hint">{t("common.loading")}</div>
            ) : lines.length === 0 ? (
              <div className="empty-hint">{t("detail.logs.noMatch")}</div>
            ) : (
              <pre className="log-view" ref={viewRef} tabIndex={0}>
                {lines.map((line, index) => (
                  <div className="log-line" key={index}>
                    {line}
                  </div>
                ))}
              </pre>
            )}
          </section>
        </div>
      ) : null}

      {exportOpen ? (
        <Modal
          title={t("detail.exportTitle")}
          onClose={() => setExportOpen(false)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setExportOpen(false)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="instance-export-start" disabled={exporting} onClick={() => void startExport()}>
                {t("detail.exportStart")}
              </button>
            </>
          }
        >
          <p className="row__hint">{t("detail.exportHint")}</p>
          <div className="field">
            <label htmlFor="export-target">{t("detail.exportTarget")}</label>
            <div className="field-pair">
              <input
                id="export-target"
                type="text"
                data-export-target
                value={exportTarget}
                placeholder="D:\minecraft\DL"
                onChange={(event) => setExportTarget(event.target.value)}
              />
              <button
                type="button"
                className="pill-outline"
                onClick={() =>
                  void api
                    .pickDirectory()
                    .then((dir) => {
                      if (dir) setExportTarget(dir);
                    })
                    .catch((error) => notify(String(error)))
                }
              >
                {t("detail.exportPick")}
              </button>
            </div>
          </div>
        </Modal>
      ) : null}
    </div>
  );
}

interface FilesPaneProps {
  instanceId: number;
  kind: ContentFileKind;
  notify: (message: string) => void;
  importLabelKey: string;
  openLabelKey: string;
  emptyKey: string;
}

/**
 * 模组 / 资源包 / 光影共用的列表：开关启用状态、批量启停、删除（二次确认）、按文件名搜索。
 * 启用/禁用只改后缀（.jar ⇄ .jar.disabled，资源包是 .zip），由后端落盘。
 */
function FilesPane({ instanceId, kind, notify, importLabelKey, openLabelKey, emptyKey }: FilesPaneProps) {
  const t = useT();
  const [files, setFiles] = useState<ModFileItem[] | null>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [pendingDelete, setPendingDelete] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setFiles(await api.modsList(instanceId, kind));
    } catch (error) {
      notify(String(error));
    }
  }, [instanceId, kind, notify]);

  useEffect(() => {
    setFiles(null);
    setSelected(new Set());
    void load();
  }, [load]);

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const list = files ?? [];
    return needle.length === 0 ? list : list.filter((file) => file.file.toLowerCase().includes(needle));
  }, [files, query]);

  const toggle = async (file: ModFileItem) => {
    setBusy(true);
    try {
      const next = await api.modsSetEnabled(instanceId, kind, file.file, !file.enabled);
      setFiles(next);
      notify(t("detail.content.toggled", { state: t(file.enabled ? "detail.content.disabled" : "detail.content.enabled"), count: 1 }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const applyMany = async (enabled: boolean) => {
    const targets = (files ?? []).filter((file) => selected.has(file.file) && file.enabled !== enabled);
    if (targets.length === 0) return;
    setBusy(true);
    try {
      let last: ModFileItem[] | null = null;
      for (const target of targets) {
        last = await api.modsSetEnabled(instanceId, kind, target.file, enabled);
      }
      if (last) setFiles(last);
      setSelected(new Set());
      notify(t("detail.content.toggled", { state: t(enabled ? "detail.content.enabled" : "detail.content.disabled"), count: targets.length }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const doDelete = async () => {
    if (!pendingDelete) return;
    const targets = pendingDelete;
    setPendingDelete(null);
    setBusy(true);
    try {
      let last: ModFileItem[] | null = null;
      for (const file of targets) {
        last = await api.modsDelete(instanceId, kind, file);
      }
      if (last) setFiles(last);
      setSelected(new Set());
      notify(t("detail.content.deleted", { count: targets.length }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const importFiles = async () => {
    setBusy(true);
    try {
      // 传空路径＝让后端自己弹文件框挑文件，前端不需要额外的原生插件
      const next = await api.modsImport(instanceId, kind, []);
      setFiles(next);
      notify(t("detail.content.imported", { count: next.length - (files?.length ?? 0) }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const allChecked = visible.length > 0 && visible.every((file) => selected.has(file.file));

  return (
    <section className="card files-pane">
      <div className="files-pane__toolbar">
        <input
          type="text"
          className="files-pane__search"
          value={query}
          data-files-search
          placeholder={t("detail.content.search")}
          onChange={(event) => setQuery(event.target.value)}
        />
        <div className="files-pane__actions">
          <button type="button" className="pill-outline" onClick={() => void api.modsOpenFolder(instanceId, kind).catch((error) => notify(String(error)))}>
            <img src={icons.folder} alt="" />
            {t(openLabelKey)}
          </button>
          <button type="button" className="pill-outline" data-action="files-import" disabled={busy} onClick={() => void importFiles()}>
            <img src={icons.download} alt="" />
            {t(importLabelKey)}
          </button>
          <button type="button" className="pill-outline" data-action="files-refresh" disabled={busy} onClick={() => void load()}>
            <img src={icons.refresh} alt="" />
            {t("detail.content.refresh")}
          </button>
        </div>
      </div>

      {selected.size > 0 ? (
        <div className="files-pane__batch" data-batch-bar>
          <span className="chip">{t("detail.content.selected", { count: selected.size })}</span>
          <button type="button" className="pill-outline" data-action="batch-enable" disabled={busy} onClick={() => void applyMany(true)}>
            {t("detail.content.batchEnable")}
          </button>
          <button type="button" className="pill-outline" data-action="batch-disable" disabled={busy} onClick={() => void applyMany(false)}>
            {t("detail.content.batchDisable")}
          </button>
          <button
            type="button"
            className="pill-outline pill-outline--danger"
            data-action="batch-delete"
            disabled={busy}
            onClick={() => setPendingDelete(Array.from(selected))}
          >
            {t("detail.content.deleteSelected")}
          </button>
        </div>
      ) : null}

      {files === null ? (
        <div className="row__hint">{t("common.loading")}</div>
      ) : visible.length === 0 ? (
        <div className="empty-hint">{query.trim().length > 0 ? t("detail.content.noMatch") : t(emptyKey)}</div>
      ) : (
        <div className="file-list">
          <div className="file-row file-row--head">
            <label className="file-row__check">
              <input
                type="checkbox"
                checked={allChecked}
                aria-label={t("detail.content.selectAll")}
                onChange={(event) => {
                  const next = new Set(selected);
                  for (const file of visible) {
                    if (event.target.checked) next.add(file.file);
                    else next.delete(file.file);
                  }
                  setSelected(next);
                }}
              />
            </label>
            <span className="file-row__name">{t("detail.content.selectAll")}</span>
            <span className="file-row__size">{t("detail.saves.size")}</span>
            <span className="file-row__ops" />
          </div>
          {visible.map((file) => (
            <div
              className={"file-row" + (file.enabled ? "" : " is-disabled")}
              key={file.file}
              data-file={file.file}
              data-enabled={file.enabled ? "1" : "0"}
            >
              <label className="file-row__check">
                <input
                  type="checkbox"
                  checked={selected.has(file.file)}
                  aria-label={file.name}
                  onChange={(event) => {
                    const next = new Set(selected);
                    if (event.target.checked) next.add(file.file);
                    else next.delete(file.file);
                    setSelected(next);
                  }}
                />
              </label>
              <div className="file-row__main">
                <div className="file-row__title">
                  <span className="file-row__name">{file.name}</span>
                  {file.version ? (
                    <span className="chip chip--tiny">
                      {t("detail.mod.version")} {file.version}
                    </span>
                  ) : null}
                  {file.loader ? <span className="chip chip--tiny">{file.loader}</span> : null}
                  {file.source ? <span className="source-badge source-badge--small">{file.source}</span> : null}
                </div>
                <div className="file-row__file">{file.file}</div>
              </div>
              <label className="switch file-row__switch" title={file.enabled ? t("detail.content.enabled") : t("detail.content.disabled")}>
                <input
                  type="checkbox"
                  checked={file.enabled}
                  aria-label={file.name}
                  data-toggle={file.file}
                  disabled={busy}
                  onChange={() => void toggle(file)}
                />
                <span className="switch__track" aria-hidden="true">
                  <span className="switch__dot" />
                </span>
              </label>
              <span className="file-row__size">{formatBytes(file.size)}</span>
              <span className="file-row__ops">
                <button
                  type="button"
                  className="icon-button icon-button--danger"
                  data-action="file-delete"
                  title={t("common.delete")}
                  onClick={() => setPendingDelete([file.file])}
                >
                  <img src={icons.trash} alt={t("common.delete")} />
                </button>
              </span>
            </div>
          ))}
        </div>
      )}

      {pendingDelete ? (
        <Modal
          title={t("detail.content.deleteTitle")}
          onClose={() => setPendingDelete(null)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setPendingDelete(null)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="file-delete-confirm" onClick={() => void doDelete()}>
                {t("common.delete")}
              </button>
            </>
          }
        >
          <p>
            {pendingDelete.length === 1
              ? t("detail.content.deleteConfirm", { name: pendingDelete[0] })
              : t("detail.content.selected", { count: pendingDelete.length })}
          </p>
        </Modal>
      ) : null}
    </section>
  );
}

interface SavesPaneProps {
  instanceId: number;
  notify: (message: string) => void;
}

/** 存档标签：存档名 / 最后修改 / 大小 / 游戏版本 + 打开目录、导出 zip、删除。 */
function SavesPane({ instanceId, notify }: SavesPaneProps) {
  const t = useT();
  const [saves, setSaves] = useState<SaveItem[] | null>(null);
  const [pendingDelete, setPendingDelete] = useState<SaveItem | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setSaves(await api.savesList(instanceId));
    } catch (error) {
      notify(String(error));
    }
  }, [instanceId, notify]);

  useEffect(() => {
    void load();
  }, [load]);

  const doDelete = async () => {
    if (!pendingDelete) return;
    const target = pendingDelete;
    setPendingDelete(null);
    setBusy(true);
    try {
      setSaves(await api.savesDelete(instanceId, target.name));
      notify(t("detail.saves.deleted", { name: target.name }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const doExport = async (save: SaveItem) => {
    setBusy(true);
    try {
      const result = await api.savesExport(instanceId, save.name);
      notify(t("detail.saves.exported", { path: result.path }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card files-pane">
      <div className="files-pane__toolbar">
        <span className="detail-card__title">{t("detail.tab.saves")}</span>
        <div className="files-pane__actions">
          <button
            type="button"
            className="pill-outline"
            onClick={() => void api.savesOpenFolder(instanceId).catch((error) => notify(String(error)))}
          >
            <img src={icons.folder} alt="" />
            {t("detail.saves.openFolder")}
          </button>
          <button type="button" className="pill-outline" disabled={busy} onClick={() => void load()}>
            <img src={icons.refresh} alt="" />
            {t("detail.content.refresh")}
          </button>
        </div>
      </div>

      {saves === null ? (
        <div className="row__hint">{t("common.loading")}</div>
      ) : saves.length === 0 ? (
        <div className="empty-hint">{t("detail.saves.empty")}</div>
      ) : (
        <div className="file-list">
          <div className="file-row file-row--head file-row--saves">
            <span className="file-row__name">{t("detail.tab.saves")}</span>
            <span className="file-row__time">{t("detail.saves.modified")}</span>
            <span className="file-row__size">{t("detail.saves.size")}</span>
            <span className="file-row__ops" />
          </div>
          {saves.map((save) => (
            <div className="file-row file-row--saves" key={save.name} data-save={save.name}>
              <div className="file-row__main">
                <div className="file-row__title">
                  <span className="file-row__name">{save.name}</span>
                  {save.mcVersion ? <span className="chip chip--tiny">{save.mcVersion}</span> : null}
                </div>
                <div className="file-row__file">{t("detail.saves.version")} · {save.mcVersion || "—"}</div>
              </div>
              <span className="file-row__time">{formatStamp(save.modifiedAt)}</span>
              <span className="file-row__size">{formatBytes(save.size)}</span>
              <span className="file-row__ops">
                <button type="button" className="pill-outline pill-outline--tiny" data-action="save-export" disabled={busy} onClick={() => void doExport(save)}>
                  {t("detail.saves.export")}
                </button>
                <button
                  type="button"
                  className="icon-button icon-button--danger"
                  data-action="save-delete"
                  title={t("common.delete")}
                  onClick={() => setPendingDelete(save)}
                >
                  <img src={icons.trash} alt={t("common.delete")} />
                </button>
              </span>
            </div>
          ))}
        </div>
      )}

      {pendingDelete ? (
        <Modal
          title={t("detail.saves.deleteTitle")}
          onClose={() => setPendingDelete(null)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setPendingDelete(null)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="save-delete-confirm" onClick={() => void doDelete()}>
                {t("common.delete")}
              </button>
            </>
          }
        >
          <p>{t("detail.saves.deleteConfirm", { name: pendingDelete.name })}</p>
        </Modal>
      ) : null}
    </section>
  );
}
