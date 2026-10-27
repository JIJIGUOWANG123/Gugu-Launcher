/**
 * 下载中心的唯一数据源：downloads_list() + install://* 事件。
 * 顶部角标、悬浮面板、设置页里的「下载中心」都读这一份，不各自实现。
 */
import { useEffect, useMemo, useSyncExternalStore } from "react";
import { api, subscribeLauncherEvents } from "./tauri-api";
import type { DownloadTask } from "./types";

const RUNNING: ReadonlyArray<DownloadTask["phase"]> = ["queued", "downloading", "verifying", "extracting"];

let tasks: DownloadTask[] = [];
let started = false;
let refreshSeq = 0;
let unsubscribeEvents: (() => void) | null = null;

const listeners = new Set<() => void>();

function notify(): void {
  listeners.forEach((listener) => listener());
}

async function refresh(): Promise<void> {
  refreshSeq += 1;
  const seq = refreshSeq;
  try {
    const list = await api.downloadsList();
    if (seq !== refreshSeq) return;
    tasks = list;
    notify();
  } catch {
    /* 列表拉不到就保持上一次的快照，界面不闪 */
  }
}

function patchProgress(payload: {
  taskId: string;
  title?: string;
  phase: DownloadTask["phase"];
  stage: string;
  progress: number;
  bytesPerSec: number;
  receivedBytes: number | null;
  totalBytes: number | null;
}): void {
  const single = tasks.find((task) => task.id === payload.taskId);
  if (!single) {
    void refresh();
    return;
  }
  tasks = tasks.map((task) =>
    task.id === payload.taskId
      ? {
          ...task,
          title: payload.title ?? task.title,
          phase: payload.phase,
          stage: payload.stage,
          progress: payload.progress,
          bytesPerSec: payload.bytesPerSec,
          receivedBytes: payload.receivedBytes,
          totalBytes: payload.totalBytes,
          finishedAt: payload.phase === "finished" ? task.finishedAt ?? new Date().toISOString() : task.finishedAt,
        }
      : task,
  );
  notify();
}

function ensureStarted(): void {
  if (started) return;
  started = true;
  void refresh();
  void subscribeLauncherEvents((event) => {
    if (event.kind === "install-progress") {
      patchProgress(event.payload);
      return;
    }
    if (event.kind === "install-error") {
      const { taskId, message } = event.payload;
      tasks = tasks.map((task) =>
        task.id === taskId ? { ...task, phase: "failed", stage: message, error: message, bytesPerSec: 0 } : task,
      );
      notify();
      void refresh();
    }
  })
    .then((off) => {
      unsubscribeEvents = off;
    })
    .catch(() => {
      /* 事件订阅失败不影响列表展示 */
    });
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  ensureStarted();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && unsubscribeEvents) {
      unsubscribeEvents();
      unsubscribeEvents = null;
      started = false;
    }
  };
}

function getSnapshot(): DownloadTask[] {
  return tasks;
}

export interface DownloadsView {
  tasks: DownloadTask[];
  running: DownloadTask[];
  finished: DownloadTask[];
  /** 顶部圆点的角标数字：进行中任务数 */
  activeCount: number;
  cancel: (id: string) => void;
  openFolder: (id: string) => void;
  refresh: () => void;
}

export function useDownloads(): DownloadsView {
  const list = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  useEffect(() => {
    ensureStarted();
  }, []);

  return useMemo(() => {
    const running = list.filter((task) => RUNNING.includes(task.phase));
    const finished = list
      .filter((task) => !RUNNING.includes(task.phase))
      .slice()
      .sort((a, b) => (b.finishedAt ?? b.createdAt).localeCompare(a.finishedAt ?? a.createdAt));
    return {
      tasks: list,
      running,
      finished,
      activeCount: running.length,
      cancel: (id: string) => {
        void api
          .downloadCancel(id)
          .catch(() => undefined)
          .then(() => refresh());
      },
      openFolder: (id: string) => {
        void api.downloadOpenFolder(id).catch(() => undefined);
      },
      refresh: () => {
        void refresh();
      },
    };
  }, [list]);
}
