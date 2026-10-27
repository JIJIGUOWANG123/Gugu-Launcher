//! 静默更新：下载安装包 → 校验（大小 + sha256）→ 生成 cmd 脚本 → detached 执行。
//!
//! 流程（auto 模式）：
//!   1) check 判定 Available（拿到 GuguLauncher-Setup.exe 的下载地址）；
//!   2) 六节点（下载前强制重测）加速下载到 .mc/update/，进度走 install://progress（taskId 前缀 update-）；
//!   3) 校验大小（Release 给了 size 就比），并记录 sha256；
//!   4) 生成 apply-update.cmd：等主进程退出 → \"Setup.exe /S\" → 启动新版 → 自删；
//!   5) detached 启动脚本，主进程退出。
//!
//! 失败保留 .mc/update/ 里的半成品，错误文案可读。

use std::path::{Path, PathBuf};

use crate::error::{AppError, Result};
use crate::events::Phase;
use crate::net::github::{accelerated_url, find_node, order_ok_by_latency, probe_all, GITHUB_NODES};
use crate::paths::Paths;
use crate::tasks::TaskHandle;
use crate::update::version::verify_size;
use crate::util;

pub const UPDATE_DIR: &str = "update";
pub const SETUP_NAME: &str = "GuguLauncher-Setup.exe";
pub const SCRIPT_NAME: &str = "apply-update.cmd";

pub fn update_dir(paths: &Paths) -> PathBuf {
    paths.root.join(UPDATE_DIR)
}

pub fn setup_path(paths: &Paths) -> PathBuf {
    update_dir(paths).join(SETUP_NAME)
}

pub fn script_path(paths: &Paths) -> PathBuf {
    update_dir(paths).join(SCRIPT_NAME)
}

/// 下载候选：按测速顺序的六个加速节点 + 原始地址兜底（去重）。
pub fn download_candidates(asset_url: &str, node_order: &[&'static str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in node_order {
        if let Some(node) = find_node(id) {
            let url = accelerated_url(node.base, asset_url);
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    // 没测通的节点也留作候选（有时探针失败但下载可用）
    for node in GITHUB_NODES.iter() {
        let url = accelerated_url(node.base, asset_url);
        if !out.contains(&url) {
            out.push(url);
        }
    }
    if !out.iter().any(|url| url == asset_url) {
        out.push(asset_url.to_string());
    }
    out
}

/// 待执行的更新（命令层拿到它以后启动脚本并退出）。
#[derive(Debug, Clone)]
pub struct Prepared {
    pub setup: PathBuf,
    pub script: PathBuf,
    pub size: u64,
    pub sha256: String,
}

/// cmd 脚本正文：等主进程退出 → 静默安装 → 启动新版 → 自删。
///
/// 关键点（单测钉住）：`/S` 静默参数、路径一律加引号（Program Files 有空格）、
/// 用 tasklist 轮询 PID 而不是盲等、装完重启启动器、最后删掉自己。
pub fn script_body(setup: &Path, launcher: &Path, wait_pid: u32) -> String {
    format!(
        "@echo off\r\n\
setlocal\r\n\
rem 咕咕启动器自动更新：等待主进程退出后静默安装并重启\r\n\
set \"SETUP={setup}\"\r\n\
set \"LAUNCHER={launcher}\"\r\n\
set \"PID={wait_pid}\"\r\n\
:wait_exit\r\n\
tasklist /FI \"PID eq %PID%\" 2>nul | find \"%PID%\" >nul\r\n\
if not errorlevel 1 (\r\n\
  timeout /t 1 /nobreak >nul\r\n\
  goto wait_exit\r\n\
)\r\n\
start \"\" /wait \"%SETUP%\" /S\r\n\
start \"\" \"%LAUNCHER%\"\r\n\
(goto) 2>nul & del \"%~f0\"\r\n",
        setup = setup.display(),
        launcher = launcher.display(),
        wait_pid = wait_pid
    )
}

/// 写脚本文件（UTF-8 带 BOM 会让 cmd 在中文路径上更稳，这里用纯 ASCII + CRLF）。
pub fn write_script(paths: &Paths, launcher: &Path, wait_pid: u32) -> Result<PathBuf> {
    let script = script_path(paths);
    if let Some(parent) = script.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = script_body(&setup_path(paths), launcher, wait_pid);
    std::fs::write(&script, body)?;
    Ok(script)
}

/// 校验落盘文件：大小对得上 + 记 sha256。
pub fn verify_downloaded(path: &Path, expected_size: Option<u64>) -> Result<(u64, String)> {
    let metadata = std::fs::metadata(path).map_err(|err| {
        AppError::Other(format!("安装包不存在（{}）：{err}", path.display()))
    })?;
    let size = metadata.len();
    if size == 0 {
        return Err(AppError::Other("安装包是空文件，已放弃".to_string()));
    }
    verify_size(expected_size, size).map_err(|message| {
        AppError::Other(format!("安装包校验失败：{message}（文件保留在 {} 便于排查）", path.display()))
    })?;
    let sha256 = util::sha256_file(path)
        .map_err(|err| AppError::Other(format!("计算 sha256 失败：{err}")))?;
    Ok((size, sha256))
}

/// 下载安装包（支持 file:// 便于本地演练），返回 (路径, 大小, sha256)。
pub async fn fetch_asset(
    client: &reqwest::Client,
    paths: &Paths,
    asset_url: &str,
    expected_size: Option<u64>,
    task: &TaskHandle,
) -> Result<(PathBuf, u64, String)> {
    let dir = update_dir(paths);
    std::fs::create_dir_all(&dir)?;
    let target = setup_path(paths);
    let part = target.with_extension("part");

    if asset_url.starts_with("file://") {
        // 本地假资产演练：直接复制，走同一套校验
        let local = file_url_to_path(asset_url);
        task.progress_forced(Phase::Downloading, "正在复制本地安装包", 0, 0);
        std::fs::copy(&local, &part)
            .map_err(|err| AppError::Other(format!("复制本地安装包失败：{err}")))?;
        std::fs::rename(&part, &target)?;
        let (size, sha256) = verify_downloaded(&target, expected_size)?;
        return Ok((target, size, sha256));
    }

    let results = probe_all(client).await;
    let order = order_ok_by_latency(&results);
    let candidates = download_candidates(asset_url, &order);
    let mut last_error = String::new();

    for candidate in candidates {
        task.check_cancelled()?;
        task.progress_forced(
            Phase::Downloading,
            &format!("正在下载安装包（{}）", short_host(&candidate)),
            0,
            0,
        );
        match stream_to_file(client, &candidate, &part, task).await {
            Ok(bytes) => {
                std::fs::rename(&part, &target)?;
                task.progress_forced(
                    Phase::Downloading,
                    &format!("已下载 {bytes} 字节，正在校验"),
                    bytes,
                    bytes,
                );
                let (size, sha256) = verify_downloaded(&target, expected_size)?;
                return Ok((target, size, sha256));
            }
            Err(AppError::Cancelled) => {
                let _ = std::fs::remove_file(&part);
                return Err(AppError::Cancelled);
            }
            Err(err) => {
                last_error = err.to_string();
                tracing::warn!(source = %short_host(&candidate), %err, "该地址下载失败，换下一个");
            }
        }
    }
    Err(AppError::Net(format!(
        "安装包下载失败（已尝试 {} 个地址）：{last_error}",
        download_candidates(asset_url, &order).len()
    )))
}

/// file:///D:/x/y.exe → D:\x\y.exe（Windows 上要先去掉多余的前导斜杠）。
pub fn file_url_to_path(url: &str) -> PathBuf {
    let rest = url.strip_prefix("file://").unwrap_or(url);
    let trimmed = if rest.len() > 3
        && rest.starts_with('/')
        && rest.as_bytes().get(2) == Some(&b':')
    {
        &rest[1..]
    } else {
        rest
    };
    PathBuf::from(trimmed.replace('/', &std::path::MAIN_SEPARATOR.to_string()))
}

fn short_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

async fn stream_to_file(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    task: &TaskHandle,
) -> Result<u64> {
    let mut response = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(|err| AppError::Net(format!("请求失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!("HTTP {}", response.status().as_u16())));
    }
    let total = response.content_length().unwrap_or(0);
    if let Some(parent) = part.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(part)?;
    use std::io::Write;
    let mut written = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| AppError::Net(format!("读取响应失败：{err}")))?
    {
        task.check_cancelled()?;
        file.write_all(&chunk)?;
        written += chunk.len() as u64;
        task.progress(Phase::Downloading, "正在下载安装包", written, total);
    }
    file.flush()?;
    if written == 0 {
        return Err(AppError::Net("响应为空".to_string()));
    }
    Ok(written)
}

/// detached 启动 cmd 脚本（主进程马上退出也不影响它）。
pub fn launch_script(script: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS
        const FLAGS: u32 = 0x0000_0200 | 0x0000_0008;
        std::process::Command::new("cmd")
            .arg("/C")
            .arg(script)
            .creation_flags(FLAGS)
            .spawn()
            .map_err(|err| AppError::Other(format!("启动更新脚本失败：{err}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = script;
        Err(AppError::Other("当前系统不支持静默更新".to_string()))
    }
}

/// 完整准备流程（下载 + 校验 + 写脚本）。
pub async fn prepare(
    client: &reqwest::Client,
    paths: &Paths,
    launcher: &Path,
    asset_url: &str,
    expected_size: Option<u64>,
    task: &TaskHandle,
) -> Result<Prepared> {
    let (setup, size, sha256) = fetch_asset(client, paths, asset_url, expected_size, task).await?;
    tracing::info!(
        size,
        sha256 = %sha256,
        setup = %setup.display(),
        "安装包已下载并校验"
    );
    let wait_pid = std::process::id();
    let script = write_script(paths, launcher, wait_pid)?;
    task.progress_forced(Phase::Extracting, "安装包就绪，正在启动安装程序", 1, 1);
    Ok(Prepared {
        setup,
        script,
        size,
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!(
            "gugu-update-{tag}-{}",
            util::unique_suffix()
        ));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    #[test]
    fn script_waits_for_exit_and_installs_silently() {
        let body = script_body(
            Path::new("C:\\Program Files\\Gugu Launcher\\update\\GuguLauncher-Setup.exe"),
            Path::new("C:\\Program Files\\Gugu Launcher\\GuguLauncher.exe"),
            4242,
        );
        // 静默参数
        assert!(body.contains("/S"), "必须带静默参数：{body}");
        // 路径带空格时必须加引号
        assert!(body.contains("\"%SETUP%\""));
        assert!(body.contains("set \"SETUP=C:\\Program Files\\Gugu Launcher\\update\\GuguLauncher-Setup.exe\""));
        assert!(body.contains("set \"LAUNCHER=C:\\Program Files\\Gugu Launcher\\GuguLauncher.exe\""));
        // 等待主进程退出（按 PID 轮询），不是盲等
        assert!(body.contains("set \"PID=4242\""));
        assert!(body.contains("tasklist /FI \"PID eq %PID%\""));
        assert!(body.contains("goto wait_exit"));
        // 安装完重启启动器并自删
        assert!(body.contains("start \"\" \"%LAUNCHER%\""));
        assert!(body.contains("del \"%~f0\""));
        assert!(body.starts_with("@echo off"));
    }

    #[test]
    fn script_is_written_with_crlf() {
        let paths = temp_paths("script");
        let launcher = Path::new("D:\\Games\\GuguLauncher.exe");
        let script = write_script(&paths, launcher, 1234).unwrap();
        assert_eq!(script.file_name().unwrap(), SCRIPT_NAME);
        let text = std::fs::read_to_string(&script).unwrap();
        assert!(text.contains("\r\n"), "cmd 需要 CRLF");
        assert!(text.contains("LAUNCHER=D:\\Games\\GuguLauncher.exe"));
        assert!(script.starts_with(update_dir(&paths)));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn download_candidates_cover_nodes_then_original() {
        let asset = "https://github.com/o/r/releases/download/v1.0.1/GuguLauncher-Setup.exe";
        let candidates = download_candidates(asset, &["ghfast", "tangbai"]);
        assert_eq!(candidates[0], format!("https://ghfast.top/{asset}"));
        assert_eq!(candidates[1], format!("https://git.tangbai.cc/{asset}"));
        // 六个节点全在 + 原始地址兜底
        assert_eq!(candidates.len(), GITHUB_NODES.len() + 1);
        assert_eq!(candidates.last().unwrap(), asset);
        // 没有重复
        let mut unique = candidates.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), candidates.len());
    }

    #[test]
    fn verify_rejects_size_mismatch_and_empty_file() {
        let paths = temp_paths("verify");
        let file = paths.root.join("fake-setup.exe");
        std::fs::write(&file, vec![0u8; 1024]).unwrap();
        let (size, sha256) = verify_downloaded(&file, Some(1024)).unwrap();
        assert_eq!(size, 1024);
        assert_eq!(sha256.len(), 64);

        let err = verify_downloaded(&file, Some(2048)).unwrap_err().to_string();
        assert!(err.contains("大小不匹配"), "{err}");
        assert!(err.contains("文件保留"), "失败要说明半成品保留位置：{err}");

        std::fs::write(&file, b"").unwrap();
        assert!(verify_downloaded(&file, None).is_err(), "空文件必须拒绝");
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn file_url_to_path_handles_windows_style() {
        assert_eq!(
            file_url_to_path("file:///D:/minecraft/.mc/update/a.exe"),
            PathBuf::from("D:\\minecraft\\.mc\\update\\a.exe")
        );
        assert_eq!(
            file_url_to_path("file://C:/tmp/b.exe"),
            PathBuf::from("C:\\tmp\\b.exe")
        );
    }

    #[tokio::test]
    async fn local_file_url_runs_whole_pipeline() {
        let paths = temp_paths("file");
        let store = crate::tasks::TaskStore::new();
        let task = store.create_with_prefix("update", "更新到 1.0.1", "", "update");
        // 造一个 1MB 的假安装包
        let source = paths.root.join("download").join("GuguLauncher-Setup.exe");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, vec![7u8; 1024 * 1024]).unwrap();

        let url = format!(
            "file:///{}",
            source.display().to_string().replace('\\', "/")
        );
        let prepared = prepare(
            &reqwest::Client::new(),
            &paths,
            Path::new("D:\\minecraft\\GuguLauncher.exe"),
            &url,
            Some(1024 * 1024),
            &task,
        )
        .await
        .unwrap();

        assert_eq!(prepared.size, 1024 * 1024);
        assert_eq!(prepared.sha256.len(), 64);
        assert!(prepared.setup.is_file());
        assert!(prepared.script.is_file());
        assert_eq!(prepared.setup.file_name().unwrap(), SETUP_NAME);
        let body = std::fs::read_to_string(&prepared.script).unwrap();
        assert!(body.contains("/S"));
        assert!(body.contains("/S"));
        assert!(body.contains("GuguLauncher-Setup.exe"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[tokio::test]
    async fn local_file_size_mismatch_fails_readably() {
        let paths = temp_paths("mismatch");
        let store = crate::tasks::TaskStore::new();
        let task = store.create_with_prefix("update", "更新失败用例", "", "update");
        let source = paths.root.join("incoming.exe");
        std::fs::write(&source, vec![1u8; 512]).unwrap();
        let url = format!(
            "file:///{}",
            source.display().to_string().replace('\\', "/")
        );
        let err = fetch_asset(&reqwest::Client::new(), &paths, &url, Some(4096), &task)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("大小不匹配"), "{err}");
        // 半成品保留，便于排查
        assert!(setup_path(&paths).is_file() || setup_path(&paths).with_extension("part").is_file());
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
