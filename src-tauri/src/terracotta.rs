//! 陶瓦联机（Terracotta）：完整协议移植（对照 备份\0.5.6\electron\src\main\net\terracotta.ts）。
//!
//! - 启动：terracotta.exe --hmcl <portFile>，父进程写 {"port":N}；轮询 100ms 最多 9s，再 GET /meta 最多 5s。
//! - 状态：每 500ms GET /state，index 单调递增（会跳号），过期响应丢弃；profiles[].name/machine_id 是玩家。
//! - 主机：GET /state/scanning?player=&public_nodes=…；加入：GET /state/guesting?room=&player=…；
//!   复位：GET /state/ide；停止：GET /panic?peaceful=true 后按端口反查 PID + taskkill /T /F 兜底。
//! - 专用服务器：替服务器向 224.0.2.60:4445 发 [MOTD]…[/MOTD][AD]端口[/AD]（每 1.5s）。
//! 简化处：不做全局单实例复用（secondary），每次启动用自己的端口文件。

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::events::Phase;
use crate::paths::Paths;
use crate::tasks::TaskHandle;
use crate::util;

pub const RELEASE: &str = "v0.4.2";
pub const ARCHIVE_NAME: &str = "terracotta-0.4.2-windows-x86_64-pkg.tar.gz";
pub const EXE_NAME: &str = "terracotta.exe";
pub const DLL_NAME: &str = "VCRUNTIME140.DLL";
pub const REQUIRED_FILES: [&str; 2] = [EXE_NAME, DLL_NAME];
pub const PUBLIC_NODES: [&str; 4] = [
    "tcp://public.easytier.top:11010",
    "tcp://public2.easytier.cn:54321",
    "https://etnode.zkitefly.eu.org/node1",
    "https://etnode.zkitefly.eu.org/node2",
];
pub const STATE_POLL_MS: u64 = 500;
pub const PORT_WAIT_MS: u64 = 9000;
pub const PORT_POLL_MS: u64 = 100;
pub const META_WAIT_MS: u64 = 5000;
pub const HTTP_TIMEOUT_MS: u64 = 4000;
pub const EXIT_GRACE_MS: u64 = 2000;
pub const LAN_GROUP: &str = "224.0.2.60";
pub const LAN_PORT: u16 = 4445;
pub const BROADCAST_MS: u64 = 1500;
pub const BROADCAST_MOTD: &str = "NaiWa Server";
pub const DEFAULT_SERVER_PORT: u16 = 25565;
pub const BROADCAST_MAX_FAILURES: u32 = 3;

pub fn mirrors() -> Vec<String> {
    vec![
        format!("https://github.com/burningtnt/Terracotta/releases/download/{RELEASE}/{ARCHIVE_NAME}"),
        format!("https://gitee.com/burningtnt/Terracotta/releases/download/{RELEASE}/{ARCHIVE_NAME}"),
    ]
}

pub fn terracotta_dir(paths: &Paths) -> PathBuf {
    paths.root.join("terracotta")
}

pub fn exe_path(paths: &Paths) -> PathBuf {
    terracotta_dir(paths).join(EXE_NAME)
}

pub fn port_file(paths: &Paths, pid: u32) -> PathBuf {
    terracotta_dir(paths).join(format!("port-{pid}.json"))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeraPlayer {
    pub name: String,
    /// 旧实现叫 machine_id，前端契约叫 address
    pub address: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeraStatus {
    pub installed: bool,
    pub version: String,
    pub hosting: bool,
    pub players: Vec<TeraPlayer>,
    pub room_code: String,
    pub error: Option<String>,
    /// waiting / host-scanning / hosting / guest-connecting / guesting / exception
    pub state: String,
}

/// 陶瓦运行状态（apply_state 是纯函数，测试直接构造它）。
pub(crate) struct Inner {
    child: Option<std::process::Child>,
    port: Option<u16>,
    state: String,
    room: String,
    players: Vec<TeraPlayer>,
    error: Option<String>,
    version: String,
    index: i64,
    stop_flag: Arc<AtomicBool>,
    poller: Option<tokio::task::JoinHandle<()>>,
    broadcast_flag: Arc<AtomicBool>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            child: None,
            port: None,
            state: "waiting".to_string(),
            room: String::new(),
            players: Vec::new(),
            error: None,
            version: String::new(),
            index: -1,
            stop_flag: Arc::new(AtomicBool::new(false)),
            poller: None,
            broadcast_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct TeraRuntime {
    inner: Arc<Mutex<Inner>>,
    events: Arc<Mutex<Option<Arc<dyn crate::events::Events>>>>,
}

impl Default for TeraRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl TeraRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            events: Arc::new(Mutex::new(None)),
        }
    }

    pub fn bind_events(&self, events: Arc<dyn crate::events::Events>) {
        if let Ok(mut guard) = self.events.lock() {
            *guard = Some(events);
        }
    }

    fn events(&self) -> Option<Arc<dyn crate::events::Events>> {
        self.events.lock().ok().and_then(|guard| guard.clone())
    }

    fn emit_state(&self, paths: &Paths) {
        if let Some(events) = self.events() {
            events.tera_state(&self.status(paths));
        }
    }

    pub fn installed(paths: &Paths) -> bool {
        let dir = terracotta_dir(paths);
        REQUIRED_FILES.iter().all(|name| dir.join(name).is_file())
    }

    pub fn status(&self, paths: &Paths) -> TeraStatus {
        status_of(&self.inner, paths)
    }

    /// 启动 → 等端口文件 → /meta → 开始轮询。
    pub async fn launch(&self, client: &reqwest::Client, paths: &Paths) -> Result<u16> {
        if let Ok(guard) = self.inner.lock() {
            if let Some(port) = guard.port {
                return Ok(port);
            }
        }
        if !Self::installed(paths) {
            return Err(AppError::Other("请先安装陶瓦联机".to_string()));
        }
        let dir = terracotta_dir(paths);
        std::fs::create_dir_all(&dir)?;
        let file = port_file(paths, std::process::id());
        let _ = std::fs::remove_file(&file);

        let mut child = std::process::Command::new(exe_path(paths))
            .arg("--hmcl")
            .arg(&file)
            .current_dir(&dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| AppError::Other(format!("启动陶瓦联机失败：{err}")))?;
        tracing::info!(exe = %exe_path(paths).display(), port_file = %file.display(), "已启动陶瓦 --hmcl");
        self.pump_logs(&mut child);

        let port = match wait_port_file(&file, PORT_WAIT_MS).await {
            Ok(port) => port,
            Err(err) => {
                let _ = child.kill();
                return Err(err);
            }
        };
        tracing::info!(port, "陶瓦端口文件已就绪");

        let deadline = Instant::now() + Duration::from_millis(META_WAIT_MS);
        loop {
            match http_get_json(client, port, "/meta").await {
                Ok(value) => {
                    let version = value.get("version").and_then(|i| i.as_str()).unwrap_or("");
                    let easytier = value
                        .get("easytier_version")
                        .and_then(|i| i.as_str())
                        .unwrap_or("");
                    tracing::info!(%version, easytier_version = %easytier, "陶瓦 /meta 就绪");
                    break;
                }
                Err(err) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        return Err(AppError::Other(format!(
                            "陶瓦端口 {port} 已就绪但 /meta 没响应：{err}"
                        )));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }

        let stop_flag = Arc::new(AtomicBool::new(false));
        {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| AppError::Other("陶瓦状态锁已损坏".to_string()))?;
            guard.child = Some(child);
            guard.port = Some(port);
            guard.state = "waiting".to_string();
            guard.index = -1;
            guard.error = None;
            guard.stop_flag = stop_flag.clone();
        }
        self.start_polling(client.clone(), paths.clone(), port, stop_flag);
        Ok(port)
    }

    fn pump_logs(&self, child: &mut std::process::Child) {
        let inner = self.inner.clone();
        let events_holder = self.events.clone();
        let streams = [
            child.stdout.take().map(StreamKind::Out),
            child.stderr.take().map(StreamKind::Err),
        ];
        for stream in streams {
            let Some(stream) = stream else { continue };
            let inner = inner.clone();
            let events_holder = events_holder.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                let lines: Box<dyn BufRead + Send> = match stream {
                    StreamKind::Out(stream) => Box::new(std::io::BufReader::new(stream)),
                    StreamKind::Err(stream) => Box::new(std::io::BufReader::new(stream)),
                };
                for line in lines.lines().map_while(|line| line.ok()) {
                    tracing::info!(target: "terracotta", "{line}");
                    if let Some(events) = events_holder.lock().ok().and_then(|g| g.clone()) {
                        events.tera_log(&line);
                    }
                    if let Some(player) = parse_log_player(&line) {
                        if let Ok(mut guard) = inner.lock() {
                            if !guard.players.iter().any(|item| item.name == player.name) {
                                guard.players.push(player);
                            }
                        }
                    }
                }
            });
        }
    }

    fn start_polling(
        &self,
        client: reqwest::Client,
        paths: Paths,
        port: u16,
        stop_flag: Arc<AtomicBool>,
    ) {
        let inner = self.inner.clone();
        let events_holder = self.events.clone();
        let handle = tokio::spawn(async move {
            while !stop_flag.load(Ordering::SeqCst) {
                match http_get_json(&client, port, "/state").await {
                    Ok(value) => {
                        let changed = inner
                            .lock()
                            .map(|mut guard| apply_state(&mut guard, &value))
                            .unwrap_or(false);
                        if changed {
                            if let Some(events) =
                                events_holder.lock().ok().and_then(|guard| guard.clone())
                            {
                                events.tera_state(&status_of(&inner, &paths));
                            }
                        }
                    }
                    Err(err) => tracing::debug!(%err, "陶瓦 /state 轮询失败（下次再试）"),
                }
                tokio::time::sleep(Duration::from_millis(STATE_POLL_MS)).await;
            }
        });
        if let Ok(mut guard) = self.inner.lock() {
            guard.poller = Some(handle);
        }
    }

    /// 主机模式：/state/scanning（server 模式额外替服务器广播）。
    pub async fn host(
        &self,
        client: &reqwest::Client,
        paths: &Paths,
        player: &str,
        server_port: Option<u16>,
    ) -> Result<TeraStatus> {
        let port = self.launch(client, paths).await?;
        let route = format!(
            "/state/scanning?player={}{}",
            encode(player),
            public_node_query()
        );
        if let Err(err) = http_get_status(client, port, &route).await {
            self.set_error(format!("请求主机模式失败：{err}"));
            return Err(AppError::Other(format!("请求主机模式失败：{err}")));
        }
        if let Ok(mut guard) = self.inner.lock() {
            guard.state = "host-scanning".to_string();
            guard.error = None;
        }
        if let Some(server_port) = server_port {
            self.start_broadcast(server_port);
        }
        tracing::info!("已请求陶瓦开始扫描局域网端口（主机模式）");
        let status = self.status(paths);
        self.emit_state(paths);
        Ok(status)
    }

    /// 加入模式：/state/guesting?room=…
    pub async fn join(
        &self,
        client: &reqwest::Client,
        paths: &Paths,
        code: &str,
        player: &str,
    ) -> Result<TeraStatus> {
        let code = validate_room_code(code)?;
        let port = self.launch(client, paths).await?;
        let route = format!(
            "/state/guesting?room={}&player={}{}",
            encode(&code),
            encode(player),
            public_node_query()
        );
        if let Err(err) = http_get_status(client, port, &route).await {
            self.set_error(format!("加入房间失败：{err}"));
            return Err(AppError::Other(format!("加入房间失败：{err}")));
        }
        if let Ok(mut guard) = self.inner.lock() {
            guard.state = "guest-connecting".to_string();
            guard.room = code.clone();
            guard.error = None;
        }
        tracing::info!(%code, "已请求加入陶瓦房间");
        let status = self.status(paths);
        self.emit_state(paths);
        Ok(status)
    }

    /// 复位回 waiting。
    pub async fn reset(&self, client: &reqwest::Client, paths: &Paths) -> Result<TeraStatus> {
        let port = self.inner.lock().ok().and_then(|guard| guard.port);
        if let Some(port) = port {
            let _ = http_get_status(client, port, "/state/ide").await;
        }
        self.stop_broadcast();
        if let Ok(mut guard) = self.inner.lock() {
            guard.state = "waiting".to_string();
            guard.index = -1;
            guard.room.clear();
            guard.players.clear();
            guard.error = None;
        }
        let status = self.status(paths);
        self.emit_state(paths);
        Ok(status)
    }

    /// 停止：/panic → 等端口关闭 → 按端口反查 PID + taskkill /T /F。
    pub async fn stop(&self, client: &reqwest::Client, paths: &Paths) -> Result<()> {
        let port = self.inner.lock().ok().and_then(|guard| guard.port);
        self.stop_broadcast();
        if let Some(port) = port {
            let _ = http_get_status(client, port, "/panic?peaceful=true").await;
            let deadline = Instant::now() + Duration::from_millis(EXIT_GRACE_MS);
            while Instant::now() < deadline && !pids_listening_on(port).is_empty() {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let mut targets = pids_listening_on(port);
            if let Ok(mut guard) = self.inner.lock() {
                if let Some(mut child) = guard.child.take() {
                    let pid = child.id();
                    if !targets.contains(&pid) {
                        targets.push(pid);
                    }
                    let _ = child.kill();
                }
            }
            for pid in targets {
                if pid != 0 && pid != std::process::id() {
                    kill_tree(pid);
                }
            }
            for pid in pids_of("easytier-core.exe") {
                kill_tree(pid);
            }
        }
        self.reset_runtime(paths);
        tracing::info!("陶瓦联机已停止");
        Ok(())
    }

    fn reset_runtime(&self, paths: &Paths) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.stop_flag.store(true, Ordering::SeqCst);
            if let Some(handle) = guard.poller.take() {
                handle.abort();
            }
            guard.child = None;
            guard.port = None;
            guard.state = "waiting".to_string();
            guard.index = -1;
            guard.room.clear();
            guard.players.clear();
            guard.error = None;
        }
        let _ = std::fs::remove_file(port_file(paths, std::process::id()));
        self.emit_state(paths);
    }

    fn set_error(&self, message: impl Into<String>) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.error = Some(message.into());
            guard.state = "exception".to_string();
        }
    }

    /// 专用服务器模式：替服务器发局域网广播。
    pub fn start_broadcast(&self, server_port: u16) {
        self.stop_broadcast();
        let flag = self.inner.lock().map(|guard| guard.broadcast_flag.clone()).ok();
        let Some(flag) = flag else { return };
        flag.store(true, Ordering::SeqCst);
        let payload = broadcast_payload(server_port);
        std::thread::spawn(move || {
            let mut failures = 0u32;
            let mut announced = false;
            while flag.load(Ordering::SeqCst) {
                match UdpSocket::bind("0.0.0.0:0") {
                    Ok(socket) => {
                        let _ = socket.set_broadcast(true);
                        let _ = socket.set_multicast_ttl_v4(1);
                        match socket.send_to(payload.as_bytes(), (LAN_GROUP, LAN_PORT)) {
                            Ok(_) => {
                                failures = 0;
                                if !announced {
                                    announced = true;
                                    tracing::info!(
                                        target: "terracotta",
                                        "局域网广播 {payload} → {LAN_GROUP}:{LAN_PORT}"
                                    );
                                }
                            }
                            Err(err) => {
                                failures += 1;
                                tracing::warn!(%err, "局域网广播发送失败");
                                if failures >= BROADCAST_MAX_FAILURES {
                                    break;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%err, "局域网广播 socket 创建失败");
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(BROADCAST_MS));
            }
        });
    }

    pub fn stop_broadcast(&self) {
        if let Ok(guard) = self.inner.lock() {
            guard.broadcast_flag.store(false, Ordering::SeqCst);
        }
    }

    /// 安装：GitHub → Gitee，校验 gzip + 必需文件（tar 解压）。
    pub async fn install(
        &self,
        client: &reqwest::Client,
        paths: &Paths,
        task: &TaskHandle,
    ) -> Result<String> {
        let dir = terracotta_dir(paths);
        std::fs::create_dir_all(&dir)?;
        let archive = paths.cache.join(ARCHIVE_NAME);
        let mut last_error = String::new();
        for url in mirrors() {
            task.progress_forced(Phase::Downloading, "正在下载陶瓦联机", 0, 0);
            match download(client, &url, &archive, task).await {
                Ok(()) => {
                    task.progress_forced(Phase::Verifying, "正在校验陶瓦安装包", 0, 0);
                    match verify_archive(&archive, None) {
                        Ok(size) => {
                            tracing::info!(size, "陶瓦安装包已下载");
                            task.progress_forced(Phase::Extracting, "正在解压陶瓦联机", 0, 0);
                            extract_archive(&archive, &dir)?;
                            let _ = std::fs::remove_file(&archive);
                            if let Ok(mut guard) = self.inner.lock() {
                                guard.version = RELEASE.trim_start_matches('v').to_string();
                                guard.error = None;
                            }
                            task.progress_forced(Phase::Finished, "陶瓦联机安装完成", 1, 1);
                            return Ok(RELEASE.trim_start_matches('v').to_string());
                        }
                        Err(err) => last_error = err.to_string(),
                    }
                }
                Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                Err(err) => last_error = err.to_string(),
            }
            tracing::warn!(%last_error, "陶瓦安装包下载失败，换下一个镜像");
        }
        self.set_error(last_error.clone());
        Err(AppError::Net(format!(
            "陶瓦联机安装失败（已尝试 GitHub 与 Gitee）：{last_error}"
        )))
    }
}

/// 除 waiting/idle/exception 之外都算「正在进行中」（hosting/guesting 给前端用）。
pub fn is_active_state(state: &str) -> bool {
    !matches!(state, "" | "waiting" | "idle" | "exception")
}

fn status_of(inner: &Arc<Mutex<Inner>>, paths: &Paths) -> TeraStatus {
    let installed = TeraRuntime::installed(paths);
    match inner.lock() {
        Ok(guard) => TeraStatus {
            installed,
            version: if guard.version.is_empty() && installed {
                RELEASE.trim_start_matches('v').to_string()
            } else {
                guard.version.clone()
            },
            hosting: is_active_state(&guard.state),
            players: guard.players.clone(),
            room_code: guard.room.clone(),
            error: guard.error.clone(),
            state: guard.state.clone(),
        },
        Err(_) => TeraStatus {
            installed,
            version: String::new(),
            hosting: false,
            players: Vec::new(),
            room_code: String::new(),
            error: Some("陶瓦状态锁已损坏".to_string()),
            state: "waiting".to_string(),
        },
    }
}

/// 应用 /state：index 必须严格递增，返回是否有变化。
pub(crate) fn apply_state(inner: &mut Inner, value: &serde_json::Value) -> bool {
    let index = value.get("index").and_then(|i| i.as_i64()).unwrap_or(-1);
    if index <= inner.index {
        return false;
    }
    let Some(next) = value.get("state").and_then(|i| i.as_str()) else {
        return false;
    };
    inner.index = index;
    inner.state = next.to_string();
    inner.room = value.get("room").and_then(|i| i.as_str()).unwrap_or("").to_string();
    inner.players = value
        .get("profiles")
        .and_then(|i| i.as_array())
        .map(|profiles| {
            profiles
                .iter()
                .map(|profile| TeraPlayer {
                    name: profile.get("name").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                    address: profile
                        .get("machine_id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .filter(|player| !player.name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if next == "exception" {
        inner.broadcast_flag.store(false, Ordering::SeqCst);
    }
    true
}

enum StreamKind {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

pub async fn wait_port_file(file: &Path, timeout_ms: u64) -> Result<u16> {
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(file) {
            if let Some(port) = parse_port_file(&text) {
                return Ok(port);
            }
        }
        if started.elapsed().as_millis() as u64 >= timeout_ms {
            return Err(AppError::Other(format!(
                "陶瓦没有在 {timeout_ms}ms 内写出端口文件（{}）",
                file.display()
            )));
        }
        tokio::time::sleep(Duration::from_millis(PORT_POLL_MS)).await;
    }
}

pub fn parse_port_file(text: &str) -> Option<u16> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let port = value.get("port").and_then(|i| i.as_u64())?;
    if (1..=65535).contains(&port) {
        Some(port as u16)
    } else {
        None
    }
}

pub fn public_node_query() -> String {
    PUBLIC_NODES
        .iter()
        .map(|node| format!("&public_nodes={}", encode(node)))
        .collect()
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub fn broadcast_payload(port: u16) -> String {
    format!("[MOTD]{BROADCAST_MOTD}[/MOTD][AD]{port}[/AD]")
}

pub fn parse_broadcast(payload: &str) -> Option<(String, u16)> {
    let motd_start = payload.find("[MOTD]")? + "[MOTD]".len();
    let motd_end = payload[motd_start..].find("[/MOTD]")? + motd_start;
    let ad_start = payload.find("[AD]")? + "[AD]".len();
    let ad_end = payload[ad_start..].find("[/AD]")? + ad_start;
    let port: u16 = payload[ad_start..ad_end].trim().parse().ok()?;
    Some((payload[motd_start..motd_end].to_string(), port))
}

pub fn validate_room_code(code: &str) -> Result<String> {
    let trimmed = code.trim().to_uppercase();
    let body = trimmed.strip_prefix("U/").unwrap_or(&trimmed).to_string();
    let groups: Vec<&str> = body.split('-').collect();
    if groups.len() != 4 || groups.iter().any(|group| group.len() != 4) {
        return Err(AppError::Invalid(
            "联机码格式不对：应为 U/XXXX-XXXX-XXXX-XXXX（16 位字母或数字）".to_string(),
        ));
    }
    for group in &groups {
        if !group
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && c != 'I' && c != 'O')
        {
            return Err(AppError::Invalid(
                "联机码只能包含字母或数字，且不含 I 与 O".to_string(),
            ));
        }
    }
    Ok(format!("U/{}", groups.join("-")))
}

pub fn parse_log_player(line: &str) -> Option<TeraPlayer> {
    let lowered = line.to_lowercase();
    let marker = if lowered.contains(" joined") {
        " joined"
    } else if lowered.contains("加入") {
        "加入"
    } else {
        return None;
    };
    let head = line.split(marker).next()?.trim();
    let name = head
        .rsplit(|c: char| c == ']' || c == ':' || c == ' ')
        .find(|part| !part.trim().is_empty())?
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    let address = if let Some(start) = line.rfind('(') {
        line[start + 1..].split(')').next().unwrap_or("").to_string()
    } else {
        String::new()
    };
    Some(TeraPlayer { name, address })
}

pub fn parse_netstat_pids(output: &str, port: u16) -> Vec<u32> {
    let suffix = format!(":{port}");
    let mut out: Vec<u32> = Vec::new();
    for line in output.lines() {
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 5 {
            continue;
        }
        if !columns[1].ends_with(&suffix) {
            continue;
        }
        if !columns[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        if let Ok(pid) = columns[4].parse::<u32>() {
            if !out.contains(&pid) {
                out.push(pid);
            }
        }
    }
    out
}

pub fn parse_tasklist_pids(output: &str, image: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for line in output.lines() {
        if !line.to_lowercase().contains(&image.to_lowercase()) {
            continue;
        }
        for part in line.split(',') {
            let cleaned = part.trim().trim_matches('"');
            if let Ok(pid) = cleaned.parse::<u32>() {
                if pid > 0 && !out.contains(&pid) {
                    out.push(pid);
                    break;
                }
            }
        }
    }
    out
}

pub fn pids_listening_on(port: u16) -> Vec<u32> {
    let mut cmd = std::process::Command::new("netstat");
    cmd.args(["-ano", "-p", "TCP"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    util::hidden(&mut cmd);
    match cmd.output() {
        Ok(output) => parse_netstat_pids(&String::from_utf8_lossy(&output.stdout), port),
        Err(_) => Vec::new(),
    }
}

pub fn pids_of(image: &str) -> Vec<u32> {
    let filter = format!("IMAGENAME eq {image}");
    let mut cmd = std::process::Command::new("tasklist");
    cmd.args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    util::hidden(&mut cmd);
    match cmd.output() {
        Ok(output) => parse_tasklist_pids(&String::from_utf8_lossy(&output.stdout), image),
        Err(_) => Vec::new(),
    }
}

pub fn kill_tree(pid: u32) {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    util::hidden(&mut cmd);
    let _ = cmd.output();
}

async fn http_get_json(
    client: &reqwest::Client,
    port: u16,
    route: &str,
) -> Result<serde_json::Value> {
    let url = format!("http://127.0.0.1:{port}{route}");
    let response = client
        .get(&url)
        .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
        .send()
        .await
        .map_err(|err| AppError::Net(format!("{route} 请求失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!(
            "{route} -> HTTP {}",
            response.status().as_u16()
        )));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| AppError::Net(format!("{route} 返回不是 JSON：{err}")))
}

async fn http_get_status(client: &reqwest::Client, port: u16, route: &str) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}{route}");
    let response = client
        .get(&url)
        .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
        .send()
        .await
        .map_err(|err| AppError::Net(format!("{route} 请求失败：{err}")))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        // 陶瓦会把人话原因放在响应体里（码无效 / 对方没开房 / 版本不一致）
        let body = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect::<String>();
        let body = body.trim().to_string();
        return Err(AppError::Net(if body.is_empty() {
            format!("{route} -> HTTP {status}")
        } else {
            format!("HTTP {status}：{body}")
        }));
    }
    Ok(())
}

pub fn verify_archive(path: &Path, expected_size: Option<u64>) -> Result<u64> {
    let metadata = std::fs::metadata(path)
        .map_err(|err| AppError::Other(format!("陶瓦安装包不存在：{err}")))?;
    let size = metadata.len();
    if size == 0 {
        return Err(AppError::Other("陶瓦安装包是空文件".to_string()));
    }
    if let Some(expected) = expected_size {
        if expected != size {
            return Err(AppError::Other(format!(
                "陶瓦安装包大小不匹配：期望 {expected} 字节，实际 {size} 字节"
            )));
        }
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() < 3 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Err(AppError::Other(
            "陶瓦安装包不是 gzip 归档，可能下载到了错误页面".to_string(),
        ));
    }
    Ok(size)
}

pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut cmd = std::process::Command::new("tar");
    cmd.args([
        "-xzf",
        &archive.display().to_string(),
        "-C",
        &dest.display().to_string(),
    ])
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());
    util::hidden(&mut cmd);
    let output = cmd
        .output()
        .map_err(|err| AppError::Other(format!("调用 tar 解压失败：{err}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Other(format!(
            "解压陶瓦安装包失败：{}",
            stderr.chars().take(200).collect::<String>()
        )));
    }
    normalize_extracted_exe(dest)?;
    for name in REQUIRED_FILES {
        if !dest.join(name).is_file() {
            return Err(AppError::Other(format!(
                "解压后缺少 {name}，安装包可能不完整"
            )));
        }
    }
    Ok(())
}

pub fn normalize_extracted_exe(dest: &Path) -> Result<()> {
    let target = dest.join(EXE_NAME);
    if target.is_file() {
        return Ok(());
    }
    let entries = std::fs::read_dir(dest)
        .map_err(|err| AppError::Other(format!("读取解压目录失败：{err}")))?;
    let mut fallback: Option<PathBuf> = None;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !name.ends_with(".exe") {
            continue;
        }
        if name.contains("terracotta") {
            std::fs::rename(&path, &target)
                .map_err(|err| AppError::Other(format!("重命名陶瓦可执行文件失败：{err}")))?;
            tracing::info!(from = %name, "陶瓦可执行文件已改为 {EXE_NAME}");
            return Ok(());
        }
        fallback = Some(path);
    }
    if let Some(path) = fallback {
        std::fs::rename(path, &target)
            .map_err(|err| AppError::Other(format!("重命名陶瓦可执行文件失败：{err}")))?;
        return Ok(());
    }
    Err(AppError::Other(
        "解压后没找到任何 .exe，安装包可能不完整".to_string(),
    ))
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    task: &TaskHandle,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut response = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(|err| AppError::Net(format!("下载陶瓦联机失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!("HTTP {}", response.status().as_u16())));
    }
    let total = response.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(dest)?;
    use std::io::Write;
    let mut written = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| AppError::Net(format!("读取陶瓦安装包失败：{err}")))?
    {
        task.check_cancelled()?;
        file.write_all(&chunk)?;
        written += chunk.len() as u64;
        task.progress(Phase::Downloading, "正在下载陶瓦联机", written, total);
    }
    file.flush()?;
    if written == 0 {
        return Err(AppError::Net("陶瓦安装包响应为空".to_string()));
    }
    let sha256 = util::sha256_file(dest).unwrap_or_default();
    tracing::info!(bytes = written, sha256 = %sha256, "陶瓦安装包已落盘");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("gugu-tera-{tag}-{}", util::unique_suffix()));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    #[test]
    fn room_code_validation() {
        assert_eq!(
            validate_room_code("u/abcd-efgh-jklm-npqr").unwrap(),
            "U/ABCD-EFGH-JKLM-NPQR"
        );
        assert_eq!(
            validate_room_code("ABCD-EFGH-JKLM-NPQR").unwrap(),
            "U/ABCD-EFGH-JKLM-NPQR"
        );
        assert!(validate_room_code("ABCD-EFGH").is_err());
        assert!(validate_room_code("ABCD-EFGH-JKLM-NPQI").is_err());
        assert!(validate_room_code("ABCD-EFGH-JKLM-NPX!").is_err());
        assert!(validate_room_code("").is_err());
    }

    #[test]
    fn port_file_parsing() {
        assert_eq!(parse_port_file(r#"{"port":54321}"#), Some(54321));
        assert_eq!(parse_port_file(r#"{"port":0}"#), None);
        assert_eq!(parse_port_file(r#"{"port":70000}"#), None);
        assert_eq!(parse_port_file("not json"), None);
        assert_eq!(parse_port_file(r#"{"other":1}"#), None);
    }

    #[test]
    fn state_apply_dedupes_by_index() {
        let mut inner = Inner::default();
        let first = serde_json::json!({
            "index": 3,
            "state": "hosting",
            "room": "ABCD-EFGH-JKLM-NPQR",
            "profiles": [{"name": "Steve", "machine_id": "m-1"}, {"name": "", "machine_id": "m-2"}]
        });
        assert!(apply_state(&mut inner, &first));
        assert_eq!(inner.state, "hosting");
        assert_eq!(inner.room, "ABCD-EFGH-JKLM-NPQR");
        assert_eq!(inner.players.len(), 1, "空名字要过滤");
        assert_eq!(inner.players[0].address, "m-1");

        assert!(!apply_state(&mut inner, &first));
        let stale = serde_json::json!({"index": 2, "state": "waiting", "room": ""});
        assert!(!apply_state(&mut inner, &stale));
        assert_eq!(inner.state, "hosting");

        let next = serde_json::json!({"index": 9, "state": "guesting", "room": "", "profiles": []});
        assert!(apply_state(&mut inner, &next));
        assert_eq!(inner.state, "guesting");
        assert!(inner.players.is_empty());

        assert!(!apply_state(&mut inner, &serde_json::json!({"index": 20})));
    }

    #[test]
    fn broadcast_payload_roundtrip() {
        let payload = broadcast_payload(25565);
        assert_eq!(payload, "[MOTD]NaiWa Server[/MOTD][AD]25565[/AD]");
        assert_eq!(
            parse_broadcast(&payload),
            Some(("NaiWa Server".to_string(), 25565))
        );
        assert_eq!(parse_broadcast("[MOTD]x[/MOTD]"), None);
        assert_eq!(parse_broadcast("[MOTD]x[/MOTD][AD]abc[/AD]"), None);
        assert_eq!(parse_broadcast(&broadcast_payload(4444)).unwrap().1, 4444);
    }

    #[test]
    fn netstat_and_tasklist_parsing() {
        let netstat = "  协议  本地地址          外部地址        状态           PID\r\n  TCP    127.0.0.1:54321        0.0.0.0:0              LISTENING       4242\r\n  TCP    127.0.0.1:54321        127.0.0.1:50000        ESTABLISHED     4242\r\n  TCP    0.0.0.0:25565          0.0.0.0:0              LISTENING       777\r\n";
        assert_eq!(parse_netstat_pids(netstat, 54321), vec![4242]);
        assert_eq!(parse_netstat_pids(netstat, 25565), vec![777]);
        assert!(parse_netstat_pids(netstat, 9999).is_empty());

        let tasklist = "\"easytier-core.exe\",\"1234\",\"Console\",\"1\",\"12,345 K\"\r\n\"easytier-core.exe\",\"5678\",\"Console\",\"1\",\"9,876 K\"\r\n";
        assert_eq!(
            parse_tasklist_pids(tasklist, "easytier-core.exe"),
            vec![1234, 5678]
        );
        assert!(parse_tasklist_pids("信息: 没有运行的任务匹配指定标准。", "x.exe").is_empty());
    }

    #[test]
    fn public_nodes_query_repeats_key() {
        let query = public_node_query();
        assert_eq!(query.matches("public_nodes=").count(), PUBLIC_NODES.len());
        assert!(query.starts_with("&public_nodes=tcp%3A%2F%2Fpublic.easytier.top%3A11010"));
    }

    #[test]
    fn log_player_sniffing() {
        let player = parse_log_player("[INFO] Steve joined (1.2.3.4:51234)").unwrap();
        assert_eq!(player.name, "Steve");
        assert_eq!(player.address, "1.2.3.4:51234");
        assert!(parse_log_player("no player here").is_none());
    }

    #[test]
    fn active_state_covers_host_and_guest_phases() {
        for state in ["host-scanning", "host-ok", "guest-connecting", "guest-starting", "guesting"] {
            assert!(is_active_state(state), "{state} 应当算进行中");
        }
        for state in ["", "waiting", "idle", "exception"] {
            assert!(!is_active_state(state), "{state} 不该算进行中");
        }
    }

    #[test]
    fn status_reports_installed_and_state() {
        let paths = temp_paths("status");
        let dir = terracotta_dir(&paths);
        std::fs::create_dir_all(&dir).unwrap();
        for name in REQUIRED_FILES {
            std::fs::write(dir.join(name), b"fake").unwrap();
        }
        let runtime = TeraRuntime::new();
        let status = runtime.status(&paths);
        assert!(status.installed);
        assert!(!status.hosting);
        assert_eq!(status.state, "waiting");
        assert_eq!(status.version, RELEASE.trim_start_matches('v'));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn extracted_exe_is_normalized_to_fixed_name() {
        let paths = temp_paths("exe");
        let dir = terracotta_dir(&paths);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("terracotta-0.4.2-windows-x86_64.exe"), b"MZ").unwrap();
        std::fs::write(dir.join(DLL_NAME), b"MZ").unwrap();
        normalize_extracted_exe(&dir).unwrap();
        assert!(dir.join(EXE_NAME).is_file());
        normalize_extracted_exe(&dir).unwrap();
        let empty = paths.root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(normalize_extracted_exe(&empty).is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn archive_verification_rejects_bad_payloads() {
        let paths = temp_paths("archive");
        let file = paths.cache.join("a.tar.gz");
        std::fs::write(&file, b"<html>404</html>").unwrap();
        assert!(verify_archive(&file, None)
            .unwrap_err()
            .to_string()
            .contains("gzip"));
        std::fs::write(&file, [0x1f, 0x8b, 0x08, 0x00]).unwrap();
        assert_eq!(verify_archive(&file, Some(4)).unwrap(), 4);
        assert!(verify_archive(&file, Some(9)).is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[tokio::test]
    async fn operations_require_install_first() {
        let paths = temp_paths("missing");
        let runtime = TeraRuntime::new();
        let client = reqwest::Client::new();
        let err = runtime
            .host(&client, &paths, "P", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("请先安装陶瓦联机"), "{err}");
        let err = runtime
            .join(&client, &paths, "ABCD-EFGH-JKLM-NPQR", "P")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("请先安装陶瓦联机"), "{err}");
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
