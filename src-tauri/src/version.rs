//! 版本 JSON：模型、inheritsFrom 合并、平台库解析、assets 索引。
//!
//! 两条已验规则（沿用旧实现）：
//! 1) 去重键 = group:artifact:classifier + 平台 rules 指纹 + 下载变体（jar / natives），
//!    子版本优先，父版本只补缺；
//! 2) rules 求值按 Mojang 语义：逐条匹配、最后一条命中的规则说了算；没有 rules 时默认允许，
//!    有 rules 但一条都没命中则不允许。
//!
//! natives 有两种写法都要认：老格式的 natives 映射（含 ${arch} 占位）与现代格式的
//! 独立 natives-<os>[-arch] 库条目。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

/// 官方客户端 json。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct VersionJson {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub version_type: Option<String>,
    #[serde(rename = "inheritsFrom", default)]
    pub inherits_from: Option<String>,
    #[serde(rename = "mainClass", default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(rename = "assetIndex", default)]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub downloads: Option<VersionDownloads>,
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments", default)]
    pub minecraft_arguments: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<Artifact>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Artifact {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub total_size: Option<u64>,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default)]
    pub major_version: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub natives: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub extract: Option<Extract>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: BTreeMap<String, Artifact>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<ArgumentEntry>,
    #[serde(default)]
    pub jvm: Vec<ArgumentEntry>,
}

/// arguments 里的一项：纯字符串，或带 rules 的 {rules, value}。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArgumentEntry {
    Plain(String),
    Ruled {
        #[serde(default)]
        rules: Vec<Rule>,
        value: ArgumentValue,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    One(String),
    Many(Vec<String>),
}

/// 解析一个资源版本支持哪些 feature；客户端固定开自定义分辨率（我们要传 --width/--height）。
pub fn default_features() -> BTreeMap<String, bool> {
    let mut map = BTreeMap::new();
    map.insert("has_custom_resolution".to_string(), true);
    map.insert("is_demo_user".to_string(), false);
    map.insert("has_quick_plays_support".to_string(), false);
    map.insert("is_quick_play_singleplayer".to_string(), false);
    map.insert("is_quick_play_multiplayer".to_string(), false);
    map.insert("is_quick_play_realms".to_string(), false);
    map
}

/// 当前平台：Windows + 本机架构（arch 用 JVM 的 os.arch 取值习惯）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
    pub version: String,
}

impl Platform {
    pub fn current() -> Self {
        Self {
            os: "windows".to_string(),
            arch: jvm_arch(std::env::consts::ARCH),
            version: String::new(),
        }
    }

    pub fn windows_x64() -> Self {
        Self {
            os: "windows".to_string(),
            arch: "amd64".to_string(),
            version: String::new(),
        }
    }

    /// 64 位 JVM 用的位数片段（老格式 natives 的 ${arch} 占位）。
    pub fn is_64bit(&self) -> bool {
        matches!(self.arch.as_str(), "amd64" | "x86_64" | "aarch64" | "arm64")
    }
}

fn jvm_arch(arch: &str) -> String {
    match arch {
        "x86_64" => "amd64".to_string(),
        "aarch64" => "aarch64".to_string(),
        other => other.to_string(),
    }
}

fn rule_matches(rule: &Rule, platform: &Platform, features: &BTreeMap<String, bool>) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name {
            if !name.eq_ignore_ascii_case(&platform.os) {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            if !arch.eq_ignore_ascii_case(&platform.arch) {
                return false;
            }
        }
        if let Some(version) = &os.version {
            // 只有 Windows 版本号有约束，其它平台直接忽略
            if platform.os.eq_ignore_ascii_case("windows")
                && !version.trim().is_empty()
                && !platform.version.contains(version.trim())
            {
                return false;
            }
        }
    }
    if let Some(required) = &rule.features {
        for (key, expected) in required {
            let actual = features.get(key).copied().unwrap_or(false);
            if actual != *expected {
                return false;
            }
        }
    }
    true
}

/// Mojang 语义：逐条匹配，最后命中的规则决定结果；没有 rules 默认允许，有 rules 全不命中则拒绝。
pub fn rules_allow(rules: &[Rule], platform: &Platform, features: &BTreeMap<String, bool>) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, platform, features) {
            allowed = rule.action.eq_ignore_ascii_case("allow");
        }
    }
    allowed
}

/// Maven 坐标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coord {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
}

/// 解析 group:artifact:version[:classifier]，顺带吃掉 @zip 这类扩展名后缀。
pub fn parse_coord(name: &str) -> Option<Coord> {
    let name = name.split('@').next().unwrap_or(name);
    let parts: Vec<&str> = name.split(':').collect();
    if parts.len() < 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let classifier = parts.get(3).map(|value| (*value).to_string());
    Some(Coord {
        group: parts[0].to_string(),
        artifact: parts[1].to_string(),
        version: parts[2].to_string(),
        classifier,
    })
}

/// 库在 libraries 目录下的相对路径。
pub fn maven_path(coord: &Coord) -> String {
    let group_path = coord.group.replace('.', "/");
    let artifact = &coord.artifact;
    let version = &coord.version;
    match &coord.classifier {
        Some(classifier) => {
            format!("{group_path}/{artifact}/{version}/{artifact}-{version}-{classifier}.jar")
        }
        None => format!("{group_path}/{artifact}/{version}/{artifact}-{version}.jar"),
    }
}

/// 下载变体：普通 jar 还是 natives（要被解压）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibKind {
    Jar,
    Natives,
}

/// 解析好的一个待下载条目。
#[derive(Debug, Clone)]
pub struct ResolvedLib {
    /// group:artifact:classifier（去重与日志用）
    pub key: String,
    pub name: String,
    pub kind: LibKind,
    /// libraries 下的相对路径（Windows 也用 / 存，落盘时按平台拼）
    pub path: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    /// natives 解压时要跳过的前缀
    pub exclude: Vec<String>,
}

/// 去重键：坐标 + 平台 rules 指纹 + 下载变体。
pub fn library_identity(lib: &Library) -> String {
    let base = match parse_coord(&lib.name) {
        Some(coord) => format!(
            "{}:{}:{}",
            coord.group,
            coord.artifact,
            coord.classifier.clone().unwrap_or_default()
        ),
        None => lib.name.clone(),
    };
    let variant = if lib.natives.is_some() { "natives" } else { "jar" };
    format!("{base}|{}|{variant}", rules_fingerprint(&lib.rules))
}

fn rules_fingerprint(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|rule| {
            let os = rule
                .os
                .as_ref()
                .map(|os| {
                    format!(
                        "{}/{}/{}",
                        os.name.clone().unwrap_or_default(),
                        os.arch.clone().unwrap_or_default(),
                        os.version.clone().unwrap_or_default()
                    )
                })
                .unwrap_or_default();
            let features = rule
                .features
                .as_ref()
                .map(|map| {
                    map.iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect::<Vec<_>>()
                        .join(";")
                })
                .unwrap_or_default();
            format!("{}/{os}/{features}", rule.action)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// 合并后的版本（inheritsFrom 链已经摊平）。
#[derive(Debug, Clone, Default)]
pub struct MergedVersion {
    pub id: String,
    pub version_type: String,
    pub main_class: Option<String>,
    pub assets: String,
    pub asset_index: Option<AssetIndexRef>,
    pub client: Option<Artifact>,
    pub java_major: Option<u32>,
    pub libraries: Vec<Library>,
    pub jvm_arguments: Vec<ArgumentEntry>,
    pub game_arguments: Vec<ArgumentEntry>,
    pub minecraft_arguments: Option<String>,
    /// 子 → 父的顺序
    pub chain: Vec<String>,
}

/// 合并 inheritsFrom 链（chain[0] 是子版本，最后一个是最底层父版本）。
pub fn merge_chain(chain: &[VersionJson]) -> Result<MergedVersion> {
    let Some(child) = chain.first() else {
        return Err(AppError::Invalid("版本链为空，无法合并".to_string()));
    };

    let mut libraries: Vec<Library> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for version in chain {
        for lib in &version.libraries {
            let identity = library_identity(lib);
            if seen.contains(&identity) {
                continue;
            }
            seen.push(identity);
            libraries.push(lib.clone());
        }
    }

    let mut jvm_arguments: Vec<ArgumentEntry> = Vec::new();
    let mut game_arguments: Vec<ArgumentEntry> = Vec::new();
    for version in chain {
        if let Some(arguments) = &version.arguments {
            jvm_arguments.extend(arguments.jvm.iter().cloned());
            game_arguments.extend(arguments.game.iter().cloned());
        }
    }

    let first_some = |pick: &dyn Fn(&VersionJson) -> Option<String>| -> Option<String> {
        chain.iter().find_map(|version| pick(version))
    };

    Ok(MergedVersion {
        id: child.id.clone(),
        version_type: first_some(&|v: &VersionJson| v.version_type.clone())
            .unwrap_or_else(|| "release".to_string()),
        main_class: first_some(&|v: &VersionJson| v.main_class.clone()),
        assets: first_some(&|v: &VersionJson| v.assets.clone()).unwrap_or_else(|| "legacy".to_string()),
        asset_index: chain.iter().find_map(|version| version.asset_index.clone()),
        client: chain
            .iter()
            .find_map(|version| version.downloads.as_ref().and_then(|d| d.client.clone())),
        java_major: chain
            .iter()
            .find_map(|version| version.java_version.as_ref().and_then(|j| j.major_version)),
        libraries,
        jvm_arguments,
        game_arguments,
        minecraft_arguments: first_some(&|v: &VersionJson| v.minecraft_arguments.clone()),
        chain: chain.iter().map(|version| version.id.clone()).collect(),
    })
}

/// natives 分类器里的架构后缀是否匹配当前平台。
///
/// 例：natives-windows 通吃；natives-windows-x86 只在 32 位 JVM 上认；
/// natives-windows-arm64 只在 aarch64 上认（这样不会把三个变体全解压进 natives 目录）。
pub fn native_classifier_matches(classifier: &str, platform: &Platform) -> bool {
    let suffix = classifier.rsplit('-').next().unwrap_or("");
    match suffix {
        "x86" => platform.arch == "x86",
        "arm64" => matches!(platform.arch.as_str(), "aarch64" | "arm64"),
        "amd64" | "x86_64" | "x64" => matches!(platform.arch.as_str(), "amd64" | "x86_64"),
        // 没写架构后缀的 natives-<os> 就是该平台的 64 位版本（官方 1.19+ 的写法）
        "windows" | "linux" | "osx" | "macos" => {
            matches!(platform.arch.as_str(), "amd64" | "x86_64")
        }
        _ => true,
    }
}

fn artifact_of(lib: &Library, classifier: Option<&str>) -> Option<Artifact> {
    let downloads = lib.downloads.as_ref()?;
    if let Some(classifier) = classifier {
        if let Some(artifact) = downloads.classifiers.get(classifier) {
            return Some(artifact.clone());
        }
    }
    downloads.artifact.clone()
}

fn url_from_maven(lib: &Library, coord: &Coord) -> String {
    let base = lib
        .url
        .clone()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| "https://libraries.minecraft.net/".to_string());
    let base = if base.ends_with('/') { base } else { format!("{base}/") };
    format!("{base}{}", maven_path(coord))
}

/// 把一个库按平台解析成 0..n 个待下载条目（普通 jar + 当前平台的 natives）。
pub fn resolve_library(
    lib: &Library,
    platform: &Platform,
    features: &BTreeMap<String, bool>,
) -> Result<Vec<ResolvedLib>> {
    if !rules_allow(&lib.rules, platform, features) {
        return Ok(Vec::new());
    }
    let Some(coord) = parse_coord(&lib.name) else {
        return Err(AppError::Invalid(format!("库坐标无法解析：{}", lib.name)));
    };

    let mut out = Vec::new();
    let exclude = lib
        .extract
        .as_ref()
        .map(|extract| extract.exclude.clone())
        .unwrap_or_default();

    // 1) 老格式：natives 映射 + classifiers
    if let Some(natives) = &lib.natives {
        if let Some(classifier) = natives.get(&platform.os) {
            let classifier = classifier.replace(
                "${arch}",
                if platform.is_64bit() { "64" } else { "32" },
            );
            if let Some(artifact) = artifact_of(lib, Some(&classifier)) {
                let path = artifact.path.clone().unwrap_or_else(|| maven_path(&coord));
                let url = if artifact.url.trim().is_empty() {
                    url_from_maven(lib, &coord)
                } else {
                    artifact.url.clone()
                };
                out.push(ResolvedLib {
                    key: format!(
                        "{}:{}:{}",
                        coord.group, coord.artifact, classifier
                    ),
                    name: lib.name.clone(),
                    kind: LibKind::Natives,
                    path,
                    url,
                    sha1: artifact.sha1.clone(),
                    size: artifact.size,
                    exclude: exclude.clone(),
                });
            }
        }
        // 老格式的 natives 库通常没有普通 artifact：别再按 maven 规则造一个 jar 条目
        if lib
            .downloads
            .as_ref()
            .and_then(|downloads| downloads.artifact.as_ref())
            .is_none()
        {
            return Ok(out);
        }
    }

    // 2) 现代格式：名字里自带 natives-<os>[-arch] 分类器
    if let Some(classifier) = coord.classifier.as_ref() {
        let looks_native = classifier.starts_with("natives");
        if looks_native {
            if !native_classifier_matches(classifier, platform) {
                return Ok(out);
            }
            let artifact = artifact_of(lib, None).or_else(|| artifact_of(lib, Some(classifier)));
            if let Some(artifact) = artifact {
                let path = artifact.path.clone().unwrap_or_else(|| maven_path(&coord));
                let url = if artifact.url.trim().is_empty() {
                    url_from_maven(lib, &coord)
                } else {
                    artifact.url.clone()
                };
                out.push(ResolvedLib {
                    key: format!("{}:{}:{}", coord.group, coord.artifact, classifier),
                    name: lib.name.clone(),
                    kind: LibKind::Natives,
                    path,
                    url,
                    sha1: artifact.sha1.clone(),
                    size: artifact.size,
                    exclude: exclude.clone(),
                });
            }
            return Ok(out);
        }
    }

    // 3) 普通 jar
    let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.clone()) else {
        // 没有 downloads 块：按 maven 前缀拼 URL
        let url = url_from_maven(lib, &coord);
        out.push(ResolvedLib {
            key: format!("{}:{}:{}", coord.group, coord.artifact, ""),
            name: lib.name.clone(),
            kind: LibKind::Jar,
            path: maven_path(&coord),
            url,
            sha1: None,
            size: None,
            exclude: Vec::new(),
        });
        return Ok(out);
    };
    let path = artifact.path.clone().unwrap_or_else(|| maven_path(&coord));
    let url = if artifact.url.trim().is_empty() {
        url_from_maven(lib, &coord)
    } else {
        artifact.url.clone()
    };
    out.push(ResolvedLib {
        key: format!("{}:{}:{}", coord.group, coord.artifact, ""),
        name: lib.name.clone(),
        kind: LibKind::Jar,
        path,
        url,
        sha1: artifact.sha1.clone(),
        size: artifact.size,
        exclude: Vec::new(),
    });
    Ok(out)
}

/// 全量解析（保持定义顺序，已按去重键去过重）。
pub fn resolve_libraries(
    libs: &[Library],
    platform: &Platform,
    features: &BTreeMap<String, bool>,
) -> Result<Vec<ResolvedLib>> {
    let mut out: Vec<ResolvedLib> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for lib in libs {
        for resolved in resolve_library(lib, platform, features)? {
            let key = format!("{}|{:?}|{}", resolved.key, resolved.kind, resolved.path);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            out.push(resolved);
        }
    }
    Ok(out)
}

/// 版本 json 里所有带 rules 的参数求值成平铺字符串。
pub fn collect_arguments(
    entries: &[ArgumentEntry],
    platform: &Platform,
    features: &BTreeMap<String, bool>,
) -> Vec<String> {
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            ArgumentEntry::Plain(value) => out.push(value.clone()),
            ArgumentEntry::Ruled { rules, value } => {
                if !rules_allow(rules, platform, features) {
                    continue;
                }
                match value {
                    ArgumentValue::One(value) => out.push(value.clone()),
                    ArgumentValue::Many(values) => out.extend(values.iter().cloned()),
                }
            }
        }
    }
    out
}

/// assets 索引（indexes/<id>.json）。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AssetIndex {
    #[serde(default)]
    pub objects: BTreeMap<String, AssetObject>,
    #[serde(rename = "map_to_resources", default)]
    pub map_to_resources: bool,
    #[serde(rename = "virtual", default)]
    pub is_virtual: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AssetObject {
    pub hash: String,
    #[serde(default)]
    pub size: u64,
}

impl AssetIndex {
    pub fn parse(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(AppError::Json)
    }
}

/// assets 对象落盘路径：assets/objects/<前两位>/<hash>。
pub fn asset_object_path(assets_root: &Path, hash: &str) -> Result<PathBuf> {
    if hash.len() < 3 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::Invalid(format!("assets 哈希不合法：{hash}")));
    }
    Ok(assets_root.join("objects").join(&hash[..2]).join(hash))
}

/// assets 对象下载地址。
pub fn asset_object_url(hash: &str) -> Result<String> {
    if hash.len() < 3 {
        return Err(AppError::Invalid(format!("assets 哈希不合法：{hash}")));
    }
    Ok(format!(
        "https://resources.download.minecraft.net/{}/{}",
        &hash[..2],
        hash
    ))
}

/// 抽样自检：最多返回 limit 个缺失的 asset 名（空 = 齐全）。
pub fn missing_assets_sample(index: &AssetIndex, assets_root: &Path, limit: usize) -> Vec<String> {
    let mut missing = Vec::new();
    for (name, object) in &index.objects {
        let Ok(path) = asset_object_path(assets_root, &object.hash) else {
            missing.push(name.clone());
            if missing.len() >= limit {
                return missing;
            }
            continue;
        };
        if !path.is_file() {
            missing.push(name.clone());
            if missing.len() >= limit {
                return missing;
            }
        }
    }
    missing
}

/// 全量统计缺失数量（自检用；不返回明细，避免大版本逐条扫描结果爆掉）。
pub fn count_missing_assets(index: &AssetIndex, assets_root: &Path) -> usize {
    index
        .objects
        .values()
        .filter(|object| {
            asset_object_path(assets_root, &object.hash)
                .map(|path| !path.is_file())
                .unwrap_or(true)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> VersionJson {
        serde_json::from_str(text).expect("测试 json 应当能解析")
    }

    fn parse_lib(text: &str) -> Library {
        serde_json::from_str(text).expect("测试库 json 应当能解析")
    }

    fn platform() -> Platform {
        Platform::windows_x64()
    }

    #[test]
    fn inherits_from_merge_prefers_child_and_keeps_parent_fill() {
        let parent = parse(
            r#"{
                "id": "1.20.1",
                "mainClass": "net.minecraft.client.main.Main",
                "assets": "5",
                "assetIndex": {"id": "5", "url": "https://example.com/5.json"},
                "javaVersion": {"majorVersion": 17},
                "downloads": {"client": {"url": "https://example.com/client.jar", "size": 10, "sha1": "aa"}},
                "libraries": [
                    {"name": "org.lwjgl:lwjgl:3.3.1", "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar", "url": "https://example.com/a.jar"}}},
                    {"name": "com.mojang:brigadier:1.0.18"}
                ]
            }"#,
        );
        let child = parse(
            r#"{
                "id": "1.20.1-forge-47.2.0",
                "inheritsFrom": "1.20.1",
                "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
                "libraries": [
                    {"name": "org.lwjgl:lwjgl:3.3.2", "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2.jar", "url": "https://example.com/b.jar"}}},
                    {"name": "net.minecraftforge:forge:47.2.0"}
                ]
            }"#,
        );
        let merged = merge_chain(&[child, parent]).expect("合并应当成功");

        assert_eq!(merged.id, "1.20.1-forge-47.2.0");
        assert_eq!(
            merged.main_class.as_deref(),
            Some("cpw.mods.bootstraplauncher.BootstrapLauncher")
        );
        assert_eq!(merged.assets, "5");
        assert_eq!(merged.java_major, Some(17));
        assert!(merged.client.is_some());
        assert_eq!(merged.chain, vec!["1.20.1-forge-47.2.0", "1.20.1"]);

        // 子版本优先：仍然只有 3 个条目（子 2 条 + 父补 1 条 brigadier），父的 lwjgl 被覆盖
        let names: Vec<&str> = merged.libraries.iter().map(|lib| lib.name.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert_eq!(names[0], "org.lwjgl:lwjgl:3.3.2");
        assert!(names.contains(&"com.mojang:brigadier:1.0.18"));
        assert!(!names.contains(&"org.lwjgl:lwjgl:3.3.1"));
    }

    #[test]
    fn dedupe_key_keeps_jar_and_natives_apart() {
        let jar = parse_lib(
            r#"{"name": "org.lwjgl:lwjgl:3.3.1", "downloads": {"artifact": {"url": "https://example.com/jar.jar"}}}"#,
        );
        let natives = parse_lib(
            r#"{"name": "org.lwjgl:lwjgl:3.3.1:natives-windows", "rules": [{"action": "allow", "os": {"name": "windows"}}], "downloads": {"artifact": {"url": "https://example.com/nat.jar"}}}"#,
        );
        assert_ne!(library_identity(&jar), library_identity(&natives));
    }

    #[test]
    fn dedupe_key_separates_platform_rule_variants() {
        let windows = parse_lib(
            r#"{"name": "org.lwjgl:lwjgl:3.3.1:natives", "rules": [{"action": "allow", "os": {"name": "windows"}}]}"#,
        );
        let osx = parse_lib(
            r#"{"name": "org.lwjgl:lwjgl:3.3.1:natives", "rules": [{"action": "allow", "os": {"name": "osx"}}]}"#,
        );
        assert_ne!(library_identity(&windows), library_identity(&osx));
    }

    #[test]
    fn rules_use_last_match_and_no_match_means_excluded() {
        let features = default_features();
        let platform = platform();

        // 没有 rules → 允许
        assert!(rules_allow(&[], &platform, &features));

        // [{allow}, {disallow osx}] → Windows 上允许
        let rules = vec![
            Rule {
                action: "allow".into(),
                ..Default::default()
            },
            Rule {
                action: "disallow".into(),
                os: Some(OsRule {
                    name: Some("osx".into()),
                    ..Default::default()
                }),
                features: None,
            },
        ];
        assert!(rules_allow(&rules, &platform, &features));

        let mut mac = platform.clone();
        mac.os = "osx".into();
        assert!(!rules_allow(&rules, &mac, &features));

        // 只允许 windows → Linux 上不允许
        let only_windows = vec![Rule {
            action: "allow".into(),
            os: Some(OsRule {
                name: Some("windows".into()),
                ..Default::default()
            }),
            features: None,
        }];
        let mut linux = platform.clone();
        linux.os = "linux".into();
        assert!(!rules_allow(&only_windows, &linux, &features));
        assert!(rules_allow(&only_windows, &platform, &features));

        // feature 规则：默认关闭的 demo 用户不生效
        let demo = vec![Rule {
            action: "allow".into(),
            os: None,
            features: Some(BTreeMap::from([("is_demo_user".to_string(), true)])),
        }];
        assert!(!rules_allow(&demo, &platform, &features));
    }

    #[test]
    fn native_arch_variant_selection() {
        let platform = platform();
        assert!(native_classifier_matches("natives-windows", &platform));
        assert!(native_classifier_matches("natives-linux", &platform));
        assert!(!native_classifier_matches("natives-windows-x86", &platform));
        assert!(!native_classifier_matches("natives-windows-arm64", &platform));

        let mut x86 = platform.clone();
        x86.arch = "x86".into();
        assert!(native_classifier_matches("natives-windows-x86", &x86));
        assert!(!native_classifier_matches("natives-windows", &x86));
    }

    #[test]
    fn modern_natives_resolve_to_single_variant_for_windows_x64() {
        let libs = vec![
            parse_lib(r#"{"name": "org.lwjgl:lwjgl:3.3.1", "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar", "url": "https://example.com/lwjgl.jar"}}}"#),
            parse_lib(r#"{"name": "org.lwjgl:lwjgl:3.3.1:natives-windows", "rules": [{"action": "allow", "os": {"name": "windows"}}], "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar", "url": "https://example.com/nat-win.jar"}}}"#),
            parse_lib(r#"{"name": "org.lwjgl:lwjgl:3.3.1:natives-windows-x86", "rules": [{"action": "allow", "os": {"name": "windows"}}], "downloads": {"artifact": {"url": "https://example.com/nat-x86.jar"}}}"#),
            parse_lib(r#"{"name": "org.lwjgl:lwjgl:3.3.1:natives-windows-arm64", "rules": [{"action": "allow", "os": {"name": "windows"}}], "downloads": {"artifact": {"url": "https://example.com/nat-arm64.jar"}}}"#),
            parse_lib(r#"{"name": "org.lwjgl:lwjgl:3.3.1:natives-linux", "rules": [{"action": "allow", "os": {"name": "linux"}}], "downloads": {"artifact": {"url": "https://example.com/nat-linux.jar"}}}"#),
        ];
        let resolved = resolve_libraries(&libs, &platform(), &default_features()).unwrap();
        let natives: Vec<&ResolvedLib> = resolved
            .iter()
            .filter(|lib| lib.kind == LibKind::Natives)
            .collect();
        assert_eq!(natives.len(), 1, "Windows x64 只该解压一份 natives");
        assert_eq!(natives[0].url, "https://example.com/nat-win.jar");
        assert_eq!(
            resolved.iter().filter(|lib| lib.kind == LibKind::Jar).count(),
            1
        );
    }

    #[test]
    fn legacy_natives_map_uses_classifier_and_arch_placeholder() {
        let lib = parse_lib(
            r#"{
                "name": "org.lwjgl:lwjgl:2.9.4",
                "natives": {"windows": "natives-windows-${arch}"},
                "extract": {"exclude": ["META-INF/"]},
                "downloads": {
                    "classifiers": {
                        "natives-windows-64": {"path": "org/lwjgl/lwjgl/2.9.4/lwjgl-2.9.4-natives-windows-64.jar", "url": "https://example.com/nat64.jar", "size": 111, "sha1": "abc"}
                    }
                }
            }"#,
        );
        let resolved = resolve_library(&lib, &platform(), &default_features()).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].kind, LibKind::Natives);
        assert_eq!(resolved[0].url, "https://example.com/nat64.jar");
        assert_eq!(resolved[0].sha1.as_deref(), Some("abc"));
        assert_eq!(resolved[0].size, Some(111));
        assert_eq!(resolved[0].exclude, vec!["META-INF/"]);
        assert_eq!(resolved[0].key, "org.lwjgl:lwjgl:natives-windows-64");
    }

    #[test]
    fn missing_downloads_falls_back_to_maven_base() {
        let lib = parse_lib(r#"{"name": "net.minecraftforge:forge:47.2.0", "url": "https://maven.minecraftforge.net/"}"#);
        let resolved = resolve_library(&lib, &platform(), &default_features()).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].url,
            "https://maven.minecraftforge.net/net/minecraftforge/forge/47.2.0/forge-47.2.0.jar"
        );
        assert_eq!(
            resolved[0].path,
            "net/minecraftforge/forge/47.2.0/forge-47.2.0.jar"
        );
    }

    #[test]
    fn coord_and_maven_path_layout() {
        let coord = parse_coord("com.mojang:logging:1.1.1").unwrap();
        assert_eq!(coord.group, "com.mojang");
        assert_eq!(coord.artifact, "logging");
        assert_eq!(coord.classifier, None);
        assert_eq!(maven_path(&coord), "com/mojang/logging/1.1.1/logging-1.1.1.jar");

        let with_classifier = parse_coord("org.lwjgl:lwjgl:3.3.1:natives-windows@jar").unwrap();
        assert_eq!(with_classifier.classifier.as_deref(), Some("natives-windows"));
        assert_eq!(
            maven_path(&with_classifier),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar"
        );

        assert!(parse_coord("broken").is_none());
        assert!(parse_coord("a::1").is_none());
    }

    #[test]
    fn arguments_are_filtered_by_rules() {
        let entries: Vec<ArgumentEntry> = serde_json::from_str(
            r#"[
                "-Dminecraft.launcher.brand=${launcher_name}",
                {"rules": [{"action": "allow", "os": {"name": "osx"}}], "value": ["-XstartOnFirstThread"]},
                {"rules": [{"action": "allow", "os": {"name": "windows"}}], "value": "-XX:+UseG1GC"},
                {"rules": [{"action": "allow", "features": {"has_custom_resolution": true}}], "value": ["--width", "${resolution_width}"]}
            ]"#,
        )
        .unwrap();
        let collected = collect_arguments(&entries, &platform(), &default_features());
        assert_eq!(
            collected,
            vec![
                "-Dminecraft.launcher.brand=${launcher_name}".to_string(),
                "-XX:+UseG1GC".to_string(),
                "--width".to_string(),
                "${resolution_width}".to_string(),
            ]
        );
    }

    #[test]
    fn asset_paths_and_missing_sample() {
        let root = std::env::temp_dir().join(format!("gugu-assets-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let hash = "0f1b2c3d4e5f60718293a4b5c6d7e8f901234567";
        let index = AssetIndex {
            objects: BTreeMap::from([
                (
                    "minecraft/sounds/a.ogg".to_string(),
                    AssetObject {
                        hash: hash.to_string(),
                        size: 4,
                    },
                ),
                (
                    "minecraft/textures/b.png".to_string(),
                    AssetObject {
                        hash: "aa11bb22cc33dd44ee55ff667788990011223344".to_string(),
                        size: 7,
                    },
                ),
            ]),
            map_to_resources: false,
            is_virtual: false,
        };

        let path = asset_object_path(&root, hash).unwrap();
        assert_eq!(
            path,
            root.join("objects").join("0f").join(hash)
        );
        assert_eq!(count_missing_assets(&index, &root), 2);
        double_check(&index, &root);

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"abcd").unwrap();
        assert_eq!(count_missing_assets(&index, &root), 1);
        let sample = missing_assets_sample(&index, &root, 10);
        assert_eq!(sample, vec!["minecraft/textures/b.png".to_string()]);

        // 抽样上限生效
        let many = missing_assets_sample(&index, &root, 1);
        assert_eq!(many.len(), 1);

        assert!(asset_object_path(&root, "zz").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn double_check(index: &AssetIndex, root: &Path) {
        let sample = missing_assets_sample(index, root, 10);
        assert_eq!(sample.len(), 2);
    }

    #[test]
    fn asset_url_uses_two_char_prefix() {
        assert_eq!(
            asset_object_url("0f1b2c3d4e5f60718293a4b5c6d7e8f901234567").unwrap(),
            "https://resources.download.minecraft.net/0f/0f1b2c3d4e5f60718293a4b5c6d7e8f901234567"
        );
    }

    #[test]
    fn single_version_without_inherits_keeps_its_own_fields() {
        let json = parse(
            r#"{
                "id": "1.20.1",
                "type": "release",
                "mainClass": "net.minecraft.client.main.Main",
                "assets": "5",
                "javaVersion": {"majorVersion": 17},
                "arguments": {"game": ["--username"], "jvm": ["-cp"]}
            }"#,
        );
        let merged = merge_chain(&[json]).unwrap();
        assert_eq!(merged.id, "1.20.1");
        assert_eq!(merged.version_type, "release");
        assert_eq!(merged.assets, "5");
        assert_eq!(merged.jvm_arguments.len(), 1);
        assert_eq!(merged.game_arguments.len(), 1);
        assert!(merged.client.is_none());
    }

    #[test]
    fn empty_chain_is_rejected() {
        assert!(merge_chain(&[]).is_err());
    }

    #[test]
    fn real_1_20_1_json_shape_parses() {
        let text = r#"{
            "id": "1.20.1",
            "mainClass": "net.minecraft.client.main.Main",
            "assets": "5",
            "assetIndex": {"id": "5", "sha1": "sha", "size": 1, "totalSize": 2, "url": "https://piston-meta.mojang.com/v1/packages/5/5.json"},
            "javaVersion": {"component": "java-runtime-gamma", "majorVersion": 17},
            "downloads": {"client": {"sha1": "c", "size": 23028853, "url": "https://piston-data.mojang.com/v1/objects/c/client.jar"}},
            "libraries": [
                {"name": "org.lwjgl:lwjgl:3.3.1", "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar", "sha1": "a", "size": 1, "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar"}, "classifiers": {}}},
                {"name": "org.lwjgl:lwjgl:3.3.1:natives-windows", "rules": [{"action": "allow", "os": {"name": "windows"}}], "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar", "url": "https://libraries.minecraft.net/x.jar"}}}
            ],
            "arguments": {"game": ["--username", "${auth_player_name}"], "jvm": ["-Djava.library.path=${natives_directory}"]}
        }"#;
        let version = parse(text);
        let merged = merge_chain(&[version]).unwrap();
        assert_eq!(merged.java_major, Some(17));
        let resolved = resolve_libraries(&merged.libraries, &platform(), &default_features()).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(merged.asset_index.unwrap().id, "5");
    }
}
