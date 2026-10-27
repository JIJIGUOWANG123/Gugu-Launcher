//! 下载任务注册表：downloads_list / download_cancel 与 install://progress 同源。
//!
//! 每个任务一条记录（内存里，退出即失），进度、阶段、取消标志都在这里，
//! 命令层只读这份数据，不自己拼状态。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::events::{Events, InstallError, InstallProgress, Phase};
use crate::util;

/// 最多保留的任务条数（老的已完成任务会被挤掉）。
const MAX_TASKS: usize = 50;

/// 同一条任务两条事件之间的最小间隔，避免把 IPC 打满。
const EMIT_INTERVAL_MS: u64 = 100;

struct Task {
    id: String,
    kind: String,
    title: String,
    stage: String,
    phase: Phase,
    progress: f64,
    bytes_per_sec: u64,
    received_bytes: Option<u64>,
    total_bytes: Option<u64>,
    target_dir: String,
    created_at: u64,
    finished_at: Option<u64>,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

/// downloads_list() 的元素（字段名与前端 DownloadTask 对齐）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTaskView {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub stage: String,
    pub phase: Phase,
    pub progress: f64,
    pub percent: f64,
    pub bytes_per_sec: u64,
    pub received_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub target_dir: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
}

/// 任务表：AppState 里只有一个实例，命令与安装流程共享。
#[derive(Clone, Default)]
pub struct TaskStore {
    inner: Arc<Mutex<Vec<Task>>>,
    events: Arc<Mutex<Option<Arc<dyn Events>>>>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 绑定事件出口（setup 阶段按 AppHandle 绑一次）。
    pub fn bind_events(&self, events: Arc<dyn Events>) {
        if let Ok(mut guard) = self.events.lock() {
            *guard = Some(events);
        }
    }

    fn events(&self) -> Option<Arc<dyn Events>> {
        self.events.lock().ok().and_then(|guard| guard.clone())
    }

    /// 新建任务并返回句柄；返回前先落一条 queued 记录。
    pub fn create(&self, kind: &str, title: &str, target_dir: &str) -> TaskHandle {
        self.create_with_prefix(kind, title, target_dir, "task")
    }

    /// 带自定义 id 前缀（GitHub 下载用 github- 前缀，前端好区分）。
    pub fn create_with_prefix(
        &self,
        kind: &str,
        title: &str,
        target_dir: &str,
        prefix: &str,
    ) -> TaskHandle {
        let id = format!("{prefix}-{}", util::unique_suffix());
        let cancel = Arc::new(AtomicBool::new(false));
        let task = Task {
            id: id.clone(),
            kind: kind.to_string(),
            title: title.to_string(),
            stage: "排队中".to_string(),
            phase: Phase::Queued,
            progress: 0.0,
            bytes_per_sec: 0,
            received_bytes: None,
            total_bytes: None,
            target_dir: target_dir.to_string(),
            created_at: util::now_unix(),
            finished_at: None,
            error: None,
            cancel: cancel.clone(),
        };
        if let Ok(mut guard) = self.inner.lock() {
            guard.push(task);
            while guard.len() > MAX_TASKS {
                // 只淘汰最早结束的任务，正在跑的一个都不动
                match guard.iter().position(|t| t.phase.is_terminal()) {
                    Some(index) => {
                        guard.remove(index);
                    }
                    None => break,
                }
            }
        }
        TaskHandle {
            store: self.clone(),
            id,
            cancel,
            last_emit: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn list(&self) -> Vec<DownloadTaskView> {
        let Ok(guard) = self.inner.lock() else {
            return Vec::new();
        };
        let mut out: Vec<DownloadTaskView> = guard.iter().map(view_of).collect();
        // 新的在前
        out.reverse();
        out
    }

    /// 取消：置标志位并把还在跑的任务标成 cancelled。未知 id 报可读错误。
    pub fn cancel(&self, id: &str) -> Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| AppError::Other("任务列表锁已损坏".to_string()))?;
        let Some(task) = guard.iter_mut().find(|task| task.id == id) else {
            return Err(AppError::Invalid(format!("下载任务不存在：{id}")));
        };
        if task.phase.is_terminal() {
            return Ok(());
        }
        task.cancel.store(true, Ordering::SeqCst);
        task.phase = Phase::Cancelled;
        task.stage = "正在取消".to_string();
        task.bytes_per_sec = 0;
        task.finished_at = Some(util::now_unix());
        Ok(())
    }

    /// download_open_folder 用：任务落地目录。
    pub fn target_dir(&self, id: &str) -> Option<String> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| {
                guard
                    .iter()
                    .find(|task| task.id == id)
                    .map(|task| task.target_dir.clone())
            })
            .filter(|dir| !dir.trim().is_empty())
    }

    pub fn is_cancelled(&self, id: &str) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| {
                guard
                    .iter()
                    .find(|task| task.id == id)
                    .map(|task| task.cancel.load(Ordering::SeqCst))
            })
            .unwrap_or(false)
    }

    /// 写入一条进度并（按节流）发事件。
    fn publish(&self, handle: &TaskHandle, stage: &str, phase: Phase, done: u64, total: u64, force: bool) {
        let progress = if total > 0 {
            (done as f64 / total as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let payload = {
            let Ok(mut guard) = self.inner.lock() else {
                return;
            };
            let Some(task) = guard.iter_mut().find(|task| task.id == handle.id) else {
                return;
            };
            if task.phase.is_terminal() && phase != Phase::Finished {
                // 已被取消/已结束：不再覆盖终态
                return;
            }
            task.stage = stage.to_string();
            task.phase = phase;
            task.progress = progress;
            task.received_bytes = Some(done);
            task.total_bytes = Some(total);
            if phase == Phase::Finished {
                task.bytes_per_sec = 0;
                task.finished_at = Some(util::now_unix());
            }
            InstallProgress {
                task_id: task.id.clone(),
                kind: task.kind.clone(),
                title: task.title.clone(),
                phase: task.phase,
                stage: task.stage.clone(),
                done,
                total,
                percent: (progress * 100.0 * 100.0).round() / 100.0,
                progress,
                bytes_per_sec: task.bytes_per_sec,
                received_bytes: task.received_bytes,
                total_bytes: task.total_bytes,
                target_dir: task.target_dir.clone(),
                // total 未知时不能报 0%：标记 indeterminate，前端按字节数展示
                indeterminate: total == 0 && phase != Phase::Finished,
            }
        };

        let now = util::now_millis() as u64;
        let last = handle.last_emit.load(Ordering::Relaxed);
        let due = force || phase.is_terminal() || now.saturating_sub(last) >= EMIT_INTERVAL_MS;
        if !due {
            return;
        }
        handle.last_emit.store(now, Ordering::Relaxed);
        if let Some(events) = self.events() {
            events.install_progress(&payload);
        }
    }

    fn set_speed(&self, id: &str, bytes_per_sec: u64) {
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(task) = guard.iter_mut().find(|task| task.id == id) {
                task.bytes_per_sec = bytes_per_sec;
            }
        }
    }
}

fn view_of(task: &Task) -> DownloadTaskView {
    DownloadTaskView {
        id: task.id.clone(),
        kind: task.kind.clone(),
        title: task.title.clone(),
        stage: task.stage.clone(),
        phase: task.phase,
        progress: task.progress,
        percent: (task.progress * 100.0 * 100.0).round() / 100.0,
        bytes_per_sec: task.bytes_per_sec,
        received_bytes: task.received_bytes,
        total_bytes: task.total_bytes,
        target_dir: task.target_dir.clone(),
        created_at: util::iso8601(task.created_at),
        finished_at: task.finished_at.map(util::iso8601),
        error: task.error.clone(),
    }
}

/// 安装/下载流程手里的任务句柄。
#[derive(Clone)]
pub struct TaskHandle {
    store: TaskStore,
    id: String,
    cancel: Arc<AtomicBool>,
    last_emit: Arc<AtomicU64>,
}

impl TaskHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst) || self.store.is_cancelled(&self.id)
    }

    /// 与 store.cancel() 共享的取消标志：下载器拿它做在飞请求的短路。
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// 每轮循环调用一次：已被取消就直接短路，避免继续下载。
    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancelled() {
            return Err(AppError::Cancelled);
        }
        Ok(())
    }

    pub fn progress(&self, phase: Phase, stage: &str, done: u64, total: u64) {
        self.store.publish(self, stage, phase, done, total, false);
    }

    pub fn progress_forced(&self, phase: Phase, stage: &str, done: u64, total: u64) {
        self.store.publish(self, stage, phase, done, total, true);
    }

    pub fn set_speed(&self, bytes_per_sec: u64) {
        self.store.set_speed(&self.id, bytes_per_sec);
    }

    /// 正常结束。
    pub fn finish(&self) {
        self.store.publish(self, "", Phase::Finished, 1, 1, true);
    }

    /// 失败：落终态 + 发 install://error。
    pub fn fail(&self, message: &str) {
        {
            if let Ok(mut guard) = self.store.inner.lock() {
                if let Some(task) = guard.iter_mut().find(|task| task.id == self.id) {
                    task.phase = Phase::Failed;
                    task.stage = message.to_string();
                    task.error = Some(message.to_string());
                    task.bytes_per_sec = 0;
                    task.finished_at = Some(util::now_unix());
                }
            }
        }
        if let Some(events) = self.store.events() {
            events.install_error(&InstallError {
                task_id: self.id.clone(),
                message: message.to_string(),
            });
        }
    }

    /// 被取消：落 cancelled 终态，不报错。
    pub fn cancel(&self) {
        {
            if let Ok(mut guard) = self.store.inner.lock() {
                if let Some(task) = guard.iter_mut().find(|task| task.id == self.id) {
                    if !task.phase.is_terminal() {
                        task.phase = Phase::Cancelled;
                        task.stage = "已取消".to_string();
                        task.bytes_per_sec = 0;
                        task.finished_at = Some(util::now_unix());
                    }
                }
            }
        }
        self.cancel.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_list_and_cancel_roundtrip() {
        let store = TaskStore::new();
        let handle = store.create("install", "1.20.1", "D:\\minecraft\\.mc\\versions\\1.20.1");
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.list()[0].phase, Phase::Queued);
        assert_eq!(store.list()[0].title, "1.20.1");
        assert!(!handle.cancelled());

        handle.progress(Phase::Downloading, "正在下载 client.jar", 5, 10);
        let view = &store.list()[0];
        assert_eq!(view.phase, Phase::Downloading);
        assert_eq!(view.progress, 0.5);
        assert_eq!(view.percent, 50.0);
        assert_eq!(view.stage, "正在下载 client.jar");

        store.cancel(handle.id()).expect("取消应成功");
        assert!(handle.cancelled());
        assert!(handle.check_cancelled().is_err());
        assert_eq!(store.list()[0].phase, Phase::Cancelled);
        assert!(store.list()[0].finished_at.is_some());
    }

    #[test]
    fn unknown_id_reports_readable_error() {
        let store = TaskStore::new();
        let err = store.cancel("nope").unwrap_err();
        assert!(err.to_string().contains("下载任务不存在"));
    }

    #[test]
    fn finished_task_is_terminal_and_keeps_progress() {
        let store = TaskStore::new();
        let handle = store.create("java", "Java 17", "D:\\minecraft\\.mc\\runtimes\\temurin-17");
        handle.progress(Phase::Downloading, "下载中", 3, 6);
        handle.finish();
        let view = &store.list()[0];
        assert_eq!(view.phase, Phase::Finished);
        assert_eq!(view.progress, 1.0);
        assert_eq!(view.bytes_per_sec, 0);
        // 终态之后不再被覆盖
        handle.progress(Phase::Downloading, "又下载", 1, 6);
        assert_eq!(store.list()[0].phase, Phase::Finished);
    }

    #[test]
    fn fail_records_error_message() {
        let store = TaskStore::new();
        let handle = store.create("install", "1.20.1", "D:\\minecraft\\.mc");
        handle.fail("所有下载源都失败");
        let view = &store.list()[0];
        assert_eq!(view.phase, Phase::Failed);
        assert_eq!(view.error.as_deref(), Some("所有下载源都失败"));
        assert!(view.finished_at.is_some());
    }

    #[test]
    fn task_cap_drops_oldest_finished_only() {
        let store = TaskStore::new();
        let running = store.create("install", "跑着的", "dir");
        for index in 0..(MAX_TASKS + 5) {
            let handle = store.create("install", &format!("t{index}"), "dir");
            handle.finish();
        }
        let list = store.list();
        assert_eq!(list.len(), MAX_TASKS);
        assert!(list.iter().any(|view| view.id == running.id()));
    }
}
