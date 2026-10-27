import { icons } from "../assets/icons";
import { useT } from "../lib/i18n";
import type { Instance } from "../lib/types";
import { InstanceIcon } from "./InstanceIcon";

interface Props {
  instance: Instance;
  onLaunch: (instance: Instance) => void;
  onOpenFolder: (instance: Instance) => void;
  onDetail: (instance: Instance) => void;
  /** 右键菜单：把鼠标坐标交回页面，由页面统一渲染菜单 */
  onContextMenu?: (instance: Instance, x: number, y: number) => void;
}

/** 实例卡：图标 + 名称 + 版本/加载器徽标 + 三个按钮（启动 / 打开文件夹 / 详情）。 */
export function InstanceCard({ instance, onLaunch, onOpenFolder, onDetail, onContextMenu }: Props) {
  const t = useT();
  return (
    <article
      className="instance-card"
      data-instance-id={instance.id}
      onContextMenu={(event) => {
        if (!onContextMenu) return;
        event.preventDefault();
        onContextMenu(instance, event.clientX, event.clientY);
      }}
    >
      <div className="instance-card__head">
        <InstanceIcon className="instance-card__icon" icon={instance.icon} alt={instance.name} />
        <div className="instance-card__meta">
          <div className="instance-card__title" title={instance.name}>
            {instance.name}
          </div>
          <div className="instance-card__version">
            <span className="version-badge">{instance.mcVersion}</span>
            <span className="loader-badge">{instance.loader || "Vanilla"}</span>
          </div>
        </div>
      </div>
      <div className="instance-card__divider" />
      <div className="instance-card__actions">
        <button className="card-action card-action--primary" title={t("instances.launch")} onClick={() => onLaunch(instance)}>
          <span className="play-triangle" aria-hidden="true" />
        </button>
        <button className="card-action" title={t("instances.folder")} onClick={() => onOpenFolder(instance)}>
          <img src={icons.folder} alt={t("instances.folder")} />
        </button>
        <button
          className="card-action"
          title={t("instances.detail")}
          onClick={() => onDetail(instance)}
        >
          <img src={icons.gear} alt={t("instances.detail")} />
        </button>
      </div>
    </article>
  );
}
