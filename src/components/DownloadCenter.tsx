import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { icons } from "../assets/icons";
import { useDownloads } from "../lib/downloads-store";
import { formatBytes, formatPercent, formatSpeed, formatStamp } from "../lib/format";
import { useT } from "../lib/i18n";
import type { DownloadTask } from "../lib/types";

interface Props {
  /** popover＝右上角下载图标的悬浮面板；inline＝设置页里的「下载中心」卡片 */
  variant?: "popover" | "inline";
  onClose?: () => void;
}

type Tab = "running" | "done";

/**
 * 下载中心：进行中（进度条 + 速度 + 阶段文字 + 取消）与已完成（时间 + 打开文件夹）。
 * 数据来自 lib/downloads-store 的同一份快照，弹窗与设置页复用同一个组件。
 */
export function DownloadCenter({ variant = "popover", onClose }: Props) {
  const t = useT();
  const { running, finished, cancel, openFolder } = useDownloads();
  const [tab, setTab] = useState<Tab>("running");

  useEffect(() => {
    if (variant !== "popover" || !onClose) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [variant, onClose]);

  // 悬浮面板出现时底层同样模糊（复用 Modal 的 body.modal-open 约定）
  useEffect(() => {
    if (variant !== "popover") return;
    document.body.classList.add("modal-open");
    return () => document.body.classList.remove("modal-open");
  }, [variant]);

  const list = tab === "running" ? running : finished;

  const renderTask = (task: DownloadTask) => {
    const isRunning = task.phase !== "finished" && task.phase !== "failed" && task.phase !== "cancelled";
    return (
      <article className={"dl-item" + (isRunning ? "" : " dl-item--done")} key={task.id}>
        <div className="dl-item__top">
          <span className="dl-item__title" title={task.title}>
            {task.title}
          </span>
          {isRunning ? (
            <button className="dl-item__icon" title={t("downloads.cancel")} onClick={() => cancel(task.id)}>
              <img src={icons.close} alt={t("downloads.cancel")} />
            </button>
          ) : (
            <button className="dl-item__link" onClick={() => openFolder(task.id)}>
              {t("downloads.openFolder")}
            </button>
          )}
        </div>
        {isRunning ? (
          <>
            <div className="dl-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(task.progress * 100)}>
              <div className={"dl-bar__fill" + (task.phase === "failed" ? " is-error" : "")} style={{ width: formatPercent(task.progress) }} />
            </div>
            <div className="dl-item__meta">
              <span className="dl-item__stage" title={task.stage}>
                {task.stage || t("downloads.phase." + task.phase)}
              </span>
              <span className="dl-item__speed">
                {formatPercent(task.progress)} · {formatSpeed(task.bytesPerSec)}
              </span>
            </div>
            {task.error ? <div className="dl-item__error">{task.error}</div> : null}
          </>
        ) : (
          <div className="dl-item__meta">
            <span>{t("downloads.finishedAt", { time: formatStamp(task.finishedAt) })}</span>
            <span>{formatBytes(task.totalBytes)}</span>
          </div>
        )}
      </article>
    );
  };

  const panel = (
    <div
      className={"dlc" + (variant === "popover" ? " dlc--popover" : " dlc--inline")}
      role={variant === "popover" ? "dialog" : undefined}
      aria-modal={variant === "popover" ? false : undefined}
      aria-label={t("downloads.title")}
    >
      {variant === "popover" ? (
        <div className="dlc__head">
          <span className="dlc__title">
            <img src={icons.download} alt="" />
            {t("titlebar.download")}
          </span>
          <button className="icon-button" title={t("common.close")} onClick={onClose}>
            <img src={icons.close} alt={t("common.close")} />
          </button>
        </div>
      ) : null}

      <div className="dlc__tabs" role="tablist">
        <button
          role="tab"
          aria-selected={tab === "running"}
          className={"dlc__tab" + (tab === "running" ? " is-active" : "")}
          onClick={() => setTab("running")}
        >
          {t("downloads.running")}
          {running.length > 0 ? <span className="dlc__count">{running.length}</span> : null}
        </button>
        <button
          role="tab"
          aria-selected={tab === "done"}
          className={"dlc__tab" + (tab === "done" ? " is-active" : "")}
          onClick={() => setTab("done")}
        >
          {t("downloads.done")}
        </button>
      </div>

      <div className="dlc__body">
        {list.length === 0 ? (
          <div className="dlc__empty">
            <img src={icons.download} alt="" />
            <span>{tab === "running" ? t("downloads.emptyRunning") : t("downloads.emptyDone")}</span>
          </div>
        ) : (
          <div className="dlc__list">{list.map(renderTask)}</div>
        )}
      </div>
    </div>
  );

  if (variant === "inline") return panel;

  return createPortal(
    <>
      <div className="dlc-backdrop" onClick={onClose} />
      {panel}
    </>,
    document.body,
  );
}
