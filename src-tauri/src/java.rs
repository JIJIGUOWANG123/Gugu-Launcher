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
/// 可自动下载的源（顺序＝UI 顺序）。
///
/// bellsoft 与 adoptium 官方（下载都走 github.com）国内太慢，已从可下载清单里删掉；
/// oracle 官方只给下载页（403 + OTN 许可），这里只保留能直接下 zip 的 6 家。
pub const INSTALLABLE_SOURCES: [&str; 6] = [
    "tuna",
    "lzu",
    "zulu-injdk",
    "oracle-injdk",
    "microsoft",
    "azul",
];
/// 只能手动安装的源（只给「前往下载页」）。
pub const MANUAL_SOURCES: [&str; 1] = ["oracle"];
/// Oracle 官方下载页（人工安装）。
pub const ORACLE_PAGE: &str = "https://www.oracle.com/java/technologies/downloads/";
/// Oracle JDK 的许可提示（写进来源描述与错误文案）。
pub const ORACLE_LICENSE_NOTE: &str = "Oracle JDK 受 OTN 许可约束，商用请自行确认";
/// Microsoft Build of OpenJDK 支持的 LTS 主版本（aka.ms 直链按主版本给）。
pub const MICROSOFT_MAJORS: [u32; 4] = [11, 17, 21, 25];
/// Microsoft 的下载页（版本说明写在 UI 里）。
pub const MICROSOFT_PAGE: &str = "https://learn.microsoft.com/zh-cn/java/openjdk/download";
/// injdk.cn 的 Zulu 镜像根（Caddy 目录列表，<major>/ 下按平台分文件）。
pub const INJDK_ZULU_ROOT: &str = "https://d10.injdk.cn/openjdk/zulu";
/// injdk.cn 的 Oracle JDK 镜像根（同上，文件名 *_windows-x64_bin.zip）。
pub const INJDK_ORACLE_ROOT: &str = "https://d10.injdk.cn/openjdk/oraclejdk";
/// 清华 TUNA 的 Adoptium 目录根（大写 Adoptium）。
pub const TUNA_ROOT: &str = "https://mirrors.tuna.tsinghua.edu.cn/Adoptium";
/// 兰州大学的 Adoptium 目录根（小写 adoptium）。
pub const LZU_ROOT: &str = "https://mirrors.lzu.edu.cn/adoptium";
/// 兰大的备用端口（实测 200，与 LZU_ROOT 同一份内容）。
pub const LZU_SPARE_ROOT: &str = "https://mirror.lzu.edu.cn:8080/adoptium";
/// Adoptium 目录镜像（文件名完全一致，只换根路径即可互备）。
pub const ADOPTIUM_MIRRORS: [&str; 3] = [TUNA_ROOT, LZU_ROOT, LZU_SPARE_ROOT];

/// 镜像的目录根（实测：TUNA 是 /Adoptium/，兰大是小写 /adoptium/）。
pub fn mirror_root(source: &str) -> Option<&'static str> {
    match source {
        "tuna" => Some(TUNA_ROOT),
        "lzu" => Some(LZU_ROOT),
        _ => None,
    }
}

/// 同文件名可互备的镜像根，按尝试顺序（含兰大 :8080 备用端口）。
pub fn mirror_roots(source: &str) -> Vec<&'static str> {
    match source {
        "tuna" => vec![TUNA_ROOT, LZU_ROOT, LZU_SPARE_ROOT],
        "lzu" => vec![LZU_ROOT, LZU_SPARE_ROOT, TUNA_ROOT],
        _ => Vec::new(),
    }
}
/// 常见 LTS 主版本。
pub const LTS_MAJORS: [u32; 5] = [8, 11, 17, 21, 25];
/// 下载 JDK 包时的分片连接数（单个 200MB 包单流很容易中途失败）。
pub const INSTALL_CONNECTIONS: usize = 4;

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

/// injdk.cn 的 Zulu 目录（Caddy 列表；文件名形如 zulu21.50.19-ca-jdk21.0.11-win_x64.zip）。
pub fn injdk_zulu_dir_url(major: u32) -> String {
    format!("{INJDK_ZULU_ROOT}/{major}/")
}

/// injdk.cn 的 Oracle JDK 目录（文件名形如 jdk-21_windows-x64_bin.zip）。
pub fn injdk_oracle_dir_url(major: u32) -> String {
    format!("{INJDK_ORACLE_ROOT}/{major}/")
}

/// Microsoft 的 aka.ms 直链（只覆盖 11/17/21/25，其余主版本返回 None）。
pub fn microsoft_zip_url(major: u32) -> Option<String> {
    if !MICROSOFT_MAJORS.contains(&major) {
        return None;
    }
    Some(format!(
        "https://aka.ms/download-jdk/microsoft-jdk-{major}-windows-x64.zip"
    ))
}

/// 清华 TUNA 的 Adoptium 目录列表（实测 200，Apache 风格 HTML）。
pub fn tuna_list_url(major: u32) -> String {
    format!("https://mirrors.tuna.tsinghua.edu.cn/Adoptium/{major}/jdk/x64/windows/")
}

/// 兰州大学镜像的 Adoptium 目录（实测 200，路径是小写 adoptium）。
pub fn lzu_list_url(major: u32) -> String {
    format!("https://mirrors.lzu.edu.cn/adoptium/{major}/jdk/x64/windows/")
}

/// 镜像目录地址（tuna / lzu 结构一致，只差根路径的字母大小写）。
pub fn mirror_list_url(source: &str, major: u32) -> String {
    match mirror_root(source) {
        Some(root) => format!("{root}/{major}/jdk/x64/windows/"),
        None => tuna_list_url(major),
    }
}

/// Azul Zulu 元数据（实测 200；返回 {value:[...],Count:n}）。
pub fn azul_api_url(major: u32) -> String {
    format!(
        "https://api.azul.com/metadata/v1/zulu/packages/?java_version={major}&os=windows&arch=x64&java_package_type=jdk&archive_type=zip&release_status=ga&latest=true&availability_types=CA"
    )
}

/// 下载源表（java_sources）：6 家可下载 + oracle 仅跳转。
pub fn sources_table() -> Vec<JavaSourceInfo> {
    vec![
        JavaSourceInfo {
            id: "tuna".to_string(),
            name: "清华 TUNA 镜像 · Temurin".to_string(),
            description: "mirrors.tuna.tsinghua.edu.cn/Adoptium，国内最快，可自动下载".to_string(),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "lzu".to_string(),
            name: "兰州大学镜像 · Temurin".to_string(),
            description:
                "mirrors.lzu.edu.cn/adoptium（备用 mirror.lzu.edu.cn:8080），与清华同结构可互备"
                    .to_string(),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "zulu-injdk".to_string(),
            name: "injdk.cn 镜像 · Zulu".to_string(),
            description: "d10.injdk.cn/openjdk/zulu，Caddy 目录列表，取 *-win_x64.zip".to_string(),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "oracle-injdk".to_string(),
            name: "injdk.cn 镜像 · Oracle JDK".to_string(),
            description: format!(
                "d10.injdk.cn/openjdk/oraclejdk，可直链下载；{ORACLE_LICENSE_NOTE}"
            ),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "microsoft".to_string(),
            name: "Microsoft Build of OpenJDK".to_string(),
            description: format!("aka.ms/download-jdk 直链，下载页 {MICROSOFT_PAGE}"),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "azul".to_string(),
            name: "Azul Zulu".to_string(),
            description: "api.azul.com 元数据 + cdn.azul.com 直链".to_string(),
            requires_license: false,
        },
        JavaSourceInfo {
            id: "oracle".to_string(),
            name: "Oracle 官方（需登录）".to_string(),
            description: format!("只能到 {ORACLE_PAGE} 手动下载；{ORACLE_LICENSE_NOTE}"),
            requires_license: true,
        },
    ]
}

/// 该源用来测速的地址。
pub fn source_probe_url(source: &str, major: u32) -> String {
    match source {
        "tuna" | "lzu" => mirror_list_url(source, major),
        "zulu-injdk" => injdk_zulu_dir_url(major),
        "oracle-injdk" => injdk_oracle_dir_url(major),
        // aka.ms 会 302 到带版本号的文件名，HEAD 跟着走就能测
        "microsoft" => microsoft_zip_url(major).unwrap_or_else(|| MICROSOFT_PAGE.to_string()),
        "azul" => azul_api_url(major),
        "oracle" => ORACLE_PAGE.to_string(),
        _ => tuna_list_url(major),
    }
}

/// Caddy 目录列表里要挑哪种 zip。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaddyKind {
    /// injdk 的 Zulu：只认 *-win_x64.zip
    Zulu,
    /// injdk 的 Oracle JDK：只认 *_windows-x64_bin.zip（老包是 -windows-x64.zip）
    Oracle,
}

impl CaddyKind {
    fn accepts(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        match self {
            CaddyKind::Zulu => lower.ends_with("-win_x64.zip"),
            CaddyKind::Oracle => {
                lower.ends_with("_windows-x64_bin.zip") || lower.ends_with("-windows-x64.zip")
            }
        }
    }

    /// zulu21.50.19-ca-jdk21.0.11-win_x64.zip → 21.0.11
    /// jdk-21_windows-x64_bin.zip → 21（老包 jdk-8u491-windows-x64.zip → 8u491）
    fn version_of(&self, name: &str) -> Option<String> {
        let lower = name.to_ascii_lowercase();
        let (marker, take_dot) = match self {
            CaddyKind::Zulu => ("jdk", true),
            CaddyKind::Oracle => ("jdk-", false),
        };
        let start = lower.find(marker)? + marker.len();
        let version: String = name[start..]
            .chars()
            .take_while(|character| {
                character.is_ascii_digit() || (take_dot && *character == '.') || (!take_dot && character.is_ascii_alphabetic())
            })
            .collect();
        if version.is_empty() {
            None
        } else {
            Some(version)
        }
    }
}

/// Caddy 目录列表 → 版本列表。
///
/// 实测结构（d10.injdk.cn）：
/// `<a href="./xxx-win_x64.zip">…</a>` 后面紧跟 `<td class="size" data-size="188640192">`。
pub fn parse_caddy_listing(
    html: &str,
    base: &str,
    major: u32,
    source: &str,
    kind: CaddyKind,
) -> Vec<JavaVersionInfo> {
    let base = if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    };
    let mut out: Vec<JavaVersionInfo> = Vec::new();
    let mut rest = html;
    while let Some(position) = rest.find("href=\"") {
        let after = &rest[position + "href=\"".len()..];
        let Some(end) = after.find('"') else { break };
        let href = &after[..end];
        // 从 href 之后继续扫，size 列在同一个 <tr> 里、href 后面
        rest = &after[end..];
        if href.starts_with("..") || !href.to_ascii_lowercase().ends_with(".zip") {
            continue;
        }
        let name = href.trim_start_matches("./");
        if !kind.accepts(name) {
            continue;
        }
        let Some(version) = kind.version_of(name) else {
            continue;
        };
        let size = rest
            .find("data-size=\"")
            .and_then(|index| {
                let tail = &rest[index + "data-size=\"".len()..];
                tail.find('"')
                    .and_then(|end| tail[..end].trim().parse::<u64>().ok())
            })
            .unwrap_or(0);
        out.push(JavaVersionInfo {
            source: source.to_string(),
            version,
            major,
            lts: LTS_MAJORS.contains(&major),
            size_bytes: size,
            url: format!("{base}{name}"),
        });
    }
    out.sort_by(|left, right| right.version.cmp(&left.version));
    out.dedup_by(|left, right| left.url == right.url);
    out
}

/// microsoft-jdk-21.0.13.0.2-windows-x64.zip → 21.0.13.0.2
pub fn microsoft_version_from_name(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let marker = "microsoft-jdk-";
    let start = lower.find(marker)? + marker.len();
    let version: String = name[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let version = version.trim_end_matches('.').to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// Microsoft 只有一个 aka.ms 直链：HEAD 跟着 302 走到带版本号的文件名，取长度与版本。
pub async fn fetch_microsoft_version(
    client: &reqwest::Client,
    major: u32,
) -> Vec<JavaVersionInfo> {
    let Some(url) = microsoft_zip_url(major) else {
        return Vec::new();
    };
    let response = match tokio::time::timeout(Duration::from_secs(20), client.head(&url).send())
        .await
    {
        Ok(Ok(response)) if response.status().is_success() => response,
        Ok(Ok(response)) => {
            tracing::warn!(major, status = response.status().as_u16(), "Microsoft JDK 探测失败");
            return Vec::new();
        }
        Ok(Err(err)) => {
            tracing::warn!(major, %err, "Microsoft JDK 探测失败");
            return Vec::new();
        }
        Err(_) => {
            tracing::warn!(major, "Microsoft JDK 探测超时");
            return Vec::new();
        }
    };
    // HEAD 的 body 是空的，reqwest 的 content_length() 会给出 0，直接读头更可靠
    let size = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let final_name = response
        .url()
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(|name| name.to_string())
        .unwrap_or_default();
    let version = microsoft_version_from_name(&final_name).unwrap_or_else(|| major.to_string());
    vec![JavaVersionInfo {
        source: "microsoft".to_string(),
        version,
        major,
        lts: LTS_MAJORS.contains(&major),
        size_bytes: size,
        url,
    }]
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

/// 从 Adoptium 目录镜像（TUNA / 兰大，结构一致）的 HTML 里解析 zip。
pub fn parse_mirror_listing(
    html: &str,
    base: &str,
    major: u32,
    source: &str,
) -> Vec<JavaVersionInfo> {
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
            source: source.to_string(),
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
        "tuna" | "lzu" => {
            parse_mirror_listing(body, &mirror_list_url(source, major), major, source)
        }
        "zulu-injdk" => parse_caddy_listing(
            body,
            &injdk_zulu_dir_url(major),
            major,
            source,
            CaddyKind::Zulu,
        ),
        "oracle-injdk" => parse_caddy_listing(
            body,
            &injdk_oracle_dir_url(major),
            major,
            source,
            CaddyKind::Oracle,
        ),
        "azul" => parse_azul(body, major),
        // microsoft 没有目录列表，走 HEAD（在 fetch_versions 里特判）
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
    // 只测可下载的 6 家：oracle 官方只有下载页（403/需登录），没有测速意义
    for source in INSTALLABLE_SOURCES {
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
    if source == "microsoft" {
        return Ok(fetch_microsoft_version(client, major).await);
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
        "tuna" | "lzu" => "temurin",
        "zulu-injdk" | "azul" => "zulu",
        "oracle-injdk" => "oracle",
        "microsoft" => "microsoft",
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


/// 下载候选：首选 → 同族镜像（同名文件，只换根路径）→ 兰大 :8080 备用端口。
///
/// 只有 Adoptium 目录镜像之间文件名完全一致，才谈得上"换镜像"；
/// zulu-injdk / oracle-injdk / microsoft / azul 各自只有一个直链，交给 auto_install 换源。
pub fn download_candidates(source: &str, url: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![(source.to_string(), url.to_string())];
    let roots = mirror_roots(source);
    if roots.is_empty() {
        return out;
    }
    let Some(current) = roots.iter().find(|root| url.starts_with(**root)).copied() else {
        return out;
    };
    let suffix = &url[current.len()..];
    for root in roots {
        if root == current {
            continue;
        }
        out.push((source.to_string(), format!("{root}{suffix}")));
    }
    out
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
    // JDK 包动辄 200MB：用 4 连接分片，比单流快得多，也不容易半路被掐断
    let fetcher = Fetcher::new(client, origins, task.cancel_flag(), INSTALL_CONNECTIONS);
    task.progress_forced(
        Phase::Downloading,
        &format!("正在下载 Java {major}（{source}）"),
        0,
        0,
    );
    // 首选源失败就自动换下一家（Adoptium 两家镜像之间文件名一致，换根即可），日志写清切换
    let candidates = download_candidates(source, &url);
    let mut downloaded_url = url.clone();
    let mut failures: Vec<String> = Vec::new();
    let mut done = false;
    for (index, (attempt_source, attempt_url)) in candidates.iter().enumerate() {
        if index > 0 {
            tracing::warn!(
                from = source,
                to = attempt_source.as_str(),
                url = %attempt_url,
                "Java {major} 下载失败，切换到 {attempt_source} 镜像"
            );
            task.progress_forced(
                Phase::Downloading,
                &format!("正在下载 Java {major}（{attempt_source}）"),
                0,
                0,
            );
        }
        match fetcher.fetch(attempt_url, &zip_path, None, None).await {
            Ok(outcome) => {
                tracing::info!(
                    source = attempt_source.as_str(),
                    url = %attempt_url,
                    bytes = outcome.bytes,
                    "Java 安装包已下载"
                );
                downloaded_url = attempt_url.clone();
                done = true;
                break;
            }
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(err) => failures.push(format!("{attempt_source}: {err}")),
        }
    }
    if !done {
        return Err(AppError::Net(format!(
            "Java {major} 全部镜像都下载失败：{}",
            failures.join("；")
        )));
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
            url: downloaded_url,
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
    fn caddy_listing_parses_zulu_win_x64_zip() {
        let html = r#"<tr class="file">
            <td></td>
            <td><a href="./zulu21.50.19-ca-jdk21.0.11-linux.x86_64.rpm"><span class="name">zulu21.50.19-ca-jdk21.0.11-linux.x86_64.rpm</span></a></td>
            <td class="size" data-size="188640192"><div class="sizebar-text">180 MiB</div></td></tr>
        <tr class="file">
            <td></td>
            <td><a href="./zulu21.50.19-ca-jdk21.0.11-win_x64.zip"><span class="name">zulu21.50.19-ca-jdk21.0.11-win_x64.zip</span></a></td>
            <td class="size" data-size="201450315"><div class="sizebar-text">192 MiB</div></td></tr>
        <tr class="file">
            <td></td>
            <td><a href="./zulu21.48.15-ca-jdk21.0.10-win_x64.zip"><span class="name">zulu21.48.15-ca-jdk21.0.10-win_x64.zip</span></a></td>
            <td class="size" data-size="201000000"><div class="sizebar-text">191 MiB</div></td></tr>"#;
        let items = parse_caddy_listing(
            html,
            "https://d10.injdk.cn/openjdk/zulu/21/",
            21,
            "zulu-injdk",
            CaddyKind::Zulu,
        );
        assert_eq!(items.len(), 2, "linux rpm 要排除：{:?}", items);
        assert_eq!(items[0].version, "21.0.11", "取 jdk 后面的 21.0.11");
        assert_eq!(items[0].size_bytes, 201_450_315);
        assert_eq!(items[0].source, "zulu-injdk");
        assert_eq!(items[1].version, "21.0.10");
        assert_eq!(
            items[0].url,
            "https://d10.injdk.cn/openjdk/zulu/21/zulu21.50.19-ca-jdk21.0.11-win_x64.zip"
        );
        assert!(parse_caddy_listing("", "https://x/", 21, "zulu-injdk", CaddyKind::Zulu).is_empty());
    }

    #[test]
    fn caddy_listing_parses_oracle_windows_zip() {
        let html = r#"<tr class="file"><td><a href="./jdk-21_linux-aarch64_bin.rpm">x</a></td>
            <td class="size" data-size="195331559">186 MiB</td></tr>
        <tr class="file"><td><a href="./jdk-21_windows-x64_bin.zip">x</a></td>
            <td class="size" data-size="185000000">176 MiB</td></tr>"#;
        let items = parse_caddy_listing(
            html,
            "https://d10.injdk.cn/openjdk/oraclejdk/21/",
            21,
            "oracle-injdk",
            CaddyKind::Oracle,
        );
        assert_eq!(items.len(), 1, "只要 *_windows-x64_bin.zip：{:?}", items);
        assert_eq!(items[0].version, "21");
        assert_eq!(items[0].size_bytes, 185_000_000);
        assert!(items[0].url.ends_with("/21/jdk-21_windows-x64_bin.zip"));
        assert!(!CaddyKind::Oracle.accepts("jdk-21_linux-aarch64_bin.rpm"));
        assert_eq!(
            CaddyKind::Oracle.version_of("jdk-8u491-windows-x64.zip").as_deref(),
            Some("8u491")
        );
    }

    #[test]
    fn microsoft_urls_cover_supported_majors_only() {
        assert_eq!(
            microsoft_zip_url(21).as_deref(),
            Some("https://aka.ms/download-jdk/microsoft-jdk-21-windows-x64.zip")
        );
        assert!(microsoft_zip_url(17).is_some());
        assert!(microsoft_zip_url(8).is_none(), "Microsoft 不提供 8");
        assert!(microsoft_zip_url(26).is_none());
        assert_eq!(microsoft_zip_url(21).unwrap(), source_probe_url("microsoft", 21));
        assert_eq!(
            microsoft_version_from_name("microsoft-jdk-21.0.13.0.2-windows-x64.zip").as_deref(),
            Some("21.0.13.0.2")
        );
        assert!(microsoft_version_from_name("something-else.zip").is_none());
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
        let items = parse_mirror_listing(
            html,
            "https://mirrors.tuna.tsinghua.edu.cn/Adoptium/21/jdk/x64/windows/",
            21,
            "tuna",
        );
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
    fn both_mirrors_parse_the_same_listing() {
        // 两家镜像的目录 HTML 结构完全一致（只有 <a href> 与 size 列）
        let html = r#"<tr><td class="link"><a href="OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip" title="x">x</a></td><td class="size">195.6 MiB</td></tr>"#;
        let tuna = parse_mirror_listing(
            html,
            "https://mirrors.tuna.tsinghua.edu.cn/Adoptium/21/jdk/x64/windows/",
            21,
            "tuna",
        );
        let lzu = parse_mirror_listing(
            html,
            "https://mirrors.lzu.edu.cn/adoptium/21/jdk/x64/windows/",
            21,
            "lzu",
        );
        assert_eq!(tuna.len(), 1);
        assert_eq!(lzu.len(), 1);
        assert_eq!(tuna[0].version, "21.0.12.1+1");
        assert_eq!(lzu[0].version, tuna[0].version, "两家文件名一致，版本号必须一致");
        assert_eq!(lzu[0].size_bytes, tuna[0].size_bytes);
        assert_eq!(lzu[0].source, "lzu");
        assert_eq!(tuna[0].source, "tuna");
        assert!(lzu[0].url.starts_with("https://mirrors.lzu.edu.cn/adoptium/"));
        assert!(tuna[0].url.starts_with("https://mirrors.tuna.tsinghua.edu.cn/Adoptium/"));
    }

    #[test]
    fn lzu_urls_use_the_lowercase_path() {
        assert_eq!(
            lzu_list_url(21),
            "https://mirrors.lzu.edu.cn/adoptium/21/jdk/x64/windows/"
        );
        assert!(lzu_list_url(17).contains("/adoptium/17/"));
        assert!(!lzu_list_url(17).contains("/Adoptium/"), "兰大是小写 adoptium");
        assert_eq!(mirror_list_url("tuna", 21), tuna_list_url(21));
        assert_eq!(mirror_list_url("lzu", 21), lzu_list_url(21));
        assert_eq!(source_probe_url("lzu", 21), lzu_list_url(21));
        assert_eq!(mirror_root("lzu"), Some(LZU_ROOT));
        assert_eq!(mirror_root("zulu-injdk"), None);
    }

    #[test]
    fn download_candidates_switch_between_mirrors() {
        let tuna_url = "https://mirrors.tuna.tsinghua.edu.cn/Adoptium/21/jdk/x64/windows/OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip";
        let candidates = download_candidates("tuna", tuna_url);
        assert_eq!(candidates.len(), 3, "首选 → 兰大 → 兰大 :8080：{:?}", candidates);
        assert_eq!(candidates[0], ("tuna".to_string(), tuna_url.to_string()));
        assert_eq!(
            candidates[1].1,
            "https://mirrors.lzu.edu.cn/adoptium/21/jdk/x64/windows/OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip",
            "换镜像只替换根路径，文件名不变"
        );
        assert_eq!(
            candidates[2].1,
            "https://mirror.lzu.edu.cn:8080/adoptium/21/jdk/x64/windows/OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip"
        );

        // 从兰大出发：先 :8080 备用端口，再清华
        let lzu_url = "https://mirrors.lzu.edu.cn/adoptium/21/jdk/x64/windows/OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip";
        let candidates = download_candidates("lzu", lzu_url);
        assert_eq!(candidates.len(), 3);
        assert!(candidates[1].1.starts_with("https://mirror.lzu.edu.cn:8080/adoptium/"));
        assert!(candidates[2].1.starts_with("https://mirrors.tuna.tsinghua.edu.cn/Adoptium/"));

        // 单一来源的源：只有它自己（换源交给 auto_install）
        for source in ["zulu-injdk", "oracle-injdk", "microsoft", "azul"] {
            let candidates = download_candidates(source, "https://x/a.zip");
            assert_eq!(candidates.len(), 1, "{source} 只有一条直链");
            assert_eq!(candidates[0].0, source);
        }
    }

    #[test]
    fn adoptium_mirror_pool_covers_both_hosts_and_spare_port() {
        assert_eq!(
            mirror_roots("tuna"),
            vec![TUNA_ROOT, LZU_ROOT, LZU_SPARE_ROOT],
            "清华首选，兰大与兰大 :8080 依次兜底"
        );
        assert_eq!(mirror_roots("lzu")[0], LZU_ROOT);
        assert!(mirror_roots("azul").is_empty(), "非 Adoptium 镜像没有同族镜像");
        assert_eq!(ADOPTIUM_MIRRORS, [TUNA_ROOT, LZU_ROOT, LZU_SPARE_ROOT]);
        assert!(INSTALLABLE_SOURCES.contains(&"lzu"));
    }

    #[test]
    fn sources_table_lists_six_installable_plus_oracle() {
        let table = sources_table();
        let ids: Vec<&str> = table.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["tuna", "lzu", "zulu-injdk", "oracle-injdk", "microsoft", "azul", "oracle"]
        );
        // 6 家可自动下载 + oracle 仅跳转
        assert_eq!(INSTALLABLE_SOURCES.len(), 6);
        for source in INSTALLABLE_SOURCES {
            assert!(ids.contains(&source), "{source} 必须出现在 java_sources 里");
        }
        assert!(!INSTALLABLE_SOURCES.contains(&"oracle"));
        assert_eq!(table.iter().filter(|item| !item.requires_license).count(), 6);
        let oracle = table.iter().find(|item| item.id == "oracle").unwrap();
        assert!(oracle.requires_license);
        assert!(oracle.description.contains("oracle.com"));
        // injdk 的 Oracle 是直链，允许下载，但描述里要点明 OTN 许可
        let injdk = table.iter().find(|item| item.id == "oracle-injdk").unwrap();
        assert!(!injdk.requires_license);
        assert!(injdk.description.contains("OTN"));
        // 每家都要有名字/描述/下载页说明
        for item in &table {
            assert!(!item.name.is_empty());
            assert!(!item.description.is_empty());
        }
        assert!(table
            .iter()
            .any(|item| item.id == "microsoft" && item.description.contains("learn.microsoft.com")));
        // bellsoft / adoptium 官方已下线
        assert!(!ids.contains(&"bellsoft"));
        assert!(!ids.contains(&"adoptium"));
    }

    #[test]
    fn probe_urls_point_at_measured_endpoints() {
        assert_eq!(
            tuna_list_url(17),
            "https://mirrors.tuna.tsinghua.edu.cn/Adoptium/17/jdk/x64/windows/"
        );
        assert_eq!(
            source_probe_url("zulu-injdk", 21),
            "https://d10.injdk.cn/openjdk/zulu/21/"
        );
        assert_eq!(
            source_probe_url("oracle-injdk", 21),
            "https://d10.injdk.cn/openjdk/oraclejdk/21/"
        );
        assert_eq!(
            source_probe_url("microsoft", 21),
            "https://aka.ms/download-jdk/microsoft-jdk-21-windows-x64.zip"
        );
        assert!(azul_api_url(21).starts_with("https://api.azul.com/metadata/v1/zulu/packages/"));
        assert_eq!(source_probe_url("oracle", 21), ORACLE_PAGE);
        assert!(source_probe_url("tuna", 11).contains("Adoptium/11/"));
        assert_eq!(
            source_probe_url("lzu", 21),
            "https://mirrors.lzu.edu.cn/adoptium/21/jdk/x64/windows/"
        );
    }

    #[test]
    fn fastest_source_ignores_failures_and_oracle() {
        let probes = vec![
            JavaSourceProbe { source_id: "zulu-injdk".into(), ok: true, latency_ms: 900, error: None },
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
        let dir = runtime_dir(&paths, "zulu-injdk", 21, "21.0.12+1");
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), "zulu-21-21.0.12_1");
        let dir = runtime_dir(&paths, "oracle-injdk", 21, "21");
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), "oracle-21-21");
        let dir = runtime_dir(&paths, "microsoft", 21, "21.0.13.0.2");
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), "microsoft-21-21.0.13.0.2");
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
            source: "zulu-injdk".into(),
            vendor: "Zulu".into(),
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
        assert!(err.to_string().contains(ORACLE_PAGE), "错误文案要给出下载页");
        let err = install(
            reqwest::Client::new(),
            &paths,
            "tuna",
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
