//! 真正的下载器：多源回退 + 429 退避 + 流式落盘 + 大文件分片 + sha1 校验。
//!
//! 源顺序由 crate::sources 决定（auto 会先测速），单个源失败立刻换下一个，
//! 429 在同一源上退避 BACKOFF_MS 后重试。取消靠共享的 AtomicBool，不下发任何 panic。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sha1::{Digest, Sha1};

use crate::download::plan_ranges;
use crate::error::{AppError, Result};
use crate::sources::{map_origin, Origin};

/// 单源内的重试次数（429 也走这里的循环）。
pub const RETRY_LIMIT: usize = 3;
/// 429 退避时间：任务书要求 800ms。
pub const BACKOFF_MS: u64 = 800;
/// 低于这个大小就不切片（分片开销不值得）。
pub const CHUNK_THRESHOLD: u64 = 8 * 1024 * 1024;
/// 单个分片至少这么大，避免把一个 8MB 文件切成一堆小请求。
pub const MIN_CHUNK_BYTES: u64 = 1024 * 1024;

/// 一次下载结果。
#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub bytes: u64,
    pub skipped: bool,
    pub source: String,
}

/// 已经存在且大小对得上就不用再下（assets 是内容寻址，文件名即哈希）。
pub fn should_skip(dest: &Path, size: Option<u64>) -> bool {
    match std::fs::metadata(dest) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => match size {
            Some(expected) => meta.len() == expected,
            None => true,
        },
        _ => false,
    }
}

/// 分片大小：按设置的连接数尽量均分，但不小于 MIN_CHUNK_BYTES。
pub fn chunk_size_for(total: u64, connections: usize) -> u64 {
    if connections <= 1 || total < CHUNK_THRESHOLD {
        return total.max(1);
    }
    let per = total / connections as u64 + 1;
    per.max(MIN_CHUNK_BYTES)
}

/// 计算文件 sha1（内容下载校验用）。
pub fn sha1_file(path: &Path) -> Result<String> {
    sha1_hex_file(path)
}

fn sha1_hex_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    Ok(hex_of(&hasher.finalize()))
}

fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

enum FetchError {
    Cancelled,
    TooManyRequests,
    Other(String),
}

/// 多源下载器。clone 便宜（内部全是 Arc / reqwest 客户端）。
#[derive(Clone)]
pub struct Fetcher {
    client: reqwest::Client,
    origins: Vec<Origin>,
    cancel: Arc<AtomicBool>,
    /// 单文件分片线程数（来自设置 perFileConnections）
    connections: usize,
}

impl Fetcher {
    pub fn new(
        client: reqwest::Client,
        origins: Vec<Origin>,
        cancel: Arc<AtomicBool>,
        connections: usize,
    ) -> Self {
        Self {
            client,
            origins,
            cancel,
            connections: connections.max(1),
        }
    }

    pub fn origins(&self) -> &[Origin] {
        &self.origins
    }

    pub fn cancel(&self) -> &Arc<AtomicBool> {
        &self.cancel
    }

    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(AppError::Cancelled);
        }
        Ok(())
    }

    fn first_source_id(&self) -> String {
        self.origins
            .first()
            .map(|origin| origin.id().to_string())
            .unwrap_or_else(|| "official".to_string())
    }

    /// 下载一个文件到 dest（先写 .part 再重命名，进程被杀不会留半截文件）。
    pub async fn fetch(
        &self,
        url: &str,
        dest: &Path,
        sha1: Option<&str>,
        size: Option<u64>,
    ) -> Result<FetchOutcome> {
        self.check_cancelled()?;
        if should_skip(dest, size) {
            let bytes = std::fs::metadata(dest).map(|meta| meta.len()).unwrap_or(0);
            return Ok(FetchOutcome {
                bytes,
                skipped: true,
                source: self.first_source_id(),
            });
        }

        let mut failures: Vec<String> = Vec::new();
        for origin in &self.origins {
            self.check_cancelled()?;
            let mapped = map_origin(origin, url);

            // 大文件先试分片并发，失败再回落到单流
            if let Some(size) = size {
                if size >= CHUNK_THRESHOLD && self.connections > 1 {
                    match self.download_chunked(&mapped, dest, size).await {
                        Ok(bytes) => {
                            self.verify(dest, sha1, origin.id(), &mut failures)?;
                            return Ok(FetchOutcome {
                                bytes,
                                skipped: false,
                                source: origin.id().to_string(),
                            });
                        }
                        Err(FetchError::Cancelled) => return Err(AppError::Cancelled),
                        Err(FetchError::Other(message)) => {
                            tracing::warn!(
                                source = origin.id(),
                                %message,
                                "分片下载失败，改用单流"
                            );
                        }
                        Err(FetchError::TooManyRequests) => {
                            tokio::time::sleep(Duration::from_millis(BACKOFF_MS)).await;
                        }
                    }
                }
            }

            let mut attempt = 0usize;
            while attempt < RETRY_LIMIT {
                attempt += 1;
                match self.download_once(&mapped, dest).await {
                    Ok(bytes) => {
                        self.verify(dest, sha1, origin.id(), &mut failures)?;
                        return Ok(FetchOutcome {
                            bytes,
                            skipped: false,
                            source: origin.id().to_string(),
                        });
                    }
                    Err(FetchError::Cancelled) => return Err(AppError::Cancelled),
                    Err(FetchError::TooManyRequests) => {
                        tracing::warn!(source = origin.id(), url = %mapped, "429 限流，退避后重试");
                        tokio::time::sleep(Duration::from_millis(BACKOFF_MS)).await;
                        continue;
                    }
                    Err(FetchError::Other(message)) => {
                        failures.push(format!("{}: {message}", origin.id()));
                        break;
                    }
                }
            }
        }

        let tried = self
            .origins
            .iter()
            .map(|origin| origin.id())
            .collect::<Vec<_>>()
            .join("、");
        let last = failures
            .last()
            .cloned()
            .unwrap_or_else(|| "没有可用的下载源".to_string());
        Err(AppError::Net(format!(
            "{url} 全部源都失败（已尝试 {tried}）：{last}"
        )))
    }

    /// sha1 校验失败时删掉落盘文件并记一条失败（交给下一个源重试）。
    fn verify(
        &self,
        dest: &Path,
        sha1: Option<&str>,
        source_id: &str,
        failures: &mut Vec<String>,
    ) -> Result<()> {
        let Some(expected) = sha1 else {
            return Ok(());
        };
        if expected.is_empty() {
            return Ok(());
        }
        let actual = sha1_hex_file(dest)?;
        if actual.eq_ignore_ascii_case(expected) {
            return Ok(());
        }
        let _ = std::fs::remove_file(dest);
        failures.push(format!("{source_id}: sha1 校验失败"));
        Err(AppError::Net(format!("{source_id}: sha1 校验失败")))
    }

    async fn download_once(&self, url: &str, dest: &Path) -> std::result::Result<u64, FetchError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|err| FetchError::Other(short(&err.to_string())))?;

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(FetchError::TooManyRequests);
        }
        if !response.status().is_success() {
            return Err(FetchError::Other(format!("HTTP {}", response.status().as_u16())));
        }

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| FetchError::Other(format!("建目录失败：{err}")))?;
        }
        let tmp: PathBuf = dest.with_extension("part");
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|err| FetchError::Other(format!("创建文件失败：{err}")))?;

        let mut response = response;
        let mut written: u64 = 0;
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                drop(file);
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(FetchError::Cancelled);
            }
            let chunk = response
                .chunk()
                .await
                .map_err(|err| FetchError::Other(short(&err.to_string())))?;
            let Some(bytes) = chunk else { break };
            tokio::io::AsyncWriteExt::write_all(&mut file, &bytes)
                .await
                .map_err(|err| FetchError::Other(format!("写文件失败：{err}")))?;
            written += bytes.len() as u64;
        }
        tokio::io::AsyncWriteExt::flush(&mut file)
            .await
            .map_err(|err| FetchError::Other(format!("刷新失败：{err}")))?;
        drop(file);
        tokio::fs::rename(&tmp, dest)
            .await
            .map_err(|err| FetchError::Other(format!("重命名失败：{err}")))?;
        if written == 0 {
            return Err(FetchError::Other("响应为空".to_string()));
        }
        Ok(written)
    }

    /// 分片并发下载：每片用 Range 请求取回后再顺序写盘。
    async fn download_chunked(
        &self,
        url: &str,
        dest: &Path,
        size: u64,
    ) -> std::result::Result<u64, FetchError> {
        let chunk_size = chunk_size_for(size, self.connections);
        let ranges = plan_ranges(size, chunk_size);
        if ranges.len() <= 1 {
            return Err(FetchError::Other("分片不足，改用单流".to_string()));
        }

        let mut set = tokio::task::JoinSet::new();
        for range in ranges {
            let client = self.client.clone();
            let url = url.to_string();
            let cancel = self.cancel.clone();
            set.spawn(async move {
                if cancel.load(Ordering::SeqCst) {
                    return Err(FetchError::Cancelled);
                }
                let response = client
                    .get(&url)
                    .header(
                        reqwest::header::RANGE,
                        format!("bytes={}-{}", range.start, range.end),
                    )
                    .send()
                    .await
                    .map_err(|err| FetchError::Other(short(&err.to_string())))?;
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(FetchError::TooManyRequests);
                }
                if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                    return Err(FetchError::Other(format!(
                        "服务器不支持分片（HTTP {}）",
                        response.status().as_u16()
                    )));
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|err| FetchError::Other(short(&err.to_string())))?;
                Ok((range.start, bytes.to_vec()))
            });
        }

        let mut parts: Vec<(u64, Vec<u8>)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            let part = joined.map_err(|err| FetchError::Other(format!("分片任务异常：{err}")))??;
            parts.push(part);
        }
        parts.sort_by_key(|(start, _)| *start);

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| FetchError::Other(format!("建目录失败：{err}")))?;
        }
        let tmp = dest.with_extension("part");
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|err| FetchError::Other(format!("创建文件失败：{err}")))?;
        let mut written = 0u64;
        for (_, bytes) in &parts {
            tokio::io::AsyncWriteExt::write_all(&mut file, bytes)
                .await
                .map_err(|err| FetchError::Other(format!("写文件失败：{err}")))?;
            written += bytes.len() as u64;
        }
        tokio::io::AsyncWriteExt::flush(&mut file)
            .await
            .map_err(|err| FetchError::Other(format!("刷新失败：{err}")))?;
        drop(file);
        tokio::fs::rename(&tmp, dest)
            .await
            .map_err(|err| FetchError::Other(format!("重命名失败：{err}")))?;
        Ok(written)
    }

    /// 拉一个文本资源（版本清单 / 版本 json / assets 索引），返回正文。
    pub async fn fetch_text(&self, url: &str) -> Result<String> {
        self.check_cancelled()?;
        let mut failures: Vec<String> = Vec::new();
        for origin in &self.origins {
            self.check_cancelled()?;
            let mapped = map_origin(origin, url);
            let mut attempt = 0usize;
            while attempt < RETRY_LIMIT {
                attempt += 1;
                match self.client.get(&mapped).send().await {
                    Ok(response) => {
                        let status = response.status();
                        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            tokio::time::sleep(Duration::from_millis(BACKOFF_MS)).await;
                            continue;
                        }
                        if !status.is_success() {
                            failures.push(format!("{}: HTTP {}", origin.id(), status.as_u16()));
                            break;
                        }
                        return response
                            .text()
                            .await
                            .map_err(|err| AppError::Net(format!("读取响应失败：{err}")));
                    }
                    Err(err) => {
                        failures.push(format!("{}: {}", origin.id(), short(&err.to_string())));
                        break;
                    }
                }
            }
        }
        let last = failures
            .last()
            .cloned()
            .unwrap_or_else(|| "没有可用的下载源".to_string());
        Err(AppError::Net(format!("{url} 全部源都失败：{last}")))
    }
}

fn short(message: &str) -> String {
    message.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::DownloadSource;

    #[test]
    fn should_skip_respects_size_and_emptiness() {
        let dir = std::env::temp_dir().join(format!("gugu-fetch-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.jar");

        assert!(!should_skip(&file, None));
        std::fs::write(&file, b"hello").unwrap();
        assert!(should_skip(&file, Some(5)));
        assert!(!should_skip(&file, Some(6)));
        assert!(should_skip(&file, None));

        std::fs::write(&file, b"").unwrap();
        assert!(!should_skip(&file, None), "空文件要重下");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha1_hex_matches_known_digest() {
        let dir = std::env::temp_dir().join(format!("gugu-fetch-sha1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("b.txt");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha1_hex_file(&file).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chunk_size_respects_connections_and_threshold() {
        // 小文件不切片
        assert_eq!(chunk_size_for(1024, 8), 1024);
        assert_eq!(chunk_size_for(CHUNK_THRESHOLD - 1, 8), CHUNK_THRESHOLD - 1);
        // 单连接不切片
        assert_eq!(chunk_size_for(CHUNK_THRESHOLD, 1), CHUNK_THRESHOLD);
        // 8 连接、32MB → 每片约 4MB
        let size = 32 * 1024 * 1024;
        let chunk = chunk_size_for(size, 8);
        assert_eq!(chunk, size / 8 + 1);
        assert!(plan_ranges(size, chunk).len() <= 9);
        // 连接多到超过最小分片时，不会被切得太碎
        let chunk = chunk_size_for(9 * 1024 * 1024, 16);
        assert_eq!(chunk, MIN_CHUNK_BYTES);
    }

    #[test]
    fn fetcher_keeps_origin_order_and_connections() {
        let cancel = Arc::new(AtomicBool::new(false));
        let fetcher = Fetcher::new(
            reqwest::Client::new(),
            vec![
                Origin::Builtin(DownloadSource::Bmclapi),
                Origin::Builtin(DownloadSource::Official),
            ],
            cancel,
            4,
        );
        assert_eq!(fetcher.origins()[0].id(), "bmclapi");
        assert_eq!(fetcher.origins().len(), 2);
    }

    #[tokio::test]
    async fn cancelled_fetcher_short_circuits() {
        let cancel = Arc::new(AtomicBool::new(true));
        let fetcher = Fetcher::new(
            reqwest::Client::new(),
            vec![Origin::Builtin(DownloadSource::Official)],
            cancel,
            1,
        );
        let dest = std::env::temp_dir().join("gugu-cancel-dest.jar");
        let err = fetcher
            .fetch("https://example.com/x.jar", &dest, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Cancelled));
        assert!(fetcher.check_cancelled().is_err());
    }

    #[tokio::test]
    async fn skip_existing_file_without_network() {
        let dir = std::env::temp_dir().join(format!("gugu-fetch-skip2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("c.jar");
        std::fs::write(&dest, b"12345").unwrap();
        let fetcher = Fetcher::new(
            reqwest::Client::new(),
            vec![Origin::Builtin(DownloadSource::Official)],
            Arc::new(AtomicBool::new(false)),
            4,
        );
        let outcome = fetcher
            .fetch("https://example.com/c.jar", &dest, None, Some(5))
            .await
            .unwrap();
        assert!(outcome.skipped);
        assert_eq!(outcome.bytes, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
