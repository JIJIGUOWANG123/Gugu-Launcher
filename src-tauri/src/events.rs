//! 事件出口：核心逻辑只依赖 Events 特征，Tauri 与 CLI/测试各自实现。
//!
//! 这样 install::Installer 与 launch 都能在没有 Tauri 运行时的环境
//! （examples / 单测）里跑，前端拿到的事件形状保持不变。

use serde::Serialize;

/// 下载/安装阶段（与前端 DownloadPhase 一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Queued,
    Downloading,
    Verifying,
    Extracting,
    Finished,
    Failed,
    Cancelled,
}

impl Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Queued => "queued",
            Phase::Downloading => "downloading",
            Phase::Verifying => "verifying",
            Phase::Extracting => "extracting",
            Phase::Finished => "finished",
            Phase::Failed => "failed",
            Phase::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Phase::Finished | Phase::Failed | Phase::Cancelled)
    }
}

/// install://progress 的载荷。
///
/// 同时带 done/total/percent（任务书约定）与
/// progress/receivedBytes/totalBytes（前端 InstallProgress），两种读法都能用。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub task_id: String,
    pub kind: String,
    pub title: String,
    pub phase: Phase,
    pub stage: String,
    pub done: u64,
    pub total: u64,
    pub percent: f64,
    pub progress: f64,
    pub bytes_per_sec: u64,
    pub received_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub target_dir: String,
}

/// install://error 的载荷。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallError {
    pub task_id: String,
    pub message: String,
}

/// 事件接收端：实现方负责把载荷转成 Tauri 事件或写日志。
pub trait Events: Send + Sync {
    fn install_progress(&self, payload: &InstallProgress);
    fn install_error(&self, payload: &InstallError);
    fn game_log(&self, instance_id: i64, line: &str);
    fn game_exit(&self, instance_id: i64, code: Option<i32>);

    /// 陶瓦联机的一行日志（tera://log）
    fn tera_log(&self, _line: &str) {}
    /// 陶瓦联机的状态变化（tera://state）
    fn tera_state(&self, _status: &crate::terracotta::TeraStatus) {}
}

/// 不关心事件的场景（examples、单测、CLI）。
pub struct NoopEvents;

impl Events for NoopEvents {
    fn install_progress(&self, _payload: &InstallProgress) {}
    fn install_error(&self, _payload: &InstallError) {}
    fn game_log(&self, _instance_id: i64, _line: &str) {}
    fn game_exit(&self, _instance_id: i64, _code: Option<i32>) {}
}

/// 把事件打到 tracing/eprintln（CLI 与 examples 用）。
pub struct LoggingEvents;

impl Events for LoggingEvents {
    fn install_progress(&self, payload: &InstallProgress) {
        tracing::info!(
            task = %payload.task_id,
            stage = %payload.stage,
            percent = payload.percent,
            "安装进度"
        );
    }

    fn install_error(&self, payload: &InstallError) {
        tracing::error!(task = %payload.task_id, message = %payload.message, "安装失败");
    }

    fn game_log(&self, instance_id: i64, line: &str) {
        eprintln!("[实例 {instance_id}] {line}");
    }

    fn game_exit(&self, instance_id: i64, code: Option<i32>) {
        tracing::info!(instance_id, code = ?code, "游戏进程退出");
    }
}

/// Tauri 事件出口：事件名固定为 install://progress、install://error、
/// game://log、game://exit。
pub struct TauriEvents {
    app: tauri::AppHandle,
}

impl TauriEvents {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameLogPayload<'a> {
    id: i64,
    instance_id: i64,
    line: &'a str,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameExitPayload {
    id: i64,
    instance_id: i64,
    code: Option<i32>,
}

impl Events for TauriEvents {
    fn install_progress(&self, payload: &InstallProgress) {
        use tauri::Emitter;
        if let Err(err) = self.app.emit("install://progress", payload) {
            tracing::warn!(%err, "发送 install://progress 失败");
        }
    }

    fn install_error(&self, payload: &InstallError) {
        use tauri::Emitter;
        if let Err(err) = self.app.emit("install://error", payload) {
            tracing::warn!(%err, "发送 install://error 失败");
        }
    }

    fn game_log(&self, instance_id: i64, line: &str) {
        use tauri::Emitter;
        let payload = GameLogPayload {
            id: instance_id,
            instance_id,
            line,
        };
        if let Err(err) = self.app.emit("game://log", payload) {
            tracing::warn!(%err, "发送 game://log 失败");
        }
    }

    fn game_exit(&self, instance_id: i64, code: Option<i32>) {
        use tauri::Emitter;
        let payload = GameExitPayload {
            id: instance_id,
            instance_id,
            code,
        };
        if let Err(err) = self.app.emit("game://exit", payload) {
            tracing::warn!(%err, "发送 game://exit 失败");
        }
    }

    fn tera_log(&self, line: &str) {
        use tauri::Emitter;
        if let Err(err) = self.app.emit("tera://log", serde_json::json!({ "line": line })) {
            tracing::warn!(%err, "发送 tera://log 失败");
        }
    }

    fn tera_state(&self, status: &crate::terracotta::TeraStatus) {
        use tauri::Emitter;
        if let Err(err) = self.app.emit("tera://state", status) {
            tracing::warn!(%err, "发送 tera://state 失败");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_serializes_to_frontend_names() {
        assert_eq!(serde_json::to_string(&Phase::Downloading).unwrap(), "\"downloading\"");
        assert_eq!(serde_json::to_string(&Phase::Extracting).unwrap(), "\"extracting\"");
        assert_eq!(serde_json::to_string(&Phase::Cancelled).unwrap(), "\"cancelled\"");
        assert!(Phase::Finished.is_terminal());
        assert!(!Phase::Downloading.is_terminal());
        assert_eq!(Phase::Queued.as_str(), "queued");
    }

    #[test]
    fn progress_payload_carries_both_shapes() {
        let payload = InstallProgress {
            task_id: "t1".into(),
            kind: "install".into(),
            title: "1.20.1".into(),
            phase: Phase::Downloading,
            stage: "正在下载 assets".into(),
            done: 5,
            total: 10,
            percent: 50.0,
            progress: 0.5,
            bytes_per_sec: 1024,
            received_bytes: Some(5),
            total_bytes: Some(10),
            target_dir: "D:\\minecraft\\.mc".into(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["taskId"], "t1");
        assert_eq!(json["percent"], 50.0);
        assert_eq!(json["progress"], 0.5);
        assert_eq!(json["phase"], "downloading");
        assert_eq!(json["bytesPerSec"], 1024);
        assert_eq!(json["totalBytes"], 10);
    }
}
