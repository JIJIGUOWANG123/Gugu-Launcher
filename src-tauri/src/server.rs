//! 本地开服（多人页「服务器」子页）：核心下载、目录生成、进程与日志、properties 读写。
//!
//! 元数据端点（2026-02 真机实测）：
//! - 原版 vanilla：Mojang version manifest → 版本 json 的 downloads.server.url
//! - Paper：https://fill.papermc.io/v3/projects/paper/versions/<mc>/builds （200，
//!   数组，每项 downloads["server:default"].url / .size；旧 api.papermc.io/v2 已 410）
//! - Purpur：https://api.purpurmc.org/v2/purpur/<mc>/latest → 直链 /latest/download （200）
//! - Fabric：https://meta.fabricmc.net/v2/versions/loader/<mc>/<loader>/<installer>/server/jar
//!   （200，直接是可运行的启动 jar）；版本号来自 /v2/versions/loader/<mc> 与
//!   /v2/versions/installer
//! - Forge：https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json
//!   → maven 的 forge-<mc>-<ver>-installer.jar
//! - NeoForge：maven.neoforged.net 的 maven-metadata.xml（本机网络会偶发连不上，
//!   失败时返回空列表并记 warn，不影响其它核心）
//! - FastMirror 只是下载站，没有公开 API：设置里给「服务端镜像」留自定义根地址，
//!   UI 里给「打开 FastMirror 下载页」按钮，这里不硬编码任何未公开接口。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::downloader::Fetcher;
use crate::error::{AppError, Result};
use crate::events::Phase;
use crate::paths::Paths;
use crate::sources::Origin;
use crate::tasks::TaskHandle;
use crate::util;

/// 服务端目录名（<数据根>/servers/<名字>）。
pub const SERVERS_DIR: &str = "servers";
/// 元数据文件名。
pub const META_FILE: &str = "server.json";
/// 服务端默认端口。
pub const DEFAULT_PORT: u16 = 25565;
/// 停止时等多久才强杀。
pub const STOP_TIMEOUT_SECS: u64 = 10;
/// 端口上限/下限校验。
pub const PORT_MIN: u16 = 1;

/// server_cores 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCore {
    pub id: String,
    pub name: String,
    pub latest_version: Option<String>,
    pub download_page: String,
}

/// server_versions 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerVersion {
    pub core: String,
    pub version: String,
    pub build: Option<String>,
    pub size_bytes: Option<u64>,
    pub url: String,
}

/// server.json：一个本地服务器实例的元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMeta {
    pub id: i64,
    pub name: String,
    pub core: String,
    pub mc_version: String,
    pub port: u16,
    pub memory_mb: u32,
    /// 核心 jar 文件名（相对目录）
    pub jar: String,
    pub created_at: u64,
}

/// 运行时状态（server_list 的 state 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerState {
    Stopped,
    Starting,
    Running,
    Stopping,
}

/// server_list 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    pub id: i64,
    pub name: String,
    pub core: String,
    pub mc_version: String,
    pub port: u16,
    pub memory_mb: u32,
    pub state: ServerState,
    pub players: Option<u32>,
    pub dir: String,
}

/// 服务端目录布局。
#[derive(Debug, Clone)]
pub struct ServerLayout {
    pub dir: PathBuf,
    pub jar: PathBuf,
    pub eula: PathBuf,
    pub properties: PathBuf,
    pub start_bat: PathBuf,
    pub meta: PathBuf,
}

pub fn servers_root(paths: &Paths) -> PathBuf {
    paths.root.join(SERVERS_DIR)
}

pub fn layout(paths: &Paths, name: &str) -> ServerLayout {
    let dir = servers_root(paths).join(util::sanitize_file_part(name));
    ServerLayout {
        jar: dir.join(meta_jar_placeholder()),
        eula: dir.join("eula.txt"),
        properties: dir.join("server.properties"),
        start_bat: dir.join("start.bat"),
        meta: dir.join(META_FILE),
        dir,
    }
}

/// 目录布局里的 jar 路径占位（真实文件名从 server.json 读）。
fn meta_jar_placeholder() -> &'static str {
    "server.jar"
}

// ------------------------------------------------------------------ 核心表

pub const CORE_IDS: [&str; 6] = ["vanilla", "paper", "purpur", "fabric", "forge", "neoforge"];
pub const FAST_MIRROR_PAGE: &str = "https://www.fastmirror.net/#/download";

/// server_cores：6 个核心的静态表（latestVersion 由版本查询补）。
pub fn cores_table() -> Vec<ServerCore> {
    let make = |id: &str, name: &str, page: &str| ServerCore {
        id: id.to_string(),
        name: name.to_string(),
        latest_version: None,
        download_page: page.to_string(),
    };
    vec![
        make("vanilla", "原版 Vanilla", "https://www.minecraft.net/download/server"),
        make("paper", "Paper", "https://papermc.io/downloads/paper"),
        make("purpur", "Purpur", "https://purpurmc.org/downloads"),
        make("fabric", "Fabric", "https://fabricmc.net/use/server/"),
        make("forge", "Forge", "https://files.minecraftforge.net/net/minecraftforge/forge/"),
        make("neoforge", "NeoForge", "https://neoforged.net/"),
    ]
}

/// 核心的显示名（日志/错误用）。
pub fn core_name(core: &str) -> String {
    cores_table()
        .into_iter()
        .find(|item| item.id == core)
        .map(|item| item.name)
        .unwrap_or_else(|| core.to_string())
}

// ------------------------------------------------------------------ 端点

pub fn paper_builds_url(mc_version: &str) -> String {
    format!("https://fill.papermc.io/v3/projects/paper/versions/{mc_version}/builds")
}

pub fn purpur_latest_url(mc_version: &str) -> String {
    format!("https://api.purpurmc.org/v2/purpur/{mc_version}/latest")
}

pub fn purpur_download_url(mc_version: &str) -> String {
    format!("https://api.purpurmc.org/v2/purpur/{mc_version}/latest/download")
}

pub fn fabric_loader_url(mc_version: &str) -> String {
    format!("https://meta.fabricmc.net/v2/versions/loader/{mc_version}")
}

pub fn fabric_installer_url() -> String {
    "https://meta.fabricmc.net/v2/versions/installer".to_string()
}

pub fn fabric_server_jar_url(mc_version: &str, loader: &str, installer: &str) -> String {
    format!("https://meta.fabricmc.net/v2/versions/loader/{mc_version}/{loader}/{installer}/server/jar")
}

pub fn forge_promotions_url() -> String {
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json".to_string()
}

pub fn forge_installer_url(mc_version: &str, version: &str) -> String {
    format!(
        "https://maven.minecraftforge.net/net/minecraftforge/forge/{mc_version}-{version}/forge-{mc_version}-{version}-installer.jar"
    )
}

pub fn neoforge_metadata_url() -> String {
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml".to_string()
}

// ------------------------------------------------------------------ 解析纯函数

/// Paper fill v3：取最新 STABLE 的 server:default。
pub fn parse_paper_builds(json: &str, mc_version: &str) -> Option<ServerVersion> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let items = value.as_array()?;
    let mut best: Option<(i64, &serde_json::Value)> = None;
    for item in items {
        let channel = item.get("channel").and_then(|v| v.as_str()).unwrap_or("");
        if channel != "STABLE" {
            continue;
        }
        let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        if best.map(|(current, _)| id > current).unwrap_or(true) {
            best = Some((id, item));
        }
    }
    let (id, item) = best?;
    let download = item
        .pointer("/downloads/server:default")
        .or_else(|| item.pointer("/downloads/server"))
        .or_else(|| item.pointer("/downloads/application"))?;
    let url = download.get("url").and_then(|v| v.as_str())?.to_string();
    Some(ServerVersion {
        core: "paper".to_string(),
        version: mc_version.to_string(),
        build: Some(id.to_string()),
        size_bytes: download.get("size").and_then(|v| v.as_u64()),
        url,
    })
}

/// Purpur：latest 的 build 号 + 直链。
pub fn parse_purpur_latest(json: &str, mc_version: &str) -> Option<ServerVersion> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let build = value
        .get("build")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    Some(ServerVersion {
        core: "purpur".to_string(),
        version: mc_version.to_string(),
        build,
        size_bytes: None,
        url: purpur_download_url(mc_version),
    })
}

/// Fabric：取第一个稳定 loader 与第一个稳定 installer，拼 server jar 直链。
pub fn parse_fabric_versions(loaders: &str, installers: &str, mc_version: &str) -> Option<ServerVersion> {
    let loader_json: serde_json::Value = serde_json::from_str(loaders).ok()?;
    let loader = loader_json
        .as_array()?
        .iter()
        .find(|item| item.pointer("/loader/stable").and_then(|v| v.as_bool()).unwrap_or(false))
        .or_else(|| loader_json.as_array().and_then(|items| items.first()))?;
    let loader_version = loader.pointer("/loader/version").and_then(|v| v.as_str())?;
    let installer_json: serde_json::Value = serde_json::from_str(installers).ok()?;
    let installer = installer_json
        .as_array()?
        .iter()
        .find(|item| item.get("stable").and_then(|v| v.as_bool()).unwrap_or(false))
        .or_else(|| installer_json.as_array().and_then(|items| items.first()))?;
    let installer_version = installer.get("version").and_then(|v| v.as_str())?;
    Some(ServerVersion {
        core: "fabric".to_string(),
        version: mc_version.to_string(),
        build: Some(format!("loader {loader_version}")),
        size_bytes: None,
        url: fabric_server_jar_url(mc_version, loader_version, installer_version),
    })
}

/// Forge：promotions_slim.json 里 <mc>-latest / <mc>-recommended。
pub fn parse_forge_promo(json: &str, mc_version: &str) -> Option<ServerVersion> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let promos = value.get("promos")?.as_object()?;
    let latest = promos
        .get(&format!("{mc_version}-recommended"))
        .or_else(|| promos.get(&format!("{mc_version}-latest")))
        .and_then(|v| v.as_str())?;
    Some(ServerVersion {
        core: "forge".to_string(),
        version: mc_version.to_string(),
        build: Some(latest.to_string()),
        size_bytes: None,
        url: forge_installer_url(mc_version, latest),
    })
}

/// NeoForge：maven-metadata.xml 里 20.4.x 这类版本号（前缀是 <mc 去掉 1.> . <patch>）。
pub fn parse_neoforge_metadata(xml: &str, mc_version: &str) -> Option<ServerVersion> {
    let prefix = if let Some(rest) = mc_version.strip_prefix("1.") {
        rest.replace('.', ".")
    } else {
        mc_version.to_string()
    };
    let mut candidates: Vec<String> = Vec::new();
    for chunk in xml.split("<version>").skip(1) {
        let Some(end) = chunk.find("</version>") else {
            continue;
        };
        let version = chunk[..end].trim();
        if version.is_empty() {
            continue;
        }
        if prefix.is_empty() || version.starts_with(&prefix) {
            candidates.push(version.to_string());
        }
    }
    candidates.sort();
    let version = candidates.last()?.clone();
    Some(ServerVersion {
        core: "neoforge".to_string(),
        version: mc_version.to_string(),
        build: Some(version.clone()),
        size_bytes: None,
        url: format!(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"
        ),
    })
}

/// Mojang 版本清单里挑该版本的 json 地址（vanilla 服务端 jar 在版本 json 里）。
pub fn parse_manifest_json_url(manifest: &str, mc_version: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(manifest).ok()?;
    value
        .get("versions")?
        .as_array()?
        .iter()
        .find(|item| item.get("id").and_then(|v| v.as_str()) == Some(mc_version))
        .and_then(|item| item.get("url"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

/// 版本 json 里的 downloads.server.url。
pub fn parse_version_server_url(version_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(version_json).ok()?;
    value
        .pointer("/downloads/server/url")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

// ------------------------------------------------------------------ properties

/// 一行 properties（注释/空行原样保留）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyLine {
    Raw(String),
    Entry { key: String, value: String },
}

/// 有序的 properties：读写都保持顺序与注释。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PropertySet {
    pub lines: Vec<PropertyLine>,
}

impl PropertySet {
    pub fn parse(text: &str) -> Self {
        let mut lines = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
                lines.push(PropertyLine::Raw(line.to_string()));
                continue;
            }
            match line.split_once('=') {
                Some((key, value)) => lines.push(PropertyLine::Entry {
                    key: key.trim().to_string(),
                    value: value.trim().to_string(),
                }),
                None => lines.push(PropertyLine::Raw(line.to_string())),
            }
        }
        Self { lines }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().find_map(|line| match line {
            PropertyLine::Entry { key: current, value } if current == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// 覆盖已有键（原位），新键追加到末尾。
    pub fn patch(&mut self, patch: &BTreeMap<String, String>) {
        for (key, value) in patch {
            let mut replaced = false;
            for line in self.lines.iter_mut() {
                if let PropertyLine::Entry { key: current, value: current_value } = line {
                    if current == key {
                        *current_value = value.clone();
                        replaced = true;
                    }
                }
            }
            if !replaced {
                self.lines.push(PropertyLine::Entry {
                    key: key.clone(),
                    value: value.clone(),
                });
            }
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                PropertyLine::Raw(text) => out.push_str(text),
                PropertyLine::Entry { key, value } => {
                    out.push_str(key);
                    out.push('=');
                    out.push_str(value);
                }
            }
            out.push_str("\r\n");
        }
        out
    }
}

/// 新建服务器时的 server.properties（顺序固定，便于排查）。
pub fn default_properties(motd: &str, port: u16, online_mode: bool, max_players: u32) -> String {
    let lines = [
        "#Minecraft server properties".to_string(),
        format!("motd={motd}"),
        format!("server-port={port}"),
        format!("online-mode={}", if online_mode { "true" } else { "false" }),
        format!("max-players={max_players}"),
        "gamemode=survival".to_string(),
        "difficulty=easy".to_string(),
        "pvp=true".to_string(),
        "level-name=world".to_string(),
        "allow-flight=false".to_string(),
        "enable-command-block=false".to_string(),
        "view-distance=10".to_string(),
        "simulation-distance=10".to_string(),
        "white-list=false".to_string(),
    ];
    let mut text = String::new();
    for line in lines {
        text.push_str(&line);
        text.push_str("\r\n");
    }
    text
}

pub fn eula_content(accept: bool) -> String {
    format!(
        "#By changing the setting below to TRUE you are indicating your agreement to the Minecraft EULA (https://aka.ms/MinecraftEULA).\r\neula={}\r\n",
        if accept { "true" } else { "false" }
    )
}

/// start.bat 正文（路径带空格要加引号）。
pub fn start_bat_content(java: &Path, memory_mb: u32, jar_name: &str, install_first: bool) -> String {
    let mut text = String::from("@echo off\r\nchcp 65001 >nul\r\ncd /d \"%~dp0\"\r\n");
    if install_first {
        // Forge / NeoForge：第一次启动先用 installer 生成服务端
        text.push_str(&format!(
            "if not exist libraries (\r\n  \"{}\" -jar \"{}\" --installServer\r\n)\r\n",
            java.display(),
            jar_name
        ));
        text.push_str(&format!(
            "\"{}\" -Xmx{}M -jar \"{}\" nogui\r\npause\r\n",
            java.display(),
            memory_mb,
            jar_name
        ));
    } else {
        text.push_str(&format!(
            "\"{}\" -Xmx{}M -jar \"{}\" nogui\r\npause\r\n",
            java.display(),
            memory_mb,
            jar_name
        ));
    }
    text
}

// ------------------------------------------------------------------ 进程输出解析

/// 服务端启动完成的标志行。
pub fn is_server_ready(line: &str) -> bool {
    line.contains("Done (") && line.contains("For help, type")
}

/// 玩家加入/离开（返回 (joined?, 名字)）。
pub fn parse_player_event(line: &str) -> Option<(bool, String)> {
    let marker_joined = " joined the game";
    let marker_left = " left the game";
    if let Some(index) = line.find(marker_joined) {
        let head = &line[..index];
        let name = head.rsplit(':').next().unwrap_or(head).trim();
        return Some((true, name.to_string()));
    }
    if let Some(index) = line.find(marker_left) {
        let head = &line[..index];
        let name = head.rsplit(':').next().unwrap_or(head).trim();
        return Some((false, name.to_string()));
    }
    None
}

/// netstat 里是否已有监听该端口的行。
pub fn parse_netstat_listening(output: &str, port: u16) -> bool {
    let needle = format!(":{port}");
    for line in output.lines() {
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 2 {
            continue;
        }
        let protocol = columns[0].to_ascii_uppercase();
        if protocol != "TCP" && protocol != "UDP" {
            continue;
        }
        let local = columns[1];
        let matches_port = local.ends_with(&needle)
            || local
                .rsplit_once(&needle)
                .map(|(_, tail)| tail.is_empty())
                .unwrap_or(false);
        if matches_port && (protocol == "UDP" || line.contains("LISTENING")) {
            return true;
        }
    }
    false
}

/// 端口占用检查（netstat -ano，隐藏窗口）。
pub async fn port_conflict(port: u16) -> Option<String> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netstat");
        cmd.args(["-ano", "-p", "TCP"]);
        crate::util::hidden_async(&mut cmd);
        let output = cmd.output().await.ok()?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if parse_netstat_listening(&text, port) {
            return Some(format!("端口 {port} 已被占用，请换一个端口"));
        }
        None
    }
    #[cfg(not(windows))]
    {
        let _ = port;
        None
    }
}

// ------------------------------------------------------------------ 目录与元数据

pub fn read_meta(dir: &Path) -> Option<ServerMeta> {
    let text = std::fs::read_to_string(dir.join(META_FILE)).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

pub fn write_meta(dir: &Path, meta: &ServerMeta) -> Result<()> {
    let text = serde_json::to_string_pretty(meta)?;
    std::fs::write(dir.join(META_FILE), text)?;
    Ok(())
}

/// 列出全部本地服务器（目录扫描，顺序按创建时间）。
pub fn list(paths: &Paths) -> Vec<ServerMeta> {
    let root = servers_root(paths);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<ServerMeta> = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if let Some(meta) = read_meta(&dir) {
            out.push(meta);
        }
    }
    out.sort_by_key(|meta| meta.created_at);
    out
}

/// 按 id 找目录（找不到就报可读错误）。
pub fn find(paths: &Paths, id: i64) -> Result<(PathBuf, ServerMeta)> {
    let root = servers_root(paths);
    let entries = std::fs::read_dir(&root)
        .map_err(|_| AppError::Invalid(format!("本地还没有服务器（{}）", root.display())))?;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if let Some(meta) = read_meta(&dir) {
            if meta.id == id {
                return Ok((dir, meta));
            }
        }
    }
    Err(AppError::Invalid(format!("服务器不存在：{id}")))
}

/// 名字合法性（目录名冲突也算不合法）。
pub fn validate_name(paths: &Paths, name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("服务器名字不能为空".to_string()));
    }
    let dir = servers_root(paths).join(util::sanitize_file_part(trimmed));
    if dir.exists() {
        return Err(AppError::Invalid(format!(
            "同名服务器已经存在：{}",
            util::sanitize_file_part(trimmed)
        )));
    }
    Ok(trimmed.to_string())
}

// ------------------------------------------------------------------ 运行时状态表

/// 控制台历史最多保留多少条用户指令。
pub const HISTORY_CAP: usize = 50;

/// server_console_state 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleState {
    pub running: bool,
    pub stdin_open: bool,
    pub history: Vec<String>,
}

/// 归一化用户指令：去掉前导斜杠与空白；空指令直接拒绝。
pub fn normalize_command(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_start_matches('/').trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("指令不能为空".to_string()));
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(AppError::Invalid("指令不能包含换行".to_string()));
    }
    Ok(trimmed.to_string())
}

/// 真正写进 stdin 的那一行（末尾必须带换行，否则服务器不会执行）。
pub fn command_line(command: &str) -> String {
    format!("{command}\n")
}

/// 环形缓冲：只保留最近 HISTORY_CAP 条。
pub fn push_history(history: &mut Vec<String>, command: &str) {
    history.push(command.to_string());
    if history.len() > HISTORY_CAP {
        let drop_count = history.len() - HISTORY_CAP;
        history.drain(0..drop_count);
    }
}

/// 正在运行的服务器（进程 + 写 stdin 的句柄 + 控制台历史）。
pub struct LiveServer {
    pub child: tokio::process::Child,
    pub stdin: Option<tokio::process::ChildStdin>,
    pub state: ServerState,
    pub players: u32,
    /// 最近发过的用户指令（最多 HISTORY_CAP 条）
    pub history: Vec<String>,
}

/// 进程表：AppState 里只有一个。
#[derive(Default)]
pub struct ServerStore {
    inner: tokio::sync::Mutex<std::collections::HashMap<i64, LiveServer>>,
}

impl ServerStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn state_of(&self, id: i64) -> (ServerState, Option<u32>) {
        let guard = self.inner.lock().await;
        match guard.get(&id) {
            Some(live) => (
                live.state,
                if live.state == ServerState::Running {
                    Some(live.players)
                } else {
                    None
                },
            ),
            None => (ServerState::Stopped, None),
        }
    }

    pub async fn set_state(&self, id: i64, state: ServerState) {
        let mut guard = self.inner.lock().await;
        if let Some(live) = guard.get_mut(&id) {
            live.state = state;
            if state != ServerState::Running {
                live.players = 0;
            }
        }
    }

    pub async fn bump_players(&self, id: i64, joined: bool) {
        let mut guard = self.inner.lock().await;
        if let Some(live) = guard.get_mut(&id) {
            live.players = if joined {
                live.players.saturating_add(1)
            } else {
                live.players.saturating_sub(1)
            };
        }
    }

    /// 往服务器 stdin 写一条指令（不带 shell，不经过 cmd.exe）。
    pub async fn send_command(&self, id: i64, raw: &str) -> Result<()> {
        let command = normalize_command(raw)?;
        let mut guard = self.inner.lock().await;
        let Some(live) = guard.get_mut(&id) else {
            return Err(AppError::Invalid("服务端未运行".to_string()));
        };
        if live.state != ServerState::Running {
            return Err(AppError::Invalid("服务端未运行".to_string()));
        }
        let Some(stdin) = live.stdin.as_mut() else {
            return Err(AppError::Other("服务端标准输入已关闭".to_string()));
        };
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(command_line(&command).as_bytes())
            .await
            .map_err(|err| AppError::Other(format!("写入服务器标准输入失败：{err}")))?;
        stdin
            .flush()
            .await
            .map_err(|err| AppError::Other(format!("刷新服务器标准输入失败：{err}")))?;
        push_history(&mut live.history, &command);
        tracing::info!(id, command = %command, "已向服务端发送指令");
        Ok(())
    }

    /// 控制台状态：是否运行、stdin 是否可写、最近指令。
    pub async fn console_state(&self, id: i64) -> ConsoleState {
        let guard = self.inner.lock().await;
        match guard.get(&id) {
            Some(live) => ConsoleState {
                running: live.state == ServerState::Running,
                stdin_open: live.stdin.is_some(),
                history: live.history.clone(),
            },
            None => ConsoleState {
                running: false,
                stdin_open: false,
                history: Vec::new(),
            },
        }
    }

    /// 拿 stdin 写 "stop\n"。
    pub async fn request_stop(&self, id: i64) -> bool {
        let mut guard = self.inner.lock().await;
        let Some(live) = guard.get_mut(&id) else {
            return false;
        };
        live.state = ServerState::Stopping;
        use tokio::io::AsyncWriteExt;
        if let Some(stdin) = live.stdin.as_mut() {
            let _ = stdin.write_all(b"stop\n").await;
            let _ = stdin.flush().await;
            return true;
        }
        false
    }

    /// 等进程退出（STOP_TIMEOUT_SECS 秒），超时强杀。
    pub async fn wait_exit(&self, id: i64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(STOP_TIMEOUT_SECS);
        loop {
            {
                let mut guard = self.inner.lock().await;
                let Some(live) = guard.get_mut(&id) else {
                    return true;
                };
                match live.child.try_wait() {
                    Ok(Some(_)) => {
                        guard.remove(&id);
                        return true;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(%err, id, "检查服务器进程状态失败");
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                let mut guard = self.inner.lock().await;
                if let Some(mut live) = guard.remove(&id) {
                    tracing::warn!(id, "服务器 10 秒内没有退出，强制结束进程树");
                    let _ = live.child.start_kill();
                    #[cfg(windows)]
                    if let Some(pid) = live.child.id() {
                        let mut cmd = tokio::process::Command::new("taskkill");
                        cmd.args(["/T", "/F", "/PID", &pid.to_string()]);
                        crate::util::hidden_async(&mut cmd);
                        let _ = cmd.output().await;
                    }
                }
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// 等进程自然退出（不带超时、不强杀），返回退出码。
    pub async fn wait_finished(&self, id: i64) -> Option<i32> {
        loop {
            {
                let mut guard = self.inner.lock().await;
                let Some(live) = guard.get_mut(&id) else {
                    return None;
                };
                match live.child.try_wait() {
                    Ok(Some(status)) => {
                        guard.remove(&id);
                        return status.code();
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(%err, id, "检查服务器进程状态失败");
                        guard.remove(&id);
                        return None;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// 注册新进程。
    pub async fn insert(&self, id: i64, live: LiveServer) {
        let mut guard = self.inner.lock().await;
        guard.insert(id, live);
    }

    pub async fn remove(&self, id: i64) {
        let mut guard = self.inner.lock().await;
        guard.remove(&id);
    }

    pub async fn is_running(&self, id: i64) -> bool {
        let guard = self.inner.lock().await;
        guard.contains_key(&id)
    }
}

// ------------------------------------------------------------------ 下载与创建

async fn fetch_text(client: &reqwest::Client, url: &str, what: &str) -> Result<String> {
    let response = tokio::time::timeout(std::time::Duration::from_secs(20), client.get(url).send())
        .await
        .map_err(|_| AppError::Net(format!("{what} 请求超时")))?
        .map_err(|err| AppError::Net(format!("{what} 请求失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!("{what} 返回 HTTP {}", response.status().as_u16())));
    }
    Ok(response.text().await?)
}

/// 查一个核心的可用版本（server_versions）。
pub async fn versions(
    client: &reqwest::Client,
    core: &str,
    mc_version: Option<&str>,
    manifest: Option<&str>,
) -> Vec<ServerVersion> {
    let mc = mc_version.unwrap_or("").trim().to_string();
    if mc.is_empty() {
        return Vec::new();
    }
    let result: Result<Option<ServerVersion>> = match core {
        "vanilla" => {
            let manifest = match manifest {
                Some(text) => text.to_string(),
                None => match fetch_text(
                    client,
                    "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json",
                    "版本清单",
                )
                .await
                {
                    Ok(text) => text,
                    Err(err) => {
                        tracing::warn!(%err, "读取版本清单失败");
                        return Vec::new();
                    }
                },
            };
            match parse_manifest_json_url(&manifest, &mc) {
                Some(url) => match fetch_text(client, &url, "版本 json").await {
                    Ok(json) => Ok(parse_version_server_url(&json).map(|url| ServerVersion {
                        core: "vanilla".to_string(),
                        version: mc.clone(),
                        build: None,
                        size_bytes: None,
                        url,
                    })),
                    Err(err) => Err(err),
                },
                None => Ok(None),
            }
        }
        "paper" => match fetch_text(client, &paper_builds_url(&mc), "Paper 构建列表").await {
            Ok(json) => Ok(parse_paper_builds(&json, &mc)),
            Err(err) => Err(err),
        },
        "purpur" => match fetch_text(client, &purpur_latest_url(&mc), "Purpur 构建信息").await {
            Ok(json) => Ok(parse_purpur_latest(&json, &mc)),
            Err(err) => Err(err),
        },
        "fabric" => {
            let loaders = fetch_text(client, &fabric_loader_url(&mc), "Fabric loader 列表").await;
            let installers = fetch_text(client, &fabric_installer_url(), "Fabric installer 列表").await;
            match (loaders, installers) {
                (Ok(loaders), Ok(installers)) => {
                    Ok(parse_fabric_versions(&loaders, &installers, &mc))
                }
                (Err(err), _) | (_, Err(err)) => Err(err),
            }
        }
        "forge" => match fetch_text(client, &forge_promotions_url(), "Forge 版本表").await {
            Ok(json) => Ok(parse_forge_promo(&json, &mc)),
            Err(err) => Err(err),
        },
        "neoforge" => match fetch_text(client, &neoforge_metadata_url(), "NeoForge 版本表").await {
            Ok(xml) => Ok(parse_neoforge_metadata(&xml, &mc)),
            Err(err) => Err(err),
        },
        other => Err(AppError::Invalid(format!("未知的服务端核心：{other}"))),
    };
    match result {
        Ok(Some(version)) => vec![version],
        Ok(None) => Vec::new(),
        Err(err) => {
            tracing::warn!(%err, core, mc, "查询服务端版本失败");
            Vec::new()
        }
    }
}

/// 创建服务器：下载核心 → 写 eula / server.properties / start.bat / server.json。
pub async fn create(
    client: reqwest::Client,
    paths: &Paths,
    task: &TaskHandle,
    core: &str,
    mc_version: &str,
    name: &str,
    port: u16,
    memory_mb: u32,
    accept_eula: bool,
    java: &Path,
    origins: Vec<Origin>,
) -> Result<ServerMeta> {
    if !CORE_IDS.contains(&core) {
        return Err(AppError::Invalid(format!("未知的服务端核心：{core}")));
    }
    let name = validate_name(paths, name)?;
    if !(PORT_MIN..=65535).contains(&port) {
        return Err(AppError::Invalid(format!("端口不合法：{port}")));
    }
    if let Some(message) = port_conflict(port).await {
        return Err(AppError::Invalid(message));
    }
    task.progress_forced(Phase::Downloading, "正在查询服务端下载地址", 0, 0);
    let versions = versions(&client, core, Some(mc_version), None).await;
    let picked = versions
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Net(format!("{} 没有 {mc_version} 的服务端下载地址", core_name(core))))?;

    let dir = servers_root(paths).join(util::sanitize_file_part(&name));
    std::fs::create_dir_all(&dir)?;
    let jar_name = format!("{core}-{mc_version}.jar");
    let jar_path = dir.join(&jar_name);
    let fetcher = Fetcher::new(client, origins, task.cancel_flag(), 1);
    task.progress_forced(
        Phase::Downloading,
        &format!("正在下载 {} 服务端", core_name(core)),
        0,
        0,
    );
    if let Err(err) = fetcher.fetch(&picked.url, &jar_path, None, None).await {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(err);
    }
    let size = std::fs::metadata(&jar_path).map(|meta| meta.len()).unwrap_or(0);
    tracing::info!(core, mc_version, bytes = size, url = %picked.url, "服务端核心已下载");

    task.progress_forced(Phase::Extracting, "正在生成服务器配置", 0, 0);
    std::fs::write(dir.join("eula.txt"), eula_content(accept_eula))?;
    std::fs::write(
        dir.join("server.properties"),
        default_properties(&format!("{name} · 咕咕启动器"), port, true, 20),
    )?;
    let install_first = core == "forge" || core == "neoforge";
    std::fs::write(
        dir.join("start.bat"),
        start_bat_content(java, memory_mb, &jar_name, install_first),
    )?;
    std::fs::create_dir_all(dir.join("logs"))?;

    let meta = ServerMeta {
        id: util::now_millis() as i64,
        name,
        core: core.to_string(),
        mc_version: mc_version.to_string(),
        port,
        memory_mb,
        jar: jar_name,
        created_at: util::now_unix(),
    };
    write_meta(&dir, &meta)?;
    task.progress_forced(Phase::Finished, "服务器创建完成", 1, 1);
    Ok(meta)
}

/// 起进程：命令层与自检工具共用同一套参数（-Xmx、-jar、nogui、隐藏窗口）。
pub struct SpawnedServer {
    pub child: tokio::process::Child,
    pub stdin: Option<tokio::process::ChildStdin>,
    pub stdout: Option<tokio::process::ChildStdout>,
    pub stderr: Option<tokio::process::ChildStderr>,
}

pub fn spawn(dir: &Path, meta: &ServerMeta, java: &Path) -> Result<SpawnedServer> {
    let jar = dir.join(&meta.jar);
    if !jar.is_file() {
        return Err(AppError::Invalid(format!(
            "服务端核心缺失：{}（请重新创建服务器）",
            jar.display()
        )));
    }
    let mut command = tokio::process::Command::new(java);
    command
        .arg(format!("-Xmx{}M", meta.memory_mb.max(512)))
        .arg("-jar")
        .arg(&jar)
        .arg("nogui")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::util::hidden_async(&mut command);
    let mut child = command.spawn().map_err(|err| {
        AppError::Other(format!(
            "启动服务端失败（{}）：{err}。请确认 Java 版本与核心匹配",
            java.display()
        ))
    })?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    Ok(SpawnedServer {
        child,
        stdin,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAPER: &str = r#"[
        {"id":196,"channel":"STABLE","time":"2023-09-21T16:59:41Z","downloads":{"server:default":{"name":"paper-1.20.1-196.jar","checksums":{"sha256":"ab"},"size":43230582,"url":"https://fill-data.papermc.io/v1/objects/ab/paper-1.20.1-196.jar"}}},
        {"id":197,"channel":"ALPHA","downloads":{"server:default":{"name":"paper-1.20.1-197.jar","size":1,"url":"https://x/alpha.jar"}}},
        {"id":195,"channel":"STABLE","downloads":{"server:default":{"name":"old.jar","size":10,"url":"https://x/old.jar"}}}
    ]"#;

    #[test]
    fn paper_picks_latest_stable_build() {
        let picked = parse_paper_builds(PAPER, "1.20.1").unwrap();
        assert_eq!(picked.build.as_deref(), Some("196"), "要取 STABLE 里 id 最大的");
        assert_eq!(picked.size_bytes, Some(43_230_582));
        assert!(picked.url.ends_with("paper-1.20.1-196.jar"));
        assert!(parse_paper_builds("[]", "1.20.1").is_none());
        assert!(parse_paper_builds("not json", "1.20.1").is_none());
    }

    #[test]
    fn purpur_latest_builds_direct_download_url() {
        let json = r#"{"project":"purpur","version":"1.20.1","build":"2062","result":"SUCCESS"}"#;
        let picked = parse_purpur_latest(json, "1.20.1").unwrap();
        assert_eq!(picked.build.as_deref(), Some("2062"));
        assert_eq!(picked.url, "https://api.purpurmc.org/v2/purpur/1.20.1/latest/download");
    }

    #[test]
    fn fabric_uses_stable_loader_and_installer() {
        let loaders = r#"[{"loader":{"version":"0.16.9","stable":false}},{"loader":{"version":"0.15.11","stable":true}}]"#;
        let installers = r#"[{"version":"1.1.2","stable":true},{"version":"1.0.0","stable":false}]"#;
        let picked = parse_fabric_versions(loaders, installers, "1.20.1").unwrap();
        assert_eq!(picked.build.as_deref(), Some("loader 0.15.11"));
        assert_eq!(
            picked.url,
            "https://meta.fabricmc.net/v2/versions/loader/1.20.1/0.15.11/1.1.2/server/jar"
        );
    }

    #[test]
    fn forge_promo_prefers_recommended() {
        let json = r#"{"promos":{"1.20.1-latest":"47.4.28","1.20.1-recommended":"47.3.0"}}"#;
        let picked = parse_forge_promo(json, "1.20.1").unwrap();
        assert_eq!(picked.build.as_deref(), Some("47.3.0"));
        assert_eq!(
            picked.url,
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.3.0/forge-1.20.1-47.3.0-installer.jar"
        );
        assert!(parse_forge_promo(r#"{"promos":{}}"#, "1.20.1").is_none());
    }

    #[test]
    fn neoforge_metadata_filters_by_mc_prefix() {
        let xml = r#"<metadata><versioning><versions><version>20.2.86</version><version>20.4.237</version><version>21.1.1</version></versions></versioning></metadata>"#;
        let picked = parse_neoforge_metadata(xml, "1.20.4").unwrap();
        assert_eq!(picked.build.as_deref(), Some("20.4.237"));
        assert!(picked.url.contains("neoforge-20.4.237-installer.jar"));
        assert!(parse_neoforge_metadata(xml, "1.19.2").is_none());
    }

    #[test]
    fn vanilla_urls_come_from_manifest_and_version_json() {
        let manifest = r#"{"versions":[{"id":"1.20.1","url":"https://launchermeta.mojang.com/v1/packages/abc/1.20.1.json"},{"id":"1.21.4","url":"https://x/2.json"}]}"#;
        let url = parse_manifest_json_url(manifest, "1.20.1").unwrap();
        assert!(url.ends_with("1.20.1.json"));
        assert!(parse_manifest_json_url(manifest, "9.9.9").is_none());

        let version_json = r#"{"downloads":{"server":{"url":"https://piston-data.mojang.com/v1/objects/xx/server.jar","size":51000000}}}"#;
        assert_eq!(
            parse_version_server_url(version_json).unwrap(),
            "https://piston-data.mojang.com/v1/objects/xx/server.jar"
        );
        assert!(parse_version_server_url("{}").is_none());
    }

#[test]
    fn command_normalisation_strips_slash_and_spaces() {
        assert_eq!(normalize_command("list").unwrap(), "list");
        assert_eq!(normalize_command("/list").unwrap(), "list");
        assert_eq!(normalize_command("  /save-all  ").unwrap(), "save-all");
        assert_eq!(normalize_command("//list").unwrap(), "list");
        assert_eq!(normalize_command("say hi there").unwrap(), "say hi there");
        assert!(normalize_command("").is_err());
        assert!(normalize_command("   ").is_err());
        assert!(normalize_command("/").is_err());
        assert!(normalize_command("list\nstop").is_err());
        assert!(command_line("list").ends_with('\n'));
        assert_eq!(command_line("list"), "list\n");
    }

    #[test]
    fn history_ring_keeps_last_fifty() {
        let mut history = Vec::new();
        for index in 0..60 {
            push_history(&mut history, &format!("cmd{index}"));
        }
        assert_eq!(history.len(), HISTORY_CAP);
        assert_eq!(HISTORY_CAP, 50);
        assert_eq!(history.first().map(String::as_str), Some("cmd10"));
        assert_eq!(history.last().map(String::as_str), Some("cmd59"));
    }

    #[tokio::test]
    async fn send_command_rejects_when_server_is_not_running() {
        let store = ServerStore::new();
        let error = store.send_command(1, "list").await.unwrap_err().to_string();
        assert!(error.contains("服务端未运行"), "{error}");
        let error = store.send_command(1, "/  ").await.unwrap_err().to_string();
        assert!(error.contains("不能为空"), "{error}");
        let state = store.console_state(1).await;
        assert!(!state.running);
        assert!(!state.stdin_open);
        assert!(state.history.is_empty());
    }

    #[tokio::test]
    async fn send_command_writes_a_line_into_a_real_process_stdin() {
        use tokio::io::AsyncReadExt;
        // cmd /c more 会把 stdin 原样回显到 stdout：用它验证「真的写进去了」
        let mut command = tokio::process::Command::new("cmd");
        command
            .args(["/c", "more"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        crate::util::hidden_async(&mut command);
        let mut child = command.spawn().expect("spawn cmd more");
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("stdout");
        let store = ServerStore::new();
        store
            .insert(
                7,
                LiveServer {
                    child,
                    stdin: stdin.take(),
                    state: ServerState::Running,
                    players: 0,
                    history: Vec::new(),
                },
            )
            .await;
        store.send_command(7, "/list").await.unwrap();
        let mut buffer = [0u8; 64];
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            stdout.read(&mut buffer),
        )
        .await
        .expect("读回显超时")
        .expect("读回显失败");
        let echoed = String::from_utf8_lossy(&buffer[..read]).to_string();
        assert!(echoed.contains("list"), "回显里要有指令：{echoed:?}");
        let state = store.console_state(7).await;
        assert!(state.running);
        assert!(state.stdin_open);
        assert_eq!(state.history, vec!["list".to_string()]);
        // 停止后 stdin 关闭、进程表清空
        store.remove(7).await;
        let state = store.console_state(7).await;
        assert!(!state.running);
        assert!(!state.stdin_open);
    }

    #[tokio::test]
    async fn console_state_exposes_history_and_rejects_after_stop() {
        let store = ServerStore::new();
        let mut command = tokio::process::Command::new("cmd");
        command
            .args(["/c", "more"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::util::hidden_async(&mut command);
        let mut child = command.spawn().expect("spawn");
        let stdin = child.stdin.take();
        store
            .insert(
                9,
                LiveServer {
                    child,
                    stdin,
                    state: ServerState::Running,
                    players: 2,
                    history: Vec::new(),
                },
            )
            .await;
        store.send_command(9, "list").await.unwrap();
        store.send_command(9, "save-all").await.unwrap();
        let state = store.console_state(9).await;
        assert_eq!(
            state.history,
            vec!["list".to_string(), "save-all".to_string()]
        );
        store.set_state(9, ServerState::Stopping).await;
        let error = store.send_command(9, "list").await.unwrap_err().to_string();
        assert!(error.contains("服务端未运行"), "{error}");
        store.remove(9).await;
    }

    #[tokio::test]
    async fn request_stop_needs_a_live_entry_with_stdin() {
        let store = ServerStore::new();
        assert!(!store.request_stop(3).await, "没有进程时写不了 stop");
        let mut command = tokio::process::Command::new("cmd");
        command
            .args(["/c", "more"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::util::hidden_async(&mut command);
        let child = command.spawn().expect("spawn");
        store
            .insert(
                3,
                LiveServer {
                    child,
                    stdin: None,
                    state: ServerState::Running,
                    players: 0,
                    history: Vec::new(),
                },
            )
            .await;
        assert!(!store.request_stop(3).await, "stdin 不在时不能假装停服");
        store.remove(3).await;
    }

    #[test]
    fn properties_patch_keeps_order_and_comments() {
        let text = "#Minecraft server properties\r\nmotd=旧\r\nserver-port=25565\r\nonline-mode=true\r\n";
        let mut set = PropertySet::parse(text);
        assert_eq!(set.get("motd"), Some("旧"));
        let mut patch = BTreeMap::new();
        patch.insert("motd".to_string(), "新".to_string());
        patch.insert("max-players".to_string(), "30".to_string());
        set.patch(&patch);
        let rendered = set.render();
        assert!(rendered.starts_with("#Minecraft server properties\r\n"));
        assert!(rendered.contains("motd=新\r\n"));
        assert!(rendered.contains("server-port=25565"));
        assert!(rendered.contains("max-players=30"), "新键追加在末尾");
        // 顺序没变
        let motd_index = rendered.find("motd=").unwrap();
        let port_index = rendered.find("server-port=").unwrap();
        assert!(motd_index < port_index);
        assert_eq!(set.get("online-mode"), Some("true"));
    }

    #[test]
    fn default_properties_contain_expected_keys() {
        let text = default_properties("我的服", 25577, false, 10);
        let set = PropertySet::parse(&text);
        assert_eq!(set.get("server-port"), Some("25577"));
        assert_eq!(set.get("online-mode"), Some("false"));
        assert_eq!(set.get("max-players"), Some("10"));
        assert_eq!(set.get("motd"), Some("我的服"));
    }

    #[test]
    fn start_bat_quotes_paths_and_memory() {
        let text = start_bat_content(
            Path::new("C:\\Program Files\\Java\\jdk-21\\bin\\javaw.exe"),
            1024,
            "paper-1.20.1.jar",
            false,
        );
        assert!(text.contains("\"C:\\Program Files\\Java\\jdk-21\\bin\\javaw.exe\" -Xmx1024M"));
        assert!(text.contains("\"paper-1.20.1.jar\" nogui"));
        assert!(text.contains("cd /d \"%~dp0\""));
        assert!(!text.contains("--installServer"));

        let forge = start_bat_content(Path::new("C:\\jdk\\bin\\java.exe"), 2048, "forge.jar", true);
        assert!(forge.contains("--installServer"), "forge 首次要跑 installer");
    }

    #[test]
    fn eula_content_toggles() {
        assert!(eula_content(true).contains("eula=true"));
        assert!(eula_content(false).contains("eula=false"));
    }

    #[test]
    fn netstat_parsing_detects_listening_port() {
        let output = "\r\n  TCP    0.0.0.0:25565          0.0.0.0:0              LISTENING       4242\r\n  TCP    0.0.0.0:25577          0.0.0.0:0              LISTENING       4343\r\n";
        assert!(parse_netstat_listening(output, 25565));
        assert!(parse_netstat_listening(output, 25577));
        assert!(!parse_netstat_listening(output, 25565 + 1));
        assert!(!parse_netstat_listening("", 25565));
        // 只有 ESTABLISHED 不算占用
        let established = "  TCP    127.0.0.1:25565        127.0.0.1:51000        ESTABLISHED     4242";
        assert!(!parse_netstat_listening(established, 25565));
    }

    #[test]
    fn log_lines_recognize_ready_and_players() {
        let ready = "[12:00:00] [Server thread/INFO]: Done (3.215s)! For help, type \"help\"";
        assert!(is_server_ready(ready));
        assert!(!is_server_ready("[12:00:00] [Server thread/INFO]: Starting minecraft server version 1.20.1"));

        assert_eq!(
            parse_player_event("[12:00:01] [Server thread/INFO]: Steve joined the game"),
            Some((true, "Steve".to_string()))
        );
        assert_eq!(
            parse_player_event("[12:00:02] [Server thread/INFO]: Steve left the game"),
            Some((false, "Steve".to_string()))
        );
        assert_eq!(parse_player_event("[12:00:03] [Server thread/INFO]: Saving chunks"), None);
    }

    #[test]
    fn cores_table_lists_six_with_pages() {
        let table = cores_table();
        let ids: Vec<&str> = table.iter().map(|core| core.id.as_str()).collect();
        assert_eq!(ids, vec!["vanilla", "paper", "purpur", "fabric", "forge", "neoforge"]);
        assert!(table.iter().all(|core| core.download_page.starts_with("https://")));
        assert_eq!(core_name("paper"), "Paper");
        assert_eq!(core_name("xx"), "xx");
        assert!(FAST_MIRROR_PAGE.starts_with("https://www.fastmirror.net/"));
    }

    #[test]
    fn endpoints_match_measured_urls() {
        assert_eq!(
            paper_builds_url("1.20.1"),
            "https://fill.papermc.io/v3/projects/paper/versions/1.20.1/builds"
        );
        assert_eq!(
            purpur_latest_url("1.20.1"),
            "https://api.purpurmc.org/v2/purpur/1.20.1/latest"
        );
        assert_eq!(
            fabric_installer_url(),
            "https://meta.fabricmc.net/v2/versions/installer"
        );
        assert!(neoforge_metadata_url().contains("maven.neoforged.net"));
        assert!(forge_promotions_url().ends_with("promotions_slim.json"));
    }

    #[test]
    fn layout_and_name_validation() {
        let root = std::env::temp_dir().join(format!("gugu-server-layout-{}", util::unique_suffix()));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        let layout = layout(&paths, "我的 服/1");
        assert_eq!(layout.dir.parent().unwrap(), servers_root(&paths));
        assert!(layout.dir.file_name().unwrap().to_string_lossy().contains("我的 服_1"));

        assert!(validate_name(&paths, "  ").is_err());
        assert_eq!(validate_name(&paths, "abc").unwrap(), "abc");
        std::fs::create_dir_all(&layout.dir).unwrap();
        assert!(validate_name(&paths, "我的 服/1").is_err(), "同名目录要拒绝");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn meta_roundtrip_and_list() {
        let root = std::env::temp_dir().join(format!("gugu-server-meta-{}", util::unique_suffix()));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        let dir = servers_root(&paths).join("s1");
        std::fs::create_dir_all(&dir).unwrap();
        let meta = ServerMeta {
            id: 42,
            name: "s1".to_string(),
            core: "paper".to_string(),
            mc_version: "1.20.1".to_string(),
            port: 25577,
            memory_mb: 1024,
            jar: "paper-1.20.1.jar".to_string(),
            created_at: 1_700_000_000,
        };
        write_meta(&dir, &meta).unwrap();
        assert_eq!(read_meta(&dir), Some(meta.clone()));
        let all = list(&paths);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, 42);
        let (found_dir, found) = find(&paths, 42).unwrap();
        assert_eq!(found_dir, dir);
        assert_eq!(found.core, "paper");
        assert!(find(&paths, 7).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
