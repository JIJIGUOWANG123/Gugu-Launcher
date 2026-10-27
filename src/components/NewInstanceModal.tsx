import { useEffect, useMemo, useState } from "react";
import { Modal } from "./Modal";
import { useT } from "../lib/i18n";
import { classifyVersions, type VersionKind } from "../lib/versions";
import { api } from "../lib/tauri-api";
import type { LoaderVersion, VersionManifest } from "../lib/types";

/** 新建实例悬浮窗提交出去的载荷（loader 是 Vanilla / Fabric / Forge / NeoForge）。 */
export interface NewInstancePayload {
  name: string;
  mcVersion: string;
  loader: string;
  /** 选中加载器的具体构建；原版留空 */
  loaderVersion?: string;
  memoryMb: number;
}

interface Props {
  versions: VersionManifest | null;
  onClose: () => void;
  onSubmit: (payload: NewInstancePayload) => Promise<void>;
}

/** 加载器：原版不需要后端数据，其余三种并行查 loader_versions。labelKey 为空表示直接显示 id。 */
const LOADERS: Array<{ id: string; labelKey: string | null }> = [
  { id: "Vanilla", labelKey: "instances.loaderVanilla" },
  { id: "Fabric", labelKey: null },
  { id: "Forge", labelKey: null },
  { id: "NeoForge", labelKey: null },
];

const LOADER_API_ID: Record<string, "fabric" | "forge" | "neoforge"> = {
  Fabric: "fabric",
  Forge: "forge",
  NeoForge: "neoforge",
};

/** 每个加载器的查询结果：加载中 / 可用构建 / 后端原因。 */
interface LoaderSlot {
  loading: boolean;
  versions: LoaderVersion[];
  error: string;
}

const KINDS = ["release", "snapshot", "april", "old"] as const;

const MIN_MEMORY = 1024;
const MAX_MEMORY = 8192;
const DEFAULT_MEMORY = 2048;

/** 新建实例：选正式版（可搜索）→ 选加载器（先只支持原版）→ 实例名 + 内存。 */
export function NewInstanceModal({ versions, onClose, onSubmit }: Props) {
  const t = useT();

  // 字段名兼容与四类划分都交给 lib/versions
  const classified = useMemo(() => classifyVersions(versions?.versions ?? []), [versions]);
  const loading = versions === null;
  const [kinds, setKinds] = useState<VersionKind[]>(["release", "snapshot", "april", "old"]);
  const visible = classified.all.filter((entry) => kinds.includes(entry.kind));

  const [query, setQuery] = useState("");
  const [mcVersion, setMcVersion] = useState<string>(
    () => versions?.latestRelease ?? classifyVersions(versions?.versions ?? []).release[0]?.id ?? "1.21.4",
  );
  const [loader, setLoader] = useState("Vanilla");
  const [loaderVersion, setLoaderVersion] = useState("");
  const [loaderSlots, setLoaderSlots] = useState<Record<string, LoaderSlot>>({});
  const [name, setName] = useState("");
  const [memoryMb, setMemoryMb] = useState(DEFAULT_MEMORY);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 选定 MC 版本后并行查三个加载器的构建（互不牵连：一个失败不影响其它）
  useEffect(() => {
    let alive = true;
    const pending: Record<string, LoaderSlot> = {};
    for (const id of ["fabric", "forge", "neoforge"]) pending[id] = { loading: true, versions: [], error: "" };
    setLoaderSlots(pending);
    void Promise.all(
      (["fabric", "forge", "neoforge"] as const).map(async (id) => {
        try {
          const versions = await api.loaderVersions({ loader: id, mcVersion });
          return { id, versions, error: "" };
        } catch (error) {
          return { id, versions: [] as LoaderVersion[], error: String(error instanceof Error ? error.message : error) };
        }
      }),
    ).then((rows) => {
      if (!alive) return;
      const next: Record<string, LoaderSlot> = {};
      for (const row of rows) next[row.id] = { loading: false, versions: row.versions, error: row.error };
      setLoaderSlots(next);
    });
    return () => {
      alive = false;
    };
  }, [mcVersion]);

  const activeApiId = LOADER_API_ID[loader];
  const activeSlot = activeApiId ? loaderSlots[activeApiId] : undefined;

  // 选中加载器后自动挑一个具体版本：后端推荐的优先，否则最新（列表第一条）
  useEffect(() => {
    if (!activeSlot || activeSlot.loading) return;
    if (activeSlot.versions.length === 0) {
      setLoaderVersion("");
      return;
    }
    const preferred = activeSlot.versions.find((item) => item.recommended) ?? activeSlot.versions[0];
    setLoaderVersion(preferred.version);
  }, [activeApiId, activeSlot]);

  const filtered = visible.filter((entry) => entry.version.id.toLowerCase().includes(query.trim().toLowerCase()));
  const shownName = name.trim().length > 0 ? name : mcVersion;

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await onSubmit({
        name: shownName,
        mcVersion,
        loader,
        loaderVersion: activeApiId ? loaderVersion : "",
        memoryMb,
      });
      onClose();
    } catch (caught) {
      setError(String(caught));
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("instances.newTitle")}
      onClose={onClose}
      footer={
        <>
          <button onClick={onClose}>{t("common.cancel")}</button>
          <button className="pill-button" disabled={busy} onClick={() => void submit()}>
            {t("instances.createStart")}
          </button>
        </>
      }
    >
      <div className="field">
        <label htmlFor="new-instance-version-search">{t("instances.newVersion")}</label>
        <input
          id="new-instance-version-search"
          type="text"
          value={query}
          placeholder={t("instances.versionSearch")}
          onChange={(event) => setQuery(event.target.value)}
        />
        <div className="version-kinds" data-version-kinds>
          {KINDS.map((kind) => {
            const active = kinds.includes(kind);
            return (
              <button
                key={kind}
                type="button"
                aria-pressed={active}
                className={"chip chip--button" + (active ? " is-active" : "")}
                data-version-kind={kind}
                onClick={() =>
                  setKinds((prev) => (prev.includes(kind) ? prev.filter((item) => item !== kind) : [...prev, kind]))
                }
              >
                {t("instances.kind." + kind)} {classified.counts[kind]}
              </button>
            );
          })}
          {kinds.length === 0 ? (
            <button type="button" className="link-button" data-action="version-select-all" onClick={() => setKinds(["release", "snapshot", "april", "old"])}>
              {t("instances.selectAllKinds")}
            </button>
          ) : null}
        </div>
        <div className="version-list" role="listbox" aria-label={t("instances.newVersion")} data-version-list>
          {loading ? (
            <div className="version-list__loading" data-version-loading>
              <span className="skel skel--line" />
              <span className="skel skel--line skel--short" />
              <span className="skel skel--line" />
            </div>
          ) : kinds.length === 0 ? (
            <div className="version-list__empty" data-version-no-kind>
              {t("instances.noKindSelected")}
              <button type="button" className="link-button" data-action="version-select-all" onClick={() => setKinds(["release", "snapshot", "april", "old"])}>
                {t("instances.selectAllKinds")}
              </button>
            </div>
          ) : filtered.length === 0 ? (
            <div className="version-list__empty" data-version-empty>
              {classified.all.length === 0 ? t("instances.versionLoadFailed") : t("instances.versionEmpty")}
            </div>
          ) : (
            filtered.map((entry) => {
              const version = entry.version;
              return (
                <button
                  key={version.id}
                  role="option"
                  aria-selected={version.id === mcVersion}
                  className={"version-option" + (version.id === mcVersion ? " is-active" : "")}
                  data-version-item={version.id}
                  data-version-item-kind={entry.kind}
                  onClick={() => setMcVersion(version.id)}
                >
                  <span>{version.id}</span>
                  {entry.kind === "april" ? <span aria-hidden="true">🎉</span> : null}
                  {version.id === versions?.latestRelease ? (
                    <span className="version-option__tag">{t("home.latestRelease")}</span>
                  ) : null}
                  <span className={"version-kind version-kind--" + entry.kind} data-version-kind-badge>
                    {t("instances.kind." + entry.kind)}
                  </span>
                </button>
              );
            })
          )}
        </div>
        <div className="row__hint">{t("instances.releaseOnly")}</div>
      </div>

      <div className="field">
        <label>{t("instances.loader")}</label>
        <div className="loader-picker">
          {LOADERS.map((item) => {
            const apiId = LOADER_API_ID[item.id];
            const slot = apiId ? loaderSlots[apiId] : undefined;
            const loading = Boolean(slot?.loading);
            const unavailable = Boolean(slot && !slot.loading && slot.versions.length === 0);
            return (
              <button
                key={item.id}
                type="button"
                className={"loader-option" + (loader === item.id ? " is-active" : "")}
                data-loader={item.id}
                data-loader-available={unavailable ? "0" : "1"}
                aria-disabled={unavailable}
                onClick={() => setLoader(item.id)}
              >
                <span>{item.labelKey ? t(item.labelKey) : item.id}</span>
                {loading ? <span className="loader-option__tag">{t("common.loading")}</span> : null}
                {unavailable ? (
                  <span className="loader-option__tag" data-loader-unavailable>
                    {t("instances.loaderUnavailable")}
                  </span>
                ) : null}
              </button>
            );
          })}
        </div>

        {activeSlot && !activeSlot.loading && activeSlot.versions.length > 0 ? (
          <div className="field-pair loader-version">
            <label htmlFor="new-instance-loader-version">{t("instances.loaderVersion")}</label>
            <select
              id="new-instance-loader-version"
              data-loader-version
              value={loaderVersion}
              onChange={(event) => setLoaderVersion(event.target.value)}
            >
              {activeSlot.versions.map((item) => (
                <option key={item.id} value={item.version}>
                  {item.version}
                  {item.recommended ? t("instances.loaderRecommended") : ""}
                </option>
              ))}
            </select>
          </div>
        ) : null}

        {activeSlot && !activeSlot.loading && activeSlot.error ? (
          <div className="field-error" data-loader-error>
            {activeSlot.error}
          </div>
        ) : null}
        {!activeApiId ? <div className="row__hint">{t("instances.loaderVanillaHint")}</div> : null}
      </div>

      <div className="field">
        <label htmlFor="new-instance-name">{t("instances.newName")}</label>
        <input
          id="new-instance-name"
          type="text"
          value={name}
          placeholder={mcVersion}
          onChange={(event) => setName(event.target.value)}
        />
        <div className="row__hint">{t("instances.nameHint")}</div>
      </div>

      <div className="field">
        <label htmlFor="new-instance-memory">
          {t("instances.memory")}
          <span className="field__value">{t("instances.memoryValue", { value: memoryMb })}</span>
        </label>
        <input
          id="new-instance-memory"
          type="range"
          min={MIN_MEMORY}
          max={MAX_MEMORY}
          step={256}
          value={memoryMb}
          onChange={(event) => setMemoryMb(Number(event.target.value))}
        />
        <div className="row__hint">{t("instances.memoryHint")}</div>
      </div>

      {error ? <div className="field-error">{error}</div> : null}
    </Modal>
  );
}
