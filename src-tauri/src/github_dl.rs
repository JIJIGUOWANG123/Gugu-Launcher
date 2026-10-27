//! GitHub 下载器：URL 白名单 + 加速节点（auto 下载前强制重测）+ 分片并发。
//!
//! 进度沿 install://progress 推送（taskId 前缀 github-），失败推 install://error，
//! 取消走 github_cancel（任务表里的取消标志）。

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::download::plan_ranges;
use crate::downloader::chunk_size_for;
use crate::error::{AppError, Result};
use crate::events::Phase;
use crate::net::github::{accelerated_url, find_node, order_ok_by_latency, probe_all, GITHUB_NODES};
use crate::paths::Paths;
use crate::settings::Settings;
use crate::tasks::TaskHandle;

/// 允许下载的主机（只认这几家，避免被当成通用下载器）。
pub const ALLOWED_HOSTS: [&str; 7] = [
    "github.com",
    "www.github.com",
    "raw.githubusercontent.com",
    "objects.githubusercontent.com",
    "codeload.github.com",
    "github-releases.githubusercontent.com",
    "gist.githubusercontent.com",
];

/// 从 URL 里取主机名（小写）。
pub fn host_of(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let host = rest.split('/').next()?.trim();
    if host.is_empty() {
        return None;
    }
    Some(host.split(':').next().unwrap_or(host).to_string())
}

/// 是否是允许下载的 GitHub 地址。
pub fn is_github_url(url: &str) -> bool {
    match host_of(url) {
        Some(host) => ALLOWED_HOSTS.contains(&host.as_str()),
        None => false,
    }
}

/// 把文件名里的非法字符替换掉（Windows 上不允许 \ / : * ? " < > |）。
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed
    }
}

/// 从 URL 推断文件名：去掉查询串与片段，再做一次百分号解码。
pub fn infer_file_name(url: &str) -> Option<String> {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let without_query = without_fragment.split('?').next().unwrap_or(without_fragment);
    let trimmed = without_query.trim_end_matches('/');
    // 先去掉 scheme://host，只有路径部分参与取文件名（否则 https://example.com/ 会取成 example.com）
    let path = match trimmed.find("://") {
        Some(index) => {
            let rest = &trimmed[index + 3..];
            match rest.find('/') {
                Some(slash) => &rest[slash + 1..],
                None => return None,
            }
        }
        None => trimmed,
    };
    let last = path.rsplit('/').next()?;
    if last.is_empty() {
        return None;
    }
    let decoded = percent_decode(last);
    let name = sanitize_file_name(&decoded);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// 只处理 %xx（URL 文件名里最常见的情况）。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &text[index + 1..index + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 目标目录里已有同名文件时追加 (1)(2)…
pub fn unique_target(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string());
    let extension = path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    for index in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({index}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}{extension}", crate::util::unique_suffix()))
}

/// 落地目录：命令传入 > 设置 > <数据根>/DL。
pub fn resolve_dir(paths: &Paths, settings: &Settings, requested: &str) -> PathBuf {
    let requested = requested.trim();
    if !requested.is_empty() {
        return PathBuf::from(requested);
    }
    let configured = settings.github_download_dir.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    paths.root.join("DL")
}

/// 下载候选地址：按设置的节点（auto 时先强制重测）排好，最后兜底原始地址。
pub async fn candidate_urls(client: &reqwest::Client, settings: &Settings, url: &str) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    match settings.github_accelerator.as_str() {
        "auto" => {
            let results = probe_all(client).await;
            for node in order_ok_by_latency(&results) {
                if let Some(found) = find_node(node) {
                    candidates.push(accelerated_url(found.base, url));
                }
            }
            tracing::info!(nodes = candidates.len(), "GitHub 下载已重测加速节点");
        }
        other => {
            if let Some(node) = find_node(other) {
                candidates.push(accelerated_url(node.base, url));
            }
        }
    }
    // 其余节点也作为候选（失败的会被跳过），最后是原始地址
    for node in GITHUB_NODES.iter() {
        let candidate = accelerated_url(node.base, url);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates.push(url.to_string());
    candidates
}

/// 下一次载任务的完整流程（命令层 spawn 它）。
pub async fn download(
    client: reqwest::Client,
    paths: Paths,
    settings: Settings,
    task: TaskHandle,
    url: String,
    dir: String,
    file_name: String,
) -> Result<PathBuf> {
    if !is_github_url(&url) {
        return Err(AppError::Invalid(format!(
            "只支持从 GitHub 下载（github.com / raw.githubusercontent.com 等），收到：{url}"
        )));
    }
    let name = if file_name.trim().is_empty() {
        infer_file_name(&url).ok_or_else(|| {
            AppError::Invalid(format!("无法从地址推断文件名，请手动填写：{url}"))
        })?
    } else {
        sanitize_file_name(file_name.trim())
    };
    let target_dir = resolve_dir(&paths, &settings, &dir);
    std::fs::create_dir_all(&target_dir)?;
    let target = unique_target(&target_dir, &name);
    tracing::info!(
        url = %url,
        target = %target.display(),
        "开始 GitHub 下载"
    );

    let candidates = candidate_urls(&client, &settings, &url).await;
    let connections = settings.effective_connections();
    let mut last_error = String::new();

    for candidate in candidates {
        task.check_cancelled()?;
        match download_from(&client, &candidate, &target, connections, &task).await {
            Ok(bytes) => {
                task.progress_forced(Phase::Finished, "下载完成", 1, 1);
                tracing::info!(
                    bytes,
                    target = %target.display(),
                    source = %candidate,
                    "GitHub 下载完成"
                );
                return Ok(target);
            }
            Err(AppError::Cancelled) => {
                let _ = std::fs::remove_file(&target);
                let _ = std::fs::remove_file(target.with_extension("part"));
                return Err(AppError::Cancelled);
            }
            Err(err) => {
                last_error = err.to_string();
                tracing::warn!(source = %candidate, %err, "该地址下载失败，换下一个");
            }
        }
    }
    Err(AppError::Net(format!("所有 GitHub 地址都下载失败：{last_error}")))
}

/// 从单个地址下载（大文件分片），并汇报进度。
async fn download_from(
    client: &reqwest::Client,
    url: &str,
    target: &Path,
    connections: usize,
    task: &TaskHandle,
) -> Result<u64> {
    task.progress_forced(Phase::Downloading, &format!("正在下载 {}", url), 0, 0);
    let total = head_size(client, url).await;
    let part = target.with_extension("part");
    if let Some(total) = total {
        if total >= 2 * 1024 * 1024 && connections > 1 {
            let chunk = chunk_size_for(total, connections);
            let ranges = plan_ranges(total, chunk);
            if ranges.len() > 1 {
                let mut set = tokio::task::JoinSet::new();
                for range in ranges {
                    let client = client.clone();
                    let url = url.to_string();
                    set.spawn(async move {
                        let response = client
                            .get(&url)
                            .header(
                                reqwest::header::RANGE,
                                format!("bytes={}-{}", range.start, range.end),
                            )
                            .send()
                            .await
                            .map_err(|err| AppError::Other(format!("分片请求失败：{err}")))?;
                        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                            return Err(AppError::Other("服务器不支持分片下载".to_string()));
                        }
                        let bytes = response
                            .bytes()
                            .await
                            .map_err(|err| AppError::Other(format!("读取分片失败：{err}")))?;
                        Ok((range.start, bytes.to_vec()))
                    });
                }
                let mut parts: Vec<(u64, Vec<u8>)> = Vec::new();
                while let Some(joined) = set.join_next().await {
                    task.check_cancelled()?;
                    let part = joined
                        .map_err(|err| AppError::Other(format!("分片任务异常：{err}")))??;
                    parts.push(part);
                }
                parts.sort_by_key(|(start, _)| *start);
                let mut file = std::fs::File::create(&part)?;
                let mut written = 0u64;
                for (_, bytes) in &parts {
                    std::io::Write::write_all(&mut file, bytes)?;
                    written += bytes.len() as u64;
                    task.progress(
                        Phase::Downloading,
                        "正在写入分片",
                        written,
                        total,
                    );
                }
                drop(file);
                std::fs::rename(&part, target)?;
                return Ok(written);
            }
        }
    }

    // 单流兜底
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|err| AppError::Other(format!("请求失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Other(format!("HTTP {}", response.status().as_u16())));
    }
    let expected = response.content_length().or(total).unwrap_or(0);
    let mut file = std::fs::File::create(&part)?;
    let mut written = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| AppError::Other(format!("读取响应失败：{err}")))?
    {
        if task.cancelled() {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(AppError::Cancelled);
        }
        std::io::Write::write_all(&mut file, &chunk)?;
        written += chunk.len() as u64;
        task.progress(Phase::Downloading, "正在下载", written, expected);
    }
    drop(file);
    if written == 0 {
        return Err(AppError::Other("响应为空".to_string()));
    }
    std::fs::rename(&part, target)?;
    Ok(written)
}

async fn head_size(client: &reqwest::Client, url: &str) -> Option<u64> {
    let response = client.head(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.content_length()
}

/// 让编译器确认取消标志确实被读到了（分片路径里会用到）。
pub fn is_cancelled(task: &TaskHandle) -> bool {
    task.cancel_flag().load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("gugu-gh-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    #[test]
    fn whitelist_accepts_github_and_rejects_others() {
        for url in [
            "https://github.com/JIJIGUOWANG123/Gugu-Launcher/releases/download/v1/a.zip",
            "https://raw.githubusercontent.com/o/r/main/file.txt",
            "https://objects.githubusercontent.com/x/y",
            "https://codeload.github.com/o/r/zip/refs/heads/main",
        ] {
            assert!(is_github_url(url), "{url} 应当允许");
        }
        for url in [
            "https://example.com/a.zip",
            "http://github.com.evil.com/a.zip",
            "ftp://github.com/a.zip",
            "github.com/a.zip",
            "",
        ] {
            assert!(!is_github_url(url), "{url} 应当拒绝");
        }
    }

    #[test]
    fn host_extraction_handles_ports_and_case() {
        assert_eq!(host_of("HTTPS://GitHub.COM:443/a/b").unwrap(), "github.com");
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn file_name_inference_handles_query_and_chinese() {
        assert_eq!(
            infer_file_name("https://github.com/o/r/releases/download/v1/GuguLauncher-Setup.exe?raw=1").unwrap(),
            "GuguLauncher-Setup.exe"
        );
        assert_eq!(
            infer_file_name("https://raw.githubusercontent.com/o/r/main/%E6%A8%A1%E7%BB%84.jar").unwrap(),
            "模组.jar"
        );
        // 查询串与片段在截断时就被丢掉了，只剩最后一段
        assert_eq!(
            infer_file_name("https://example.com/a/b/bad:name?.jar#frag").unwrap(),
            "bad_name"
        );
        assert_eq!(
            infer_file_name("https://github.com/a/b/bad:name.jar").unwrap(),
            "bad_name.jar"
        );
        assert_eq!(infer_file_name("https://example.com/"), None);
    }

    #[test]
    fn sanitize_keeps_chinese_and_replaces_forbidden() {
        assert_eq!(sanitize_file_name("整合包 v1.zip"), "整合包 v1.zip");
        assert_eq!(sanitize_file_name("a/b:c*.zip"), "a_b_c_.zip");
        assert_eq!(sanitize_file_name("   "), "download");
        assert_eq!(sanitize_file_name("..."), "download");
    }

    #[test]
    fn duplicate_names_get_suffixes() {
        let dir = std::env::temp_dir().join(format!("gugu-gh-dup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_target(&dir, "a.zip");
        assert_eq!(first.file_name().unwrap(), "a.zip");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_target(&dir, "a.zip");
        assert_eq!(second.file_name().unwrap(), "a (1).zip");
        std::fs::write(&second, b"x").unwrap();
        let third = unique_target(&dir, "a.zip");
        assert_eq!(third.file_name().unwrap(), "a (2).zip");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_defaults_to_data_root_dl() {
        let paths = temp_paths("dir");
        let mut settings = Settings::default();
        assert_eq!(resolve_dir(&paths, &settings, ""), paths.root.join("DL"));
        settings.github_download_dir = "D:\\Games\\DL".to_string();
        assert_eq!(
            resolve_dir(&paths, &settings, ""),
            PathBuf::from("D:\\Games\\DL")
        );
        assert_eq!(
            resolve_dir(&paths, &settings, "E:\\tmp"),
            PathBuf::from("E:\\tmp")
        );
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[tokio::test]
    async fn cancelled_task_stops_immediately() {
        let paths = temp_paths("cancel");
        let store = crate::tasks::TaskStore::new();
        let task = store.create("github", "测试下载", &paths.root.display().to_string());
        store.cancel(task.id()).unwrap();
        let err = download(
            reqwest::Client::new(),
            paths.clone(),
            Settings::default(),
            task,
            "https://github.com/o/r/releases/download/v1/a.zip".to_string(),
            String::new(),
            String::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Cancelled));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[tokio::test]
    async fn non_github_url_is_rejected_before_network() {
        let paths = temp_paths("reject");
        let store = crate::tasks::TaskStore::new();
        let task = store.create("github", "测试下载", &paths.root.display().to_string());
        let err = download(
            reqwest::Client::new(),
            paths.clone(),
            Settings::default(),
            task,
            "https://example.com/a.zip".to_string(),
            String::new(),
            String::new(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("只支持从 GitHub 下载"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
