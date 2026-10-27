import { useEffect, useMemo, useState } from "react";
import { icons } from "../assets/icons";
import { InstanceCard } from "../components/InstanceCard";
import { Modal } from "../components/Modal";
import { NewInstanceModal, type NewInstancePayload } from "../components/NewInstanceModal";
import { formatDuration } from "../lib/format";
import { useT } from "../lib/i18n";
import type { Instance, VersionManifest } from "../lib/types";
import { InstanceDetailPage } from "./InstanceDetailPage";

interface Props {
  instances: Instance[];
  versions: VersionManifest | null;
  onCreate: (payload: NewInstancePayload) => Promise<void>;
  onDelete: (instance: Instance) => Promise<void>;
  onLaunch: (instance: Instance) => void;
  onOpenFolder: (instance: Instance) => void;
  onRefresh: () => Promise<void>;
  notify: (message: string) => void;
  /** 详情页缺 Java 时跳设置 */
  onOpenJavaSettings?: () => void;
}

interface MenuState {
  instance: Instance;
  x: number;
  y: number;
}

/**
 * 实例页：统计条 + 三个胶囊按钮 + 实例卡网格 + 右键菜单，
 * 点「详情」在同一个页面里切到实例详情（不换主导航）。
 */
export function InstancesPage({ instances, versions, onCreate, onDelete, onLaunch, onOpenFolder, onRefresh, notify, onOpenJavaSettings }: Props) {
  const t = useT();
  const [creating, setCreating] = useState(false);
  const [detail, setDetail] = useState<Instance | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Instance | null>(null);

  // 统计条：累计时长 + 最常游玩版本（按 mcVersion 聚合 playTimeSecs）
  const totalPlay = useMemo(() => instances.reduce((sum, item) => sum + item.playTimeSecs, 0), [instances]);
  const mostPlayed = useMemo(() => {
    const byVersion = new Map<string, number>();
    for (const item of instances) {
      byVersion.set(item.mcVersion, (byVersion.get(item.mcVersion) ?? 0) + item.playTimeSecs);
    }
    const sorted = [...byVersion.entries()].sort((a, b) => b[1] - a[1]);
    return sorted[0]?.[0] ?? null;
  }, [instances]);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenu(null);
    };
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
    };
  }, [menu]);

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    const target = deleteTarget;
    setDeleteTarget(null);
    try {
      await onDelete(target);
    } catch (error) {
      notify(String(error));
    }
  };

  // 详情页：改名后 detail 里的旧对象不再准确，返回列表时用最新的 instances 重新取一次
  if (detail) {
    const fresh = instances.find((item) => item.id === detail.id) ?? detail;
    return (
      <InstanceDetailPage
        instance={fresh}
        onBack={() => setDetail(null)}
        onLaunch={onLaunch}
        onSaved={onRefresh}
        notify={notify}
        onOpenJavaSettings={onOpenJavaSettings}
      />
    );
  }

  return (
    <div>
      <section className="card stats-bar">
        <div className="stats-bar__item">
          <span>{t("instances.totalPlayTime")}：</span>
          <strong>{formatDuration(totalPlay)}</strong>
        </div>
        <div className="stats-bar__item">
          <span>{t("instances.mostPlayed")}：</span>
          <strong>{mostPlayed ?? "—"}</strong>
        </div>
      </section>

      <div className="instances-toolbar">
        <button className="pill-outline" onClick={() => notify(t("instances.importLater"))}>
          + {t("instances.importPack")}
        </button>
        <button className="pill-outline" onClick={() => notify(t("instances.importLater"))}>
          + {t("instances.importModpack")}
        </button>
        <button className="pill-outline pill-outline--primary" onClick={() => setCreating(true)}>
          + {t("instances.create")}
        </button>
      </div>

      {instances.length === 0 ? (
        <div className="empty-hint">{t("instances.empty")}</div>
      ) : (
        <div className="instance-grid">
          {instances.map((instance) => (
            <InstanceCard
              key={instance.id}
              instance={instance}
              onLaunch={onLaunch}
              onOpenFolder={onOpenFolder}
              onDetail={(target) => setDetail(target)}
              onContextMenu={(target, x, y) =>
                setMenu({
                  instance: target,
                  x: Math.min(x, window.innerWidth - 208),
                  y: Math.min(y, window.innerHeight - 148),
                })
              }
            />
          ))}
        </div>
      )}

      {menu ? (
        <div className="ctx-menu" style={{ left: menu.x, top: menu.y }} role="menu" aria-label={t("instances.menuTitle")}>
          <button
            className="ctx-menu__item"
            role="menuitem"
            onClick={() => {
              onOpenFolder(menu.instance);
              setMenu(null);
            }}
          >
            <img src={icons.folder} alt="" />
            {t("instances.menuOpenFolder")}
          </button>
          <button
            className="ctx-menu__item ctx-menu__item--danger"
            role="menuitem"
            onClick={() => {
              setDeleteTarget(menu.instance);
              setMenu(null);
            }}
          >
            <img src={icons.trashDanger} alt="" />
            {t("instances.menuDelete")}
          </button>
        </div>
      ) : null}

      {creating ? (
        <NewInstanceModal
          versions={versions}
          onClose={() => setCreating(false)}
          onSubmit={async (payload) => {
            await onCreate(payload);
          }}
        />
      ) : null}

      {deleteTarget ? (
        <Modal
          title={t("instances.menuDelete")}
          onClose={() => setDeleteTarget(null)}
          footer={
            <>
              <button onClick={() => setDeleteTarget(null)}>{t("common.cancel")}</button>
              <button className="pill-button" onClick={() => void confirmDelete()}>
                {t("common.delete")}
              </button>
            </>
          }
        >
          <div>{t("instances.deleteConfirm", { name: deleteTarget.name })}</div>
        </Modal>
      ) : null}
    </div>
  );
}
