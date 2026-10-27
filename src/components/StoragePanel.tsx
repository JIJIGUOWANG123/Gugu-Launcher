import { useCallback, useEffect, useMemo, useState } from "react";
import { icons } from "../assets/icons";
import { formatBytes } from "../lib/format";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { StorageKey, StorageStats } from "../lib/types";
import { Modal } from "./Modal";

interface Props {
  notify: (message: string) => void;
}

/** 清理动作：which 决定确认弹窗的文案。 */
type CleanAction = { key: StorageKey; titleKey: string; descKey: string };

const CLEAN_ACTIONS: Record<string, CleanAction> = {
  versions: { key: "versions", titleKey: "storage.cleanVersions", descKey: "storage.cleanVersionsDesc" },
  cache: { key: "cache", titleKey: "storage.cleanCache", descKey: "storage.cleanCacheDesc" },
  logs: { key: "logs", titleKey: "storage.cleanLogs", descKey: "storage.cleanLogsDesc" },
  assets: { key: "assets", titleKey: "storage.verifyAssets", descKey: "storage.verifyAssetsDesc" },
};

const ITEM_META: Record<StorageKey, { labelKey: string; action?: "open" | "clean" | "verify" | "logs" }> = {
  instances: { labelKey: "storage.item.instances", action: "open" },
  versions: { labelKey: "storage.item.versions", action: "clean" },
  libraries: { labelKey: "storage.item.libraries" },
  assets: { labelKey: "storage.item.assets", action: "verify" },
  cache: { labelKey: "storage.item.cache", action: "clean" },
  logs: { labelKey: "storage.item.logs", action: "logs" },
  runtimes: { labelKey: "storage.item.runtimes", action: "open" },
};

const BAR_COLORS = ["#f5c845", "#4c9aff", "#3ecf8e", "#9b7bff", "#fb923c", "#f27ba6", "#8a93a5"];

/** 设置 → 存储：总占用 + 七个分项 + 占比条 + 清理（二次确认）。 */
export function StoragePanel({ notify }: Props) {
  const t = useT();
  const [stats, setStats] = useState<StorageStats | null>(null);
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState<CleanAction | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setStats(await api.storageStats());
      setFailed(false);
    } catch (error) {
      setFailed(true);
      notify(String(error));
    }
  }, [notify]);

  useEffect(() => {
    void load();
  }, [load]);

  const total = stats?.totalBytes ?? 0;

  const segments = useMemo(() => {
    const items = stats?.items ?? [];
    return items.map((item, index) => ({
      key: item.key,
      label: t(ITEM_META[item.key].labelKey),
      percent: total > 0 ? item.bytes / total : 0,
      color: BAR_COLORS[index % BAR_COLORS.length],
    }));
  }, [stats, total, t]);

  const runClean = async () => {
    if (!pending) return;
    const action = pending;
    setPending(null);
    setBusy(true);
    try {
      const result = await api.storageClean(action.key);
      await load();
      notify(t("storage.cleaned", { size: formatBytes(result.freedBytes) }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="storage-panel">
      <section className="card settings-card">
        <div className="storage-total">
          <span className="settings-card__title" data-storage-total>
            {t("storage.total", { size: formatBytes(total) })}
          </span>
          <button
            type="button"
            className="pill-outline"
            data-action="storage-open-root"
            onClick={() => void api.openExternal("").catch(() => undefined)}
          >
            <img src={icons.folder} alt="" />
            {t("storage.openFolder")}
          </button>
        </div>
        <div className="storage-bar" data-storage-bar>
          {segments.map((segment) => (
            <span
              key={segment.key}
              className="storage-bar__seg"
              data-storage-seg={segment.key}
              title={segment.label + " " + Math.round(segment.percent * 100) + "%"}
              style={{ width: Math.max(0.6, segment.percent * 100) + "%", background: segment.color }}
            />
          ))}
        </div>
      </section>

      {failed ? (
        <div className="empty-hint" data-storage-error>
          {t("content.loadFailed")}
          <button type="button" className="pill-outline" onClick={() => void load()}>
            {t("content.retry")}
          </button>
        </div>
      ) : null}
      <div className="storage-grid">
        {(stats?.items ?? []).map((item) => {
          const meta = ITEM_META[item.key];
          return (
            <section className="card settings-card storage-item" key={item.key} data-storage-item={item.key}>
              <div className="storage-item__head">
                <span className="storage-item__name">{t(meta.labelKey)}</span>
                <span className="storage-item__size" data-storage-size>
                  {formatBytes(item.bytes)}
                </span>
              </div>
              <div className="row__hint" data-storage-files>
                {t("storage.files", { count: item.files })} · <span className="mono">{item.path}</span>
              </div>
              <div className="storage-item__actions">
                {meta.action === "open" ? (
                  <button
                    type="button"
                    className="pill-outline pill-outline--tiny"
                    data-action="storage-open"
                    onClick={() => void api.openExternal(item.path).catch((error) => notify(String(error)))}
                  >
                    {t("storage.openFolder")}
                  </button>
                ) : null}
                {item.key === "versions" ? (
                  <button
                    type="button"
                    className="pill-outline pill-outline--tiny"
                    data-action="storage-clean-versions"
                    disabled={busy}
                    onClick={() => setPending(CLEAN_ACTIONS.versions)}
                  >
                    {t("storage.cleanVersions")}
                  </button>
                ) : null}
                {item.key === "assets" ? (
                  <button type="button" className="pill-outline pill-outline--tiny" data-action="storage-verify" disabled>
                    {t("storage.verifyAssets")}
                  </button>
                ) : null}
                {item.key === "cache" ? (
                  <button
                    type="button"
                    className="pill-outline pill-outline--tiny"
                    data-action="storage-clean-cache"
                    disabled={busy}
                    onClick={() => setPending(CLEAN_ACTIONS.cache)}
                  >
                    {t("storage.cleanCache")}
                  </button>
                ) : null}
                {item.key === "logs" ? (
                  <button
                    type="button"
                    className="pill-outline pill-outline--tiny"
                    data-action="storage-clean-logs"
                    disabled={busy}
                    onClick={() => setPending(CLEAN_ACTIONS.logs)}
                  >
                    {t("storage.cleanLogs")}
                  </button>
                ) : null}
              </div>
            </section>
          );
        })}
      </div>

      {pending ? (
        <Modal
          title={t(pending.titleKey)}
          onClose={() => setPending(null)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setPending(null)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="storage-clean-confirm" onClick={() => void runClean()}>
                {t("common.confirm")}
              </button>
            </>
          }
        >
          <p>{t(pending.descKey)}</p>
        </Modal>
      ) : null}
    </div>
  );
}
