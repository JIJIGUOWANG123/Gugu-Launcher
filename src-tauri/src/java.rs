//! Java 运行时：按 JDK 去重探测、多源元数据、下载安装。
//!
//! 探测来源顺序：.mc\runtimes\* → 注册表 JavaSoft（HKLM 与 WOW6432Node）→
//! 常见安装目录 → PATH/JAVA_HOME；同一 JDK 目录只出一条，可执行文件统一取
//! bin\javaw.exe（没有才退回 bin\java.exe）。探测结果缓存 10 分钟。
//!
//! 元数据端点（2026-02 真机实测，见各 URL 构造函数的注释）：BellSoft / Adoptium /
//! 清华 TUNA 目录 / Azul 元数据；Oracle 没有稳定的公开元数据，只给下载页。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::downloader::Fetcher;
use crate::error::{AppError, Result};
use crate::events::Phase;
use crate::paths::Paths;
use crate::sources::{self, Origin};
use crate::tasks::TaskHandle;

/// 老版本下载出来的运行时目录前缀（.mc\runtimes\temurin-17）。
pub const VENDOR: &str = "temurin";
/// 每个运行时目录里的安装信息文件。
pub const STAMP_FILE: &str = ".gugu-java.json";
/// 单次测速超时（与 sources::PROBE_TIMEOUT 一致）。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// 每个源测几次（取中位数）。
pub const PROBE_SAMPLES: usize = 3;
/// 探测缓存有效期。
pub const PROBE_CACHE_TTL: Duration = Duration::from_secs(600);
/// Oracle 只能手动下载。
pub const ORACLE_PAGE: &str = "https://www.oracle.com/java/technologies/downloads/";
/// 可自动下载的源（顺序＝UI 顺序）。
pub const INSTALLABLE_SOURCES: [&str; 4] = ["bellsoft", "adoptium", "tuna", "azul"];
/// 常见 LTS 主版本。
pub const LTS_MAJORS: [u32; 5] = [8, 11, 17, 21, 25];

// ------------------------------------------------------------------ 数据结构

/// java_list() 的元素：一个 JDK 一条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaRuntime {
    /// JDK 根目录（去重键）
    pub path: String,
    /// 实际可执行文件（优先 bin\javaw.exe）
    pub java_exe: String,
    pub major: u32,
    pub version: String,
    pub vendor: String,
    pub arch: String,
    /// 设置里指定的那个，或排序后的第一条
    pub is_default: bool,
}

/// java_sources() 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaSourceInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 需要接受许可、只给下载页（Oracle）
    pub requires_license: bool,
}

/// java_probe_sources() 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaSourceProbe {
    pub source_id: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// java_versions() 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersionInfo {
    pub source: String,
    pub version: String,
    pub major: u32,
    pub lts: bool,
    pub size_bytes: u64,
    pub url: String,
}

/// 运行时目录里的 .gugu-java.json（记录来源，方便排查与升级）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaStamp {
    pub source: String,
    pub vendor: String,
    pub version: String,
    pub major: u32,
    pub url: String,
    pub installed_at: u64,
}

// ------------------------------------------------------------------ 解析纯函数

/// 从 java -version 的输出里解析 (完整版本号, 主版本号)。
pub fn parse_java_version(output: &str) -> Option<(String, u32)> {
    let marker = "version \"";
    let start = output.find(marker)? + marker.len();
    let rest = &output[start..];
    let end = rest.find('"')?;
    let version = rest[..end].trim().to_string();
    if version.is_empty() {
        return None;
    }
    let major = if let Some(stripped) = version.strip_prefix("1.") {
        stripped.split(['.', '_', '-']).next()?.parse::<u32>().ok()?
    } else {
        version.split(['.', '_', '-']).next()?.parse::<u32>().ok()?
    };
    Some((version, major))
}

/// 从 java -XshowSettings 输出里取架构（os.arch / sun.arch.data.model）。
pub fn parse_java_arch(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("os.arch = ") {
            let value = value.trim();
            return Some(match value {
                "amd64" | "x86_64" => "x64".to_string(),
                "x86" => "x86".to_string(),
                "aarch64" => "arm64".to_string(),
                other => other.to_string(),
            });
        }
        if let Some(value) = line.strip_prefix("sun.arch.data.model = ") {
            let value = value.trim();
            return Some(if value == "32" { "x86".to_string() } else { "x64".to_string() });
        }
    }
    None
}

/// 发行版识别：先看供应商关键字，再看 -version 里的 JVM 名称。
pub fn detect_vendor(hint: &str, output: &str) -> String {
    let haystack = format!("{hint} {output}").to_ascii_lowercase();
    let table: [(&str, &str); 9] = [
        ("liberica", "Liberica"),
        ("bellsoft", "Liberica"),
        ("temurin", "Temurin"),
        ("adoptium", "Temurin"),
        ("eclipse adoptium", "Temurin"),
        ("zulu", "Zulu"),
        ("azul", "Zulu"),
        ("corretto", "Corretto"),
        ("amazon", "Corretto"),
    ];
    for (needle, vendor) in table {
        if haystack.contains(needle) {
            return vendor.to_string();
        }
    }
    for (needle, vendor) in [
        ("microsoft", "Microsoft"),
        ("graalvm", "GraalVM"),
        ("sapmachine", "SapMachine"),
        ("dragonwell", "Dragonwell"),
        ("oracle", "Oracle"),
        ("java(tm) se", "Oracle"),
        ("openjdk", "OpenJDK"),
    ] {
        if haystack.contains(needle) {
            return vendor.to_string();
        }
    }
    "未知".to_string()
}

/// 解析 reg query 输出里的 JavaHome 路径。
pub fn parse_reg_query_java_homes(output: &str) -> Vec<String> {
    let mut homes = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if !line.contains("JavaHome") || !line.contains("REG_SZ") {
            continue;
        }
        if let Some(index) = line.find("REG_SZ") {
            let value = line[index + "REG_SZ".len()..].trim();
            if !value.is_empty() && !homes.iter().any(|home| home == value) {
                homes.push(value.to_string());
            }
        }
    }
    homes
}

/// 一个 Java 安装目录下可能的可执行文件（按优先级：javaw 优先）。
pub fn java_exe_candidates(dir: &Path) -> Vec<PathBuf> {
    vec![
        dir.join("bin").join("javaw.exe"),
        dir.join("bin").join("java.exe"),
        dir.join("Contents").join("Home").join("bin").join("java"),
    ]
}

/// 目录里第一个存在的可执行文件。
pub fn java_exe_in(dir: &Path) -> Option<PathBuf> {
    java_exe_candidates(dir)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

/// 去重键：同一 JDK 目录（去掉末尾分隔符、大小写无关）。
pub fn home_key(home: &Path) -> String {
    home.display()
        .to_string()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// 可执行文件 → JDK 根目录（bin 的上一级）。
pub fn jdk_home_of(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    if !bin
        .file_name()
        .map(|name| name.eq_ignore_ascii_case("bin"))
        .unwrap_or(false)
    {
        return None;
    }
    bin.parent().map(Path::to_path_buf)
}

/// 从目录列表 HTML 里挑一个 zip 下载地址（老版本兼容，install 用不到）。
pub fn pick_zip_url(html: &str, base: &str) -> Option<String> {
    let base = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
    let mut fallback: Option<String> = None;
    for chunk in html.split("href=\"").skip(1) {
        let Some(end) = chunk.find('"') else {
            continue;
        };
        let href = &chunk[..end];
        if !href.ends_with(".zip") {
            continue;
        }
        let full = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("{base}{href}")
        };
        if full.contains("-jdk_") || full.contains("/jdk-") {
            return Some(full);
        }
        if fallback.is_none() {
            fallback = Some(full);
        }
    }
    fallback
}

/// 挑最合适的运行时：优先主版本号完全相同，其次取不低于要求的最高版本。
pub fn pick_for_major<'a>(runtimes: &'a [JavaRuntime], required: u32) -> Option<&'a JavaRuntime> {
    if let Some(exact) = runtimes.iter().find(|runtime| runtime.major == required) {
        return Some(exact);
    }
    runtimes
        .iter()
        .filter(|runtime| runtime.major >= required)
        .min_by_key(|runtime| runtime.major)
}

/// 版本 json 没写 javaVersion 时，按 MC 版本推断（1.20.5+ 需要 21，1.18+ 需要 17）。
pub fn infer_major_for_mc_version(version_id: &str) -> u32 {
    let mut parts = version_id.split('.');
    let first = parts.next().and_then(|part| part.parse::<u32>().ok());
    if first != Some(1) {
        // 快照/自定义版本号：按现代版本兜底
        return 17;
    }
    let minor = parts.next().and_then(|part| part.parse::<u32>().ok());
    let patch = parts.next().and_then(|part| part.parse::<u32>().ok());
    match (minor, patch) {
        (Some(minor), _) if minor >= 21 => 21,
        (Some(20), Some(patch)) if patch >= 5 => 21,
        (Some(20), None) => 21,
        (Some(18..=20), _) => 17,
        (Some(17), _) => 16,
        (Some(_), _) => 8,
        (None, _) => 17,
    }
}

// ------------------------------------------------------------------ 探测（带缓存）

#[derive(Debug, Clone)]
struct Probe {
    version: String,
    major: u32,
    arch: String,
    vendor: String,
}

fn probe_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, (Instant, Option<Probe>)>>
{
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, (Instant, Option<Probe>)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn probe_cache_get(key: &str) -> Option<Option<Probe>> {
    let guard = probe_cache().lock().ok()?;
    let (at, value) = guard.get(key)?;
    if at.elapsed() < PROBE_CACHE_TTL {
        Some(value.clone())
    } else {
        None
    }
}

fn probe_cache_put(key: &str, value: Option<Probe>) {
    if let Ok(mut guard) = probe_cache().lock() {
        guard.insert(key.to_string(), (Instant::now(), value));
    }
}

async fn run_java(exe: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args);
    crate::util::hidden_async(&mut cmd);
    let output = cmd.output().await.ok()?;
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// 跑一次 java -version（10 分钟内同一条可执行文件只跑一次）。
async fn probe(exe: &Path) -> Option<Probe> {
    let key = exe.display().to_string().to_lowercase();
    if let Some(cached) = probe_cache_get(&key) {
        return cached;
    }
    let text = run_java(exe, &["-version"]).await?;
    let (version, major) = parse_java_version(&text)?;
    let arch = run_java(exe, &["-XshowSettings:properties", "-version"])
        .await
        .and_then(|settings| parse_java_arch(&settings))
        .unwrap_or_else(|| "x64".to_string());
    let vendor = detect_vendor(&key, &text);
    let probe = Probe {
        version,
        major,
        arch,
        vendor,
    };
    probe_cache_put(&key, Some(probe.clone()));
    Some(probe)
}

// ------------------------------------------------------------------ 候选目录收集

/// 一个候选 JDK（目录 + 可执行文件 + 来源标签）。
#[derive(Debug, Clone)]
struct Candidate {
    home: PathBuf,
    exe: PathBuf,
    source: &'static str,
}

fn push_home(list: &mut Vec<Candidate>, home: PathBuf, source: &'static str) {
    if let Some(exe) = java_exe_in(&home) {
        list.push(Candidate { home, exe, source });
    }
}

/// .mc\runtimes\<dir>（也认多套一层的 zip 根目录）里的 JDK。
fn runtime_candidates(paths: &Paths) -> Vec<Candidate> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(&paths.runtimes) else {
        return found;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        push_home(&mut found, dir.clone(), "启动器内置");
        if let Ok(inner) = std::fs::read_dir(&dir) {
            for child in inner.filter_map(|child| child.ok()) {
                let child_dir = child.path();
                if child_dir.is_dir() {
                    push_home(&mut found, child_dir, "启动器内置");
                }
            }
        }
    }
    found
}

/// 常见安装目录（含 BellSoft Liberica / Eclipse Adoptium / Zulu）。
fn common_install_candidates() -> Vec<Candidate> {
    let roots = [
        r"C:\Program Files\BellSoft",
        r"C:\Program Files (x86)\BellSoft",
        r"C:\Program Files\Eclipse Adoptium",
        r"C:\Program Files\Eclipse Foundation",
        r"C:\Program Files\Zulu",
        r"C:\Program Files (x86)\Zulu",
        r"C:\Program Files\Java",
        r"C:\Program Files (x86)\Java",
        r"C:\Program Files\Microsoft",
        r"C:\Program Files\Amazon Corretto",
        r"C:\Program Files\Temurin",
        r"C:\Program Files\AdoptOpenJDK",
    ];
    let mut found = Vec::new();
    for root in roots {
        let root = Path::new(root);
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            push_home(&mut found, dir.clone(), "系统");
            // Liberica / Adoptium 有时多套一层（<vendor>\<version>\bin）
            if let Ok(inner) = std::fs::read_dir(&dir) {
                for child in inner.filter_map(|child| child.ok()) {
                    let child_dir = child.path();
                    if child_dir.is_dir() {
                        push_home(&mut found, child_dir, "系统");
                    }
                }
            }
        }
    }
    found
}

/// PATH 与 JAVA_HOME 里的 JDK。
fn path_candidates() -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut push_exe = |exe: PathBuf| {
        if let Some(home) = jdk_home_of(&exe) {
            push_home(&mut found, home, "系统");
        }
    };
    if let Ok(path) = std::env::var("PATH") {
        for entry in std::env::split_paths(&path) {
            for name in ["javaw.exe", "java.exe"] {
                let candidate = entry.join(name);
                if candidate.is_file() {
                    push_exe(candidate);
                }
            }
        }
    }
    if let Ok(home) = std::env::var("JAVA_HOME") {
        push_home(&mut found, PathBuf::from(home), "系统");
    }
    found
}

/// 注册表 JavaSoft（HKLM 与 WOW6432Node）。
async fn registry_candidates() -> Vec<Candidate> {
    #[cfg(windows)]
    {
        let keys = [
            r"HKLM\SOFTWARE\JavaSoft\JDK",
            r"HKLM\SOFTWARE\JavaSoft\JRE",
            r"HKLM\SOFTWARE\JavaSoft\Java Development Kit",
            r"HKLM\SOFTWARE\JavaSoft\Java Runtime Environment",
            r"HKLM\SOFTWARE\WOW6432Node\JavaSoft\JDK",
            r"HKLM\SOFTWARE\WOW6432Node\JavaSoft\JRE",
            r"HKLM\SOFTWARE\WOW6432Node\JavaSoft\Java Development Kit",
        ];
        let mut found = Vec::new();
        for key in keys {
            let mut cmd = tokio::process::Command::new("reg");
            cmd.args(["query", key, "/s", "/v", "JavaHome"]);
            crate::util::hidden_async(&mut cmd);
            let Ok(output) = cmd.output().await else { continue };
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            for home in parse_reg_query_java_homes(&text) {
                push_home(&mut found, PathBuf::from(home), "注册表");
            }
        }
        found
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// 设置里手动指定的 Java：可以是 javaw.exe，也可以是 JDK 目录。
fn preferred_candidate(preferred_executable: &str) -> Option<Candidate> {
    let trimmed = preferred_executable.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.is_file() {
        let home = jdk_home_of(&path)?;
        return Some(Candidate {
            home,
            exe: path,
            source: "手动添加",
        });
    }
    java_exe_in(&path).map(|exe| Candidate {
        home: path,
        exe,
        source: "手动添加",
    })
}

/// java_list()：按 JDK 目录去重，路径统一到 bin\javaw.exe，主版本降序。
pub async fn list(paths: &Paths, preferred_executable: &str) -> Vec<JavaRuntime> {
    let mut candidates: Vec<Candidate> = Vec::new();
    let preferred = preferred_candidate(preferred_executable);
    let preferred_home = preferred.as_ref().map(|candidate| candidate.home.clone());
    candidates.extend(preferred);
    candidates.extend(runtime_candidates(paths));
    candidates.extend(registry_candidates().await);
    candidates.extend(common_install_candidates());
    candidates.extend(path_candidates());

    let mut runtimes: Vec<JavaRuntime> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for candidate in candidates {
        let key = home_key(&candidate.home);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        tracing::debug!(
            source = candidate.source,
            home = %candidate.home.display(),
            "发现 Java 候选"
        );
        let Some(probe) = probe(&candidate.exe).await else {
            continue;
        };
        runtimes.push(JavaRuntime {
            path: candidate.home.display().to_string(),
            java_exe: candidate.exe.display().to_string(),
            major: probe.major,
            version: probe.version,
            vendor: probe.vendor,
            arch: probe.arch,
            is_default: false,
        });
    }
    // 同一 JDK 里嵌的 jre 子目录不算独立 JDK：父目录已经在列表里就丢掉子目录
    let keys: Vec<String> = runtimes
        .iter()
        .map(|runtime| home_key(Path::new(&runtime.path)))
        .collect();
    let mut keep = vec![true; runtimes.len()];
    for outer in 0..keys.len() {
        for inner in 0..keys.len() {
            if outer != inner && keys[inner].starts_with(&format!("{}\\", keys[outer])) {
                keep[inner] = false;
            }
        }
    }
    let mut filtered = Vec::with_capacity(runtimes.len());
    for (index, runtime) in runtimes.into_iter().enumerate() {
        if keep[index] {
            filtered.push(runtime);
        }
    }
    let mut runtimes = filtered;

    runtimes.sort_by(|left, right| {
        right
            .major
            .cmp(&left.major)
            .then_with(|| right.version.cmp(&left.version))
            .then_with(|| home_key(Path::new(&left.path)).cmp(&home_key(Path::new(&right.path))))
    });
    let default_index = preferred_home
        .and_then(|home| {
            let key = home_key(&home);
            runtimes
                .iter()
                .position(|runtime| runtime.path.to_lowercase() == key)
        })
        .unwrap_or(0);
    if let Some(runtime) = runtimes.get_mut(default_index) {
        runtime.is_default = true;
    }
    runtimes
}

/// 启动前确保有满足要求的 Java：已有就用，没有就按最快源自动下载（日志口径统一在这里）。
pub async fn ensure_for_launch(
    client: reqwest::Client,
    paths: &Paths,
    settings: &crate::settings::Settings,
    required: u32,
    origins: Vec<Origin>,
    task: &TaskHandle,
) -> Result<PathBuf> {
    let runtimes = list(paths, &settings.java_path).await;
    if let Some(found) = pick_for_major(&runtimes, required) {
        tracing::info!(
            path = %found.java_exe,
            major = found.major,
            vendor = %found.vendor,
            "使用已安装的 Java"
        );
        return Ok(PathBuf::from(&found.java_exe));
    }
    if !settings.auto_download_java {
        tracing::warn!(required, "缺少 Java 且自动下载已关闭");
        return Err(missing_java_error(required, "autoDownloadJava 已关闭"));
    }
    tracing::info!(required, "实例缺少 Java，开始自动下载（挑最快源）");
    match auto_install(client, paths, required, origins, task).await {
        Ok((runtime, source)) => {
            let name = source_display_name(&source);
            tracing::info!(
                "实例缺少 Java {}，已自动下载 {} 的 Java {}",
                required,
                name,
                required
            );
            Ok(PathBuf::from(&runtime.java_exe))
        }
        Err(AppError::Cancelled) => Err(AppError::Cancelled),
        Err(err) => Err(missing_java_error(required, &err.to_string())),
    }
}

/// 自动下载失败时的可读错误（各入口共用同一句话）。
pub fn missing_java_error(required: u32, reason: &str) -> AppError {
    AppError::Other(format!(
        "缺少 Java {required} 且自动下载失败：{reason}，可在 设置 → Java 手动安装"
    ))
}

/// 下载源显示名（日志用）。
pub fn source_display_name(id: &str) -> String {
    sources_table()
        .into_iter()
        .find(|source| source.id == id)
        .map(|source| source.name)
        .unwrap_or_else(|| id.to_string())
}

// ------------------------------------------------------------------ 元数据端点

/// BellSoft Liberica 元数据（实测 200；注意 arch=x86 就是 amd64，
/// 传 x64 会被拒 400；os/bundle-type/package-type 用短横线参数名）。
pub fn bellsoft_api_url(major: u32) -> String {
    format!(
        "https://api.bell-sw.com/v1/liberica/releases?version-feature={major}&os=windows&arch=x86&bundle-type=jdk&package-type=zip"
    )
}

/// Eclipse Adoptium 最新资产（实测 200）。
pub fn adoptium_api_url(major: u32) -> String {
    format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?os=windows&architecture=x64&image_type=jdk&vendor=eclipse"
    )
}

/// 清华 TUNA 的 Adoptium 目录列表（实测 200，Apache 风格 HTML）。
pub fn tuna_list_url(major: u32) -> String {
    format!("https://mirrors.tuna.tsinghua.edu.cn/Adoptium/{major}/jdk/x64/windows/")
}

/// Azul Zulu 元数据（实测 200；返回 {value:[...],Count:n}）。
pub fn azul_api_url(major: u32) -> String {
    format!(
        "https://api.azul.com/metadata/v1/zulu/packages/?java_version={major}&os=windows&arch=x64&java_package_type=jdk&archive_type=zip&release_status=ga&latest=true&availability_types=CA"
    )
}

/// 下载源表（java_sources）。
pub fn sources_table() -> Vec<JavaSourceInfo> {
    vec![
        JavaSourceInfo {
            id: "bellsoft".to_string(),
            name: "BellSoft Liberica".to_string(),
            description: "Liberica 官方元数据（api.bell-sw.com），GitHub Releases 直链，LTS 版本齐全".to_string(),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "adoptium".to_string(),
            name: "Eclipse Temurin".to_string(),
            description: "Adoptium 官方元数据（api.adoptium.net），GitHub Releases 直链".to_string(),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "tuna".to_string(),
            name: "清华 TUNA 镜像 · Temurin".to_string(),
            description: "mirrors.tuna.tsinghua.edu.cn 目录镜像，国内速度最快".to_string(),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "azul".to_string(),
            name: "Azul Zulu".to_string(),
            description: "Zulu 官方元数据（api.azul.com）+ cdn.azul.com 直链".to_string(),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "oracle".to_string(),
            name: "Oracle JDK".to_string(),
            description: format!("需要接受 Oracle 许可，只能手动下载：{ORACLE_PAGE}"),
            requires_license: true,
        },
    ]
}

/// 该源用来测速的地址。
pub fn source_probe_url(source: &str, major: u32) -> String {
    match source {
        "bellsoft" => bellsoft_api_url(major),
        "adoptium" => adoptium_api_url(major),
        "tuna" => tuna_list_url(major),
        "azul" => azul_api_url(major),
        _ => ORACLE_PAGE.to_string(),
    }
}

fn value_as_u64(value: &serde_json::Value, key: &str) -> u64 {
    value.get(key).and_then(|item| item.as_u64()).unwrap_or(0)
}

/// BellSoft JSON → 版本列表。
pub fn parse_bellsoft(json: &str, major: u32) -> Vec<JavaVersionInfo> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut out: Vec<JavaVersionInfo> = Vec::new();
    for item in items {
        let arch = item.get("architecture").and_then(|v| v.as_str()).unwrap_or("");
        let bundle = item.get("bundleType").and_then(|v| v.as_str()).unwrap_or("");
        let package = item.get("packageType").and_then(|v| v.as_str()).unwrap_or("");
        let os = item.get("os").and_then(|v| v.as_str()).unwrap_or("");
        // BellSoft 的 arch=x86 同时包含 amd64 与 i586，靠 bitness 只留 64 位
        let bitness = item.get("bitness").and_then(|v| v.as_u64()).unwrap_or(64);
        if arch != "x86" || bitness != 64 || bundle != "jdk" || package != "zip" || os != "windows" {
            continue;
        }
        let Some(version) = item.get("version").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(url) = item.get("downloadUrl").and_then(|v| v.as_str()) else {
            continue;
        };
        let lts = item.get("LTS").and_then(|v| v.as_bool()).unwrap_or(false);
        out.push(JavaVersionInfo {
            source: "bellsoft".to_string(),
            version: version.to_string(),
            major,
            lts,
            size_bytes: value_as_u64(item, "size"),
            url: url.to_string(),
        });
    }
    out.sort_by(|left, right| right.version.cmp(&left.version));
    out.dedup_by(|left, right| left.url == right.url);
    out
}

/// Adoptium JSON → 版本列表（binary.package 里是 zip 直链）。
pub fn parse_adoptium(json: &str, major: u32) -> Vec<JavaVersionInfo> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let package = item.pointer("/binary/package");
        let Some(url) = package.and_then(|p| p.get("link")).and_then(|v| v.as_str()) else {
            continue;
        };
        let name = package
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let version = item
            .get("release_name")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .or_else(|| {
                item.pointer("/version/semver")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| name.to_string());
        let lts = item
            .pointer("/version/semver")
            .and_then(|v| v.as_str())
            .map(|semver| semver.to_ascii_uppercase().contains("LTS"))
            .unwrap_or(false)
            || LTS_MAJORS.contains(&major);
        out.push(JavaVersionInfo {
            source: "adoptium".to_string(),
            version: version.trim_start_matches("jdk-").to_string(),
            major,
            lts,
            size_bytes: package.and_then(|p| p.get("size")).and_then(|v| v.as_u64()).unwrap_or(0),
            url: url.to_string(),
        });
    }
    out.sort_by(|left, right| right.version.cmp(&left.version));
    out
}

/// Azul 元数据 → 版本列表（只要普通 jdk，排除 crac/fx 变体）。
pub fn parse_azul(json: &str, major: u32) -> Vec<JavaVersionInfo> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    // 实测返回 {"value":[...],"Count":n}；文档里也有直接给数组的形态
    let items = value
        .get("value")
        .and_then(|v| v.as_array())
        .or_else(|| value.as_array());
    let Some(items) = items else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let Some(url) = item.get("download_url").and_then(|v| v.as_str()) else {
            continue;
        };
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        if name.contains("-crac-") || name.contains("-fx-") {
            continue;
        }
        let version = item
            .get("java_version")
            .and_then(|v| v.as_array())
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.as_u64().map(|value| value.to_string()))
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| name.to_string());
        out.push(JavaVersionInfo {
            source: "azul".to_string(),
            version,
            major,
            lts: LTS_MAJORS.contains(&major),
            size_bytes: 0,
            url: url.to_string(),
        });
    }
    out.sort_by(|left, right| right.version.cmp(&left.version));
    out.dedup_by(|left, right| left.url == right.url);
    out
}

/// "195.6 MiB" / "1023 KiB" → 字节数。
pub fn parse_size_text(text: &str) -> u64 {
    let trimmed = text.trim();
    let split = trimmed
        .find(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split);
    let Ok(value) = number.trim().replace(',', "").parse::<f64>() else {
        return 0;
    };
    let factor = match unit.trim().to_ascii_uppercase().as_str() {
        "KIB" | "KB" => 1024.0,
        "MIB" | "MB" => 1024.0 * 1024.0,
        "GIB" | "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (value * factor) as u64
}

/// 从 TUNA 目录列表 HTML 里解析 zip（文件名带版本号，旁边有 size 列）。
pub fn parse_tuna_listing(html: &str, base: &str, major: u32) -> Vec<JavaVersionInfo> {
    let base = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(position) = rest.find("href=\"") {
        let after = &rest[position + "href=\"".len()..];
        let Some(end) = after.find('"') else { break };
        let href = &after[..end];
        rest = &after[end..];
        if !href.ends_with(".zip") || href.contains("sha256") {
            continue;
        }
        // href 后面紧跟 <td class="size">195.6 MiB</td>
        let size = rest
            .find("class=\"size\">")
            .and_then(|index| {
                let tail = &rest[index + "class=\"size\">".len()..];
                tail.find('<').map(|end| &tail[..end])
            })
            .map(parse_size_text)
            .unwrap_or(0);
        let stem = href.trim_end_matches(".zip");
        let version = stem
            .split("hotspot_")
            .nth(1)
            .map(|tail| match tail.rfind('_') {
                Some(index) => format!("{}+{}", &tail[..index], &tail[index + 1..]),
                None => tail.to_string(),
            })
            .unwrap_or_else(|| stem.to_string());
        out.push(JavaVersionInfo {
            source: "tuna".to_string(),
            version,
            major,
            lts: LTS_MAJORS.contains(&major),
            size_bytes: size,
            url: format!("{base}{href}"),
        });
    }
    out.sort_by(|left, right| right.version.cmp(&left.version));
    out.dedup_by(|left, right| left.url == right.url);
    out
}

fn parse_versions(source: &str, body: &str, major: u32) -> Vec<JavaVersionInfo> {
    match source {
        "bellsoft" => parse_bellsoft(body, major),
        "adoptium" => parse_adoptium(body, major),
        "tuna" => parse_tuna_listing(body, &tuna_list_url(major), major),
        "azul" => parse_azul(body, major),
        _ => Vec::new(),
    }
}

/// 单源测速：PROBE_SAMPLES 次 HEAD，8 秒超时，取中位数。
pub async fn probe_source(client: &reqwest::Client, source: &str) -> JavaSourceProbe {
    let url = source_probe_url(source, 21);
    let mut samples: Vec<u64> = Vec::new();
    let mut last_error: Option<String> = None;
    for _ in 0..PROBE_SAMPLES {
        let started = Instant::now();
        match tokio::time::timeout(PROBE_TIMEOUT, client.head(&url).send()).await {
            Ok(Ok(response)) if response.status().is_success() => {
                samples.push(started.elapsed().as_millis() as u64);
            }
            Ok(Ok(response)) => last_error = Some(format!("HTTP {}", response.status().as_u16())),
            Ok(Err(err)) => last_error = Some(err.to_string()),
            Err(_) => last_error = Some(format!("超时（{} 秒）", PROBE_TIMEOUT.as_secs())),
        }
    }
    if samples.is_empty() {
        return JavaSourceProbe {
            source_id: source.to_string(),
            ok: false,
            latency_ms: 0,
            error: Some(last_error.unwrap_or_else(|| "没有响应".to_string())),
        };
    }
    JavaSourceProbe {
        source_id: source.to_string(),
        ok: true,
        latency_ms: sources::median_of(samples),
        error: None,
    }
}

/// 全部源并发测速（结果顺序固定为 sources_table 的顺序）。
pub async fn probe_sources(client: &reqwest::Client) -> Vec<JavaSourceProbe> {
    let mut handles = Vec::new();
    for source in ["bellsoft", "adoptium", "tuna", "azul", "oracle"] {
        let client = client.clone();
        handles.push(tokio::spawn(async move { probe_source(&client, source).await }));
    }
    let mut out = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(probe) => out.push(probe),
            Err(err) => tracing::warn!(%err, "Java 源测速任务失败"),
        }
    }
    out
}

/// 挑最快且可下载的源。
pub fn pick_fastest_source(probes: &[JavaSourceProbe]) -> Option<String> {
    probes
        .iter()
        .filter(|probe| probe.ok && INSTALLABLE_SOURCES.contains(&probe.source_id.as_str()))
        .min_by_key(|probe| probe.latency_ms)
        .map(|probe| probe.source_id.clone())
}

async fn fetch_versions(
    client: &reqwest::Client,
    source: &str,
    major: u32,
) -> Result<Vec<JavaVersionInfo>> {
    if !INSTALLABLE_SOURCES.contains(&source) {
        // Oracle 之类：只给下载页，不给直链
        return Ok(Vec::new());
    }
    let url = source_probe_url(source, major);
    let response = tokio::time::timeout(Duration::from_secs(20), client.get(&url).send())
        .await
        .map_err(|_| AppError::Net(format!("{source} 元数据请求超时")))??
        .error_for_status()?;
    let body = response.text().await?;
    Ok(parse_versions(source, &body, major))
}

/// java_versions()：source 为空时并发问所有可下载源。
pub async fn versions(
    client: &reqwest::Client,
    source: Option<&str>,
    major: Option<u32>,
) -> Vec<JavaVersionInfo> {
    let major = major.filter(|value| *value >= 8 && *value <= 64).unwrap_or(21);
    match source.map(str::trim).filter(|value| !value.is_empty()) {
        Some(source) => match fetch_versions(client, source, major).await {
            Ok(items) => items,
            Err(err) => {
                tracing::warn!(%err, source, "获取 Java 版本列表失败");
                Vec::new()
            }
        },
        None => {
            let mut handles = Vec::new();
            for source in INSTALLABLE_SOURCES {
                let client = client.clone();
                handles.push(tokio::spawn(async move {
                    (source, fetch_versions(&client, source, major).await)
                }));
            }
            let mut out = Vec::new();
            for handle in handles {
                match handle.await {
                    Ok((_source, Ok(mut items))) => out.append(&mut items),
                    Ok((source, Err(err))) => {
                        tracing::warn!(%err, source, "获取 Java 版本列表失败")
                    }
                    Err(err) => tracing::warn!(%err, "获取 Java 版本列表任务失败"),
                }
            }
            out
        }
    }
}

/// 源 → 运行时目录里的 vendor 段。
pub fn vendor_dir_name(source: &str) -> &'static str {
    match source {
        "bellsoft" => "liberica",
        "adoptium" | "tuna" => "temurin",
        "azul" => "zulu",
        _ => VENDOR,
    }
}

/// 版本号压成目录名安全片段（21.0.12.1+1 → 21.0.12.1_1）。
pub fn sanitize_version_part(version: &str) -> String {
    version
        .chars()
        .map(|character| match character {
            '+' | ' ' | '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect()
}

/// 运行时目录：.mc\runtimes\<vendor>-<major>-<version>
pub fn runtime_dir(paths: &Paths, source: &str, major: u32, version: &str) -> PathBuf {
    paths.runtimes.join(format!(
        "{}-{}-{}",
        vendor_dir_name(source),
        major,
        sanitize_version_part(version)
    ))
}

fn write_stamp(dir: &Path, stamp: &JavaStamp) -> Result<()> {
    let path = dir.join(STAMP_FILE);
    let text = serde_json::to_string_pretty(stamp)?;
    std::fs::write(&path, text)?;
    Ok(())
}

/// 读运行时目录里的安装信息（没有就 None）。
pub fn read_stamp(dir: &Path) -> Option<JavaStamp> {
    let text = std::fs::read_to_string(dir.join(STAMP_FILE)).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// java_install()：下载 → 解压 → 写 .gugu-java.json → 探测。
pub async fn install(
    client: reqwest::Client,
    paths: &Paths,
    source: &str,
    version: &str,
    major: u32,
    origins: Vec<Origin>,
    task: &TaskHandle,
) -> Result<JavaRuntime> {
    if !INSTALLABLE_SOURCES.contains(&source) {
        return Err(AppError::Invalid(format!(
            "该下载源只能手动安装（{source}），请到 {ORACLE_PAGE} 下载"
        )));
    }
    if major < 8 || major > 64 {
        return Err(AppError::Invalid(format!("Java 主版本号不合法：{major}")));
    }
    task.progress_forced(Phase::Downloading, "正在查询下载地址", 0, 0);
    let versions = fetch_versions(&client, source, major).await?;
    let picked = versions
        .iter()
        .find(|item| item.version == version)
        .or_else(|| versions.first())
        .ok_or_else(|| {
            AppError::Other(format!("{source} 没有 Java {major} 的下载地址，换一个源试试"))
        })?;
    let url = picked.url.clone();
    let picked_version = picked.version.clone();

    let target_dir = runtime_dir(paths, source, major, &picked_version);
    let zip_path = paths.cache.join(format!(
        "{}-{}-{}.zip",
        vendor_dir_name(source),
        major,
        sanitize_version_part(&picked_version)
    ));
    let fetcher = Fetcher::new(client, origins, task.cancel_flag(), 1);
    task.progress_forced(
        Phase::Downloading,
        &format!("正在下载 Java {major}（{source}）"),
        0,
        0,
    );
    match fetcher.fetch(&url, &zip_path, None, None).await {
        Ok(outcome) => tracing::info!(source, url = %url, bytes = outcome.bytes, "Java 安装包已下载"),
        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
        Err(err) => return Err(err),
    }

    task.progress_forced(Phase::Extracting, "正在解压 Java 运行时", 0, 0);
    if target_dir.is_dir() {
        std::fs::remove_dir_all(&target_dir)?;
    }
    std::fs::create_dir_all(&target_dir)?;
    extract_zip_strip_root(&zip_path, &target_dir)?;
    let _ = std::fs::remove_file(&zip_path);

    let exe = java_exe_in(&target_dir).ok_or_else(|| {
        AppError::Other(format!(
            "Java 解压完成但没找到 java 可执行文件（{}）",
            target_dir.display()
        ))
    })?;
    let probe = probe(&exe).await.ok_or_else(|| {
        AppError::Other(format!(
            "Java 已解压但无法运行（{}），可能是下载不完整",
            exe.display()
        ))
    })?;
    write_stamp(
        &target_dir,
        &JavaStamp {
            source: source.to_string(),
            vendor: probe.vendor.clone(),
            version: probe.version.clone(),
            major: probe.major,
            url,
            installed_at: crate::util::now_unix(),
        },
    )?;
    task.progress_forced(Phase::Finished, "Java 安装完成", 1, 1);
    Ok(JavaRuntime {
        path: target_dir.display().to_string(),
        java_exe: exe.display().to_string(),
        major: probe.major,
        version: probe.version,
        vendor: probe.vendor,
        arch: probe.arch,
        is_default: true,
    })
}

/// 自动补 Java：挑最快源 → 装最新版本；失败换下一个可用源。
pub async fn auto_install(
    client: reqwest::Client,
    paths: &Paths,
    major: u32,
    origins: Vec<Origin>,
    task: &TaskHandle,
) -> Result<(JavaRuntime, String)> {
    let probes = probe_sources(&client).await;
    let mut candidates: Vec<String> = probes
        .iter()
        .filter(|probe| probe.ok && INSTALLABLE_SOURCES.contains(&probe.source_id.as_str()))
        .map(|probe| probe.source_id.clone())
        .collect();
    candidates.sort_by_key(|source| {
        probes
            .iter()
            .find(|probe| &probe.source_id == source)
            .map(|probe| probe.latency_ms)
            .unwrap_or(u64::MAX)
    });
    // 测速全军覆没时按固定顺序硬试
    if candidates.is_empty() {
        candidates = INSTALLABLE_SOURCES.iter().map(|source| source.to_string()).collect();
    }
    let mut failures: Vec<String> = Vec::new();
    for source in candidates {
        let task = task.clone();
        match install(client.clone(), paths, &source, "", major, origins.clone(), &task).await {
            Ok(runtime) => return Ok((runtime, source)),
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(err) => failures.push(format!("{source}: {err}")),
        }
    }
    Err(AppError::Net(format!(
        "Java {major} 自动下载失败：{}",
        failures.join("；")
    )))
}

/// 解压 zip 并去掉顶层目录（各家 zip 里都是一个顶层目录）。
pub fn extract_zip_strip_root(zip_path: &Path, dest: &Path) -> Result<usize> {
    let file = std::fs::File::open(zip_path)
        .map_err(|err| AppError::Other(format!("打开压缩包失败（{}）：{err}", zip_path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| AppError::Other(format!("压缩包损坏（{}）：{err}", zip_path.display())))?;

    let mut names: Vec<String> = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| AppError::Other(format!("读取压缩包条目失败：{err}")))?;
        names.push(entry.name().to_string());
    }
    let strip = common_root(&names);

    let mut written = 0usize;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| AppError::Other(format!("读取压缩包条目失败：{err}")))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let relative = match &strip {
            Some(root) => match name.strip_prefix(root.as_str()) {
                Some(rest) => rest.to_string(),
                None => continue,
            },
            None => name.clone(),
        };
        if relative.trim().is_empty() {
            continue;
        }
        let target = dest.join(relative.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
        written += 1;
    }
    if written == 0 {
        return Err(AppError::Other("压缩包里没有任何文件".to_string()));
    }
    Ok(written)
}

/// 所有条目是否共享同一个顶层目录；是则返回 "根目录/" 形式的前缀。
fn common_root(names: &[String]) -> Option<String> {
    let mut root: Option<String> = None;
    for name in names {
        let first = name.split('/').next()?.to_string();
        if !name.contains('/') {
            return None;
        }
        match &root {
            None => root = Some(first),
            Some(existing) if *existing == first => {}
            Some(_) => return None,
        }
    }
    root.map(|root| format!("{root}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_handles_modern_and_legacy() {
        let modern = "openjdk version \"17.0.8\" 2023-07-18\nOpenJDK Runtime Environment Temurin-17.0.8+7 (build 17.0.8+7)\n";
        assert_eq!(parse_java_version(modern), Some(("17.0.8".to_string(), 17)));
        let legacy = "java version \"1.8.0_381\"\nJava(TM) SE Runtime Environment (build 1.8.0_381-b09)\n";
        assert_eq!(parse_java_version(legacy), Some(("1.8.0_381".to_string(), 8)));
        assert_eq!(parse_java_version("命令找不到"), None);
    }

    #[test]
    fn parse_arch_reads_os_arch_and_bitness() {
        assert_eq!(parse_java_arch("    os.arch = amd64\n"), Some("x64".to_string()));
        assert_eq!(parse_java_arch("    os.arch = aarch64\n"), Some("arm64".to_string()));
        assert_eq!(parse_java_arch("    sun.arch.data.model = 32\n"), Some("x86".to_string()));
        assert_eq!(parse_java_arch("nothing here"), None);
    }

    #[test]
    fn detect_vendor_prefers_specific_brands() {
        assert_eq!(
            detect_vendor(
                "c:\\program files\\bellsoft\\libericajdk-21",
                "openjdk version \"21.0.1\"\nOpenJDK Runtime Environment (build 21.0.1+12)\nOpenJDK 64-Bit Server VM **Liberica**"
            ),
            "Liberica"
        );
        assert_eq!(
            detect_vendor("c:\\program files\\eclipse adoptium\\jdk-17", "Eclipse Temurin-17.0.9"),
            "Temurin"
        );
        assert_eq!(detect_vendor("c:\\program files\\zulu\\zulu-21", "Zulu21.30"), "Zulu");
        assert_eq!(detect_vendor("c:\\jdks\\jdk-17", "Microsoft build 17.0.9"), "Microsoft");
        assert_eq!(detect_vendor("c:\\jdk", "java version \"1.8.0_381\"\nJava(TM) SE Runtime Environment"), "Oracle");
        assert_eq!(detect_vendor("c:\\jdk", "openjdk version \"21\""), "OpenJDK");
        assert_eq!(detect_vendor("c:\\somewhere", "whatever"), "未知");
    }

    #[test]
    fn registry_output_extracts_homes() {
        let output = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\JavaSoft\\JDK\\1.8\r\n    JavaHome    REG_SZ    C:\\Program Files\\Java\\jdk1.8.0_381\r\n\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\JavaSoft\\JDK\\17\r\n    JavaHome    REG_SZ    C:\\Program Files\\Java\\jdk-17.0.9\r\n";
        let homes = parse_reg_query_java_homes(output);
        assert_eq!(homes.len(), 2);
        assert_eq!(homes[1], "C:\\Program Files\\Java\\jdk-17.0.9");
        assert!(parse_reg_query_java_homes("错误: 系统找不到指定的注册表项或值").is_empty());
    }

    #[test]
    fn exe_candidates_prefer_javaw_then_java() {
        let candidates = java_exe_candidates(Path::new("C:\\jdk"));
        assert_eq!(candidates[0], Path::new("C:\\jdk\\bin\\javaw.exe"));
        assert_eq!(candidates[1], Path::new("C:\\jdk\\bin\\java.exe"));
    }

    #[test]
    fn jdk_home_is_the_parent_of_bin() {
        assert_eq!(
            jdk_home_of(Path::new("C:\\jdk-21\\bin\\javaw.exe")),
            Some(PathBuf::from("C:\\jdk-21"))
        );
        assert_eq!(jdk_home_of(Path::new("C:\\jdk-21\\javaw.exe")), None);
    }

    #[test]
    fn infer_major_follows_mojang_requirements() {
        assert_eq!(infer_major_for_mc_version("1.20.1"), 17);
        assert_eq!(infer_major_for_mc_version("1.20.4"), 17);
        assert_eq!(infer_major_for_mc_version("1.20.5"), 21);
        assert_eq!(infer_major_for_mc_version("1.20.6"), 21);
        assert_eq!(infer_major_for_mc_version("1.21"), 21);
        assert_eq!(infer_major_for_mc_version("1.21.4"), 21);
        assert_eq!(infer_major_for_mc_version("1.18.2"), 17);
        assert_eq!(infer_major_for_mc_version("1.17.1"), 16);
        assert_eq!(infer_major_for_mc_version("1.16.5"), 8);
        assert_eq!(infer_major_for_mc_version("1.12.2"), 8);
        assert_eq!(infer_major_for_mc_version("25w02a"), 17);
    }

    #[test]
    fn bellsoft_json_is_filtered_to_windows_amd64_jdk_zip() {
        let json = r#"[{"version":"21.0.12","downloadUrl":"https://x/a.zip","size":195000000,"LTS":true,"architecture":"x86","bundleType":"jdk","packageType":"zip","os":"windows"},{"version":"21.0.11","downloadUrl":"https://x/b.zip","size":1,"LTS":true,"architecture":"x86","bundleType":"jdk","packageType":"zip","os":"windows"},{"version":"21.0.10","downloadUrl":"https://x/c.zip","size":1,"LTS":true,"architecture":"arm","bundleType":"jdk","packageType":"zip","os":"windows"},{"version":"21.0.9","downloadUrl":"https://x/d.deb","size":1,"LTS":true,"architecture":"x86","bundleType":"jdk","packageType":"deb","os":"windows"},{"version":"21.0.8","downloadUrl":"https://x/e.zip","size":1,"LTS":true,"architecture":"x86","bitness":32,"bundleType":"jdk","packageType":"zip","os":"windows"}]"#;
        let items = parse_bellsoft(json, 21);
        assert_eq!(items.len(), 2, "只要 windows/amd64/jdk/zip：{:?}", items);
        assert_eq!(items[0].version, "21.0.12");
        assert_eq!(items[0].size_bytes, 195_000_000);
        assert!(items[0].lts);
        assert_eq!(items[0].major, 21);
        assert!(parse_bellsoft("not json", 21).is_empty());
    }

    #[test]
    fn adoptium_json_maps_package_link_and_size() {
        let json = r#"[{"release_name":"jdk-21.0.12.1+1","version":{"semver":"21.0.12+101.0.LTS"},"binary":{"package":{"name":"OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip","link":"https://github.com/x.zip","size":205073461}}}]"#;
        let items = parse_adoptium(json, 21);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].version, "21.0.12.1+1");
        assert_eq!(items[0].size_bytes, 205_073_461);
        assert!(items[0].lts);
        assert_eq!(items[0].url, "https://github.com/x.zip");
    }

    #[test]
    fn azul_json_accepts_wrapped_and_plain_arrays() {
        let wrapped = r#"{"value":[{"download_url":"https://cdn/zulu21.zip","name":"zulu21.54.107-ca-jdk21.0.13-win_x64.zip","java_version":[21,0,13]},{"download_url":"https://cdn/crac.zip","name":"zulu21.52.203-ca-crac-jdk21.0.12.1-win_x64.zip","java_version":[21,0,12,1]}],"Count":2}"#;
        let items = parse_azul(wrapped, 21);
        assert_eq!(items.len(), 1, "crac 变体要排除");
        assert_eq!(items[0].version, "21.0.13");
        assert_eq!(items[0].url, "https://cdn/zulu21.zip");

        let plain = r#"[{"download_url":"https://cdn/zulu17.zip","name":"zulu17.zip","java_version":[17,0,9]}]"#;
        assert_eq!(parse_azul(plain, 17)[0].version, "17.0.9");
    }

    #[test]
    fn tuna_listing_reads_version_and_size() {
        let html = r#"<tr><td class="link"><a href="OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip" title="x">OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip</a></td><td class="size">195.6 MiB</td><td class="date">21 Aug 2026</td></tr>
<tr><td class="link"><a href="OpenJDK21U-jdk_x64_windows_hotspot_21.0.11_10.zip" title="y">y</a></td><td class="size">195.1 MiB</td></tr>"#;
        let items = parse_tuna_listing(html, "https://mirrors.tuna.tsinghua.edu.cn/Adoptium/21/jdk/x64/windows/", 21);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].version, "21.0.12.1+1");
        assert_eq!(items[0].size_bytes, parse_size_text("195.6 MiB"));
        assert!(items[0].size_bytes > 200_000_000);
        assert!(items[0].url.ends_with("OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip"));
        assert!(items[0].url.starts_with("https://mirrors.tuna.tsinghua.edu.cn/"));
    }

    #[test]
    fn size_text_units() {
        assert_eq!(parse_size_text("1 KiB"), 1024);
        assert_eq!(parse_size_text("2 MiB"), 2 * 1024 * 1024);
        assert_eq!(parse_size_text("garbage"), 0);
    }

    #[test]
    fn sources_table_lists_five_with_oracle_licensed() {
        let table = sources_table();
        let ids: Vec<&str> = table.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["bellsoft", "adoptium", "tuna", "azul", "oracle"]);
        let oracle = table.iter().find(|item| item.id == "oracle").unwrap();
        assert!(oracle.requires_license);
        assert!(oracle.description.contains("oracle.com"));
        assert!(table.iter().filter(|item| !item.requires_license).count() == 4);
    }

    #[test]
    fn probe_urls_point_at_measured_endpoints() {
        assert!(bellsoft_api_url(21).starts_with("https://api.bell-sw.com/v1/liberica/releases?"));
        assert!(bellsoft_api_url(21).contains("arch=x86"), "BellSoft 的 amd64 是 arch=x86");
        assert!(adoptium_api_url(21).starts_with("https://api.adoptium.net/v3/assets/latest/21/"));
        assert_eq!(
            tuna_list_url(17),
            "https://mirrors.tuna.tsinghua.edu.cn/Adoptium/17/jdk/x64/windows/"
        );
        assert!(azul_api_url(21).starts_with("https://api.azul.com/metadata/v1/zulu/packages/"));
        assert_eq!(source_probe_url("oracle", 21), ORACLE_PAGE);
        assert!(source_probe_url("tuna", 11).contains("Adoptium/11/"));
    }

    #[test]
    fn fastest_source_ignores_failures_and_oracle() {
        let probes = vec![
            JavaSourceProbe { source_id: "bellsoft".into(), ok: true, latency_ms: 900, error: None },
            JavaSourceProbe { source_id: "tuna".into(), ok: true, latency_ms: 120, error: None },
            JavaSourceProbe { source_id: "oracle".into(), ok: true, latency_ms: 5, error: None },
            JavaSourceProbe { source_id: "azul".into(), ok: false, latency_ms: 0, error: Some("超时".into()) },
        ];
        assert_eq!(pick_fastest_source(&probes).as_deref(), Some("tuna"));
        assert!(pick_fastest_source(&[]).is_none());
    }

    #[test]
    fn runtime_dir_and_version_part_are_filesystem_safe() {
        let paths = Paths::from_root("C:\\tmp\\.mc");
        let dir = runtime_dir(&paths, "bellsoft", 21, "21.0.12.1+1");
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), "liberica-21-21.0.12.1_1");
        let dir = runtime_dir(&paths, "tuna", 17, "17.0.9+9");
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), "temurin-17-17.0.9_9");
        assert_eq!(sanitize_version_part("a/b:c"), "a_b_c");
    }

    #[test]
    fn stamp_roundtrip() {
        let dir = std::env::temp_dir().join(format!("gugu-java-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let stamp = JavaStamp {
            source: "bellsoft".into(),
            vendor: "Liberica".into(),
            version: "21.0.12".into(),
            major: 21,
            url: "https://x/a.zip".into(),
            installed_at: 1_700_000_000,
        };
        write_stamp(&dir, &stamp).unwrap();
        assert_eq!(read_stamp(&dir), Some(stamp));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pick_for_major_prefers_exact_then_lowest_above() {
        let make = |major: u32, exe: &str| JavaRuntime {
            path: exe.to_string(),
            java_exe: exe.to_string(),
            major,
            version: format!("{major}.0.1"),
            vendor: "Temurin".to_string(),
            arch: "x64".to_string(),
            is_default: false,
        };
        let runtimes = vec![make(21, "j21"), make(17, "j17"), make(8, "j8")];
        assert_eq!(pick_for_major(&runtimes, 17).map(|r| r.java_exe.clone()), Some("j17".to_string()));
        assert_eq!(pick_for_major(&runtimes, 16).map(|r| r.java_exe.clone()), Some("j17".to_string()));
        assert_eq!(pick_for_major(&runtimes, 8).map(|r| r.java_exe.clone()), Some("j8".to_string()));
        assert!(pick_for_major(&runtimes, 25).is_none());
    }

    #[test]
    fn runtime_candidates_dedupe_per_jdk_directory() {
        let root = std::env::temp_dir().join(format!("gugu-java-scan2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        // 同一个 JDK 里 javaw.exe 与 java.exe 都在：只应产生一条候选
        let dir = paths.runtimes.join("liberica-21-21.0.12");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join("javaw.exe"), b"fake").unwrap();
        std::fs::write(dir.join("bin").join("java.exe"), b"fake").unwrap();
        let found = runtime_candidates(&paths);
        assert_eq!(found.len(), 1);
        assert!(found[0].exe.ends_with("javaw.exe"));
        assert_eq!(found[0].home, dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn home_key_ignores_trailing_separator_and_case() {
        assert_eq!(
            home_key(Path::new("C:\\Program Files\\BellSoft\\LibericaJDK-8\\")),
            home_key(Path::new("c:\\program files\\bellsoft\\libericajdk-8"))
        );
        assert_ne!(
            home_key(Path::new("C:\\jdk-17")),
            home_key(Path::new("C:\\jdk-21"))
        );
    }

    #[test]
    fn nested_jre_is_not_a_second_jdk() {
        let keys = vec![
            home_key(Path::new("C:\\Program Files\\BellSoft\\LibericaJDK-8")),
            home_key(Path::new("C:\\Program Files\\BellSoft\\LibericaJDK-8\\jre")),
        ];
        assert!(keys[1].starts_with(&format!("{}\\", keys[0])));
    }

    #[test]
    fn common_root_detection() {
        assert_eq!(
            common_root(&["jdk-17/bin/java.exe".to_string(), "jdk-17/release".to_string()]),
            Some("jdk-17/".to_string())
        );
        assert_eq!(
            common_root(&["jdk-17/bin/java.exe".to_string(), "other/release".to_string()]),
            None
        );
        assert_eq!(common_root(&["file.txt".to_string()]), None);
    }

    #[test]
    fn extract_zip_strips_single_root_directory() {
        let dir = std::env::temp_dir().join(format!("gugu-java-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("jdk.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("jdk-17.0.9/bin/java.exe", options).unwrap();
            std::io::Write::write_all(&mut writer, b"fake").unwrap();
            writer.start_file("jdk-17.0.9/release", options).unwrap();
            std::io::Write::write_all(&mut writer, b"JAVA_VERSION=\"17.0.9\"").unwrap();
            writer.finish().unwrap();
        }
        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let written = extract_zip_strip_root(&zip_path, &dest).unwrap();
        assert_eq!(written, 2);
        assert!(dest.join("bin").join("java.exe").is_file());
        assert!(dest.join("release").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_survives_broken_executables_and_reports_one_per_jdk() {
        let root = std::env::temp_dir().join(format!("gugu-java-list2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        let broken = paths.runtimes.join("temurin-99").join("bin").join("java.exe");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, b"not an exe").unwrap();
        let runtimes = list(&paths, "").await;
        assert!(runtimes.iter().all(|runtime| !runtime.path.contains("temurin-99")));
        // 每条都必须指向 javaw.exe（系统上没装 Java 时列表为空，也不会 panic）
        for runtime in &runtimes {
            assert!(runtime.java_exe.to_lowercase().ends_with("javaw.exe") || runtime.java_exe.to_lowercase().ends_with("java.exe"));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn install_rejects_licensed_and_absurd_majors() {
        let root = std::env::temp_dir().join(format!("gugu-java-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        let store = crate::tasks::TaskStore::new();
        let task = store.create_with_prefix("java", "Java 21", "", "java:");
        let err = install(
            reqwest::Client::new(),
            &paths,
            "oracle",
            "",
            21,
            Vec::new(),
            &task,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("手动安装"));
        let err = install(
            reqwest::Client::new(),
            &paths,
            "bellsoft",
            "",
            3,
            Vec::new(),
            &task,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("主版本号"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
