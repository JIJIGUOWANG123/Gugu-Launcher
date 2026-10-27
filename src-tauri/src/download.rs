//! 下载源表 + URL 映射（纯函数，全部带单测）+ 分片下载器接口。

use serde::{Deserialize, Serialize};

/// 下载源。auto 表示运行时按测速结果选择，映射阶段不改变 URL。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadSource {
    Official,
    Bmclapi,
    Sjtu,
    /// BMCLAPI 协议镜像（8 个国内镜像）
    Minguncle,
    Nju,
    Lzu,
    Qlu,
    Hit,
    Ha,
    Ustc,
    Nyist,
    Auto,
}

impl DownloadSource {
    pub const ALL: [DownloadSource; 12] = [
        DownloadSource::Official,
        DownloadSource::Bmclapi,
        DownloadSource::Minguncle,
        DownloadSource::Nju,
        DownloadSource::Lzu,
        DownloadSource::Qlu,
        DownloadSource::Hit,
        DownloadSource::Ha,
        DownloadSource::Ustc,
        DownloadSource::Nyist,
        DownloadSource::Sjtu,
        DownloadSource::Auto,
    ];

    /// 走 BMCLAPI 协议的镜像（不含官方 / sjtu / auto）。
    pub const MIRRORS: [DownloadSource; 9] = [
        DownloadSource::Bmclapi,
        DownloadSource::Minguncle,
        DownloadSource::Nju,
        DownloadSource::Lzu,
        DownloadSource::Qlu,
        DownloadSource::Hit,
        DownloadSource::Ha,
        DownloadSource::Ustc,
        DownloadSource::Nyist,
    ];

    pub fn id(&self) -> &'static str {
        match self {
            DownloadSource::Official => "official",
            DownloadSource::Bmclapi => "bmclapi",
            DownloadSource::Minguncle => "minguncle",
            DownloadSource::Nju => "nju",
            DownloadSource::Lzu => "lzu",
            DownloadSource::Qlu => "qlu",
            DownloadSource::Hit => "hit",
            DownloadSource::Ha => "ha",
            DownloadSource::Ustc => "ustc",
            DownloadSource::Nyist => "nyist",
            DownloadSource::Sjtu => "sjtu",
            DownloadSource::Auto => "auto",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.id() == id)
    }

    /// 源的主站（auto 没有固定主站）。
    pub fn base(&self) -> &'static str {
        match self {
            DownloadSource::Official => "https://launchermeta.mojang.com",
            DownloadSource::Bmclapi => "https://bmclapi2.bangbang93.com",
            DownloadSource::Minguncle => "https://mirror.minguncle.cn",
            DownloadSource::Nju => "https://mirror.nju.edu.cn",
            DownloadSource::Lzu => "https://mirror.lzu.edu.cn/bmclapi",
            DownloadSource::Qlu => "https://mirrors.qlu.edu.cn/bmclapi",
            DownloadSource::Hit => "https://mirrors.hit.edu.cn/bmclapi",
            DownloadSource::Ha => "https://mirrors.ha.edu.cn/bmclapi",
            DownloadSource::Ustc => "https://mirrors.ustc.edu.cn/bmclapi",
            DownloadSource::Nyist => "https://mirror.nyist.edu.cn/bmclapi",
            DownloadSource::Sjtu => "https://mirror.sjtu.edu.cn/minecraft",
            DownloadSource::Auto => "",
        }
    }

    /// 是否 BMCLAPI 协议的镜像（路径规则与 bmclapi 官方一致）。
    pub fn is_bmclapi_mirror(&self) -> bool {
        Self::MIRRORS.contains(self)
    }

    /// 只镜像 libraries / assets 的「部分镜像」：这些大学站没有 /mc/ 端点
    /// （实测 404），所以版本清单与版本 jar 仍走官方，只有库与资源走它。
    pub fn partial_mirror(&self) -> bool {
        self.is_bmclapi_mirror()
            && !matches!(self, DownloadSource::Bmclapi | DownloadSource::Minguncle)
    }

    /// 探测用的地址：完整镜像探清单，部分镜像探 maven 目录。
    pub fn probe_url(&self) -> String {
        match self {
            DownloadSource::Official | DownloadSource::Auto => OFFICIAL_MANIFEST.to_string(),
            other if other.partial_mirror() => format!("{}/maven/", other.base()),
            other => other.manifest_url(),
        }
    }

    /// 版本清单地址。
    pub fn manifest_url(&self) -> String {
        match self {
            DownloadSource::Official | DownloadSource::Auto => OFFICIAL_MANIFEST.to_string(),
            DownloadSource::Sjtu => {
                "https://mirror.sjtu.edu.cn/minecraft/launchermeta/mc/game/version_manifest_v2.json".to_string()
            }
            other => format!("{}/mc/game/version_manifest_v2.json", other.base()),
        }
    }
}

pub const OFFICIAL_MANIFEST: &str = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";

/// 官方域名 → BMCLAPI 协议路径后缀（各镜像只换根地址）。
pub const BMCLAPI_RULES: [(&str, &str); 5] = [
    ("https://launchermeta.mojang.com/", ""),
    ("https://piston-meta.mojang.com/", ""),
    ("https://piston-data.mojang.com/", ""),
    ("https://resources.download.minecraft.net/", "assets/"),
    ("https://libraries.minecraft.net/", "maven/"),
];

const SJTU_MAP: [(&str, &str); 5] = [
    ("https://launchermeta.mojang.com/", "https://mirror.sjtu.edu.cn/minecraft/launchermeta/"),
    ("https://piston-meta.mojang.com/", "https://mirror.sjtu.edu.cn/minecraft/piston-meta/"),
    ("https://piston-data.mojang.com/", "https://mirror.sjtu.edu.cn/minecraft/piston-data/"),
    ("https://resources.download.minecraft.net/", "https://mirror.sjtu.edu.cn/minecraft/assets/"),
    ("https://libraries.minecraft.net/", "https://mirror.sjtu.edu.cn/minecraft/maven/"),
];

fn rewrite_with(table: &[(&str, &str)], original: &str) -> Option<String> {
    table
        .iter()
        .find_map(|(from, to)| original.strip_prefix(from).map(|rest| format!("{to}{rest}")))
}

/// 按 BMCLAPI 协议把官方 URL 换到某个镜像根。
pub fn map_bmclapi(base: &str, original: &str) -> Option<String> {
    let base = base.trim_end_matches('/');
    BMCLAPI_RULES.iter().find_map(|(from, suffix)| {
        original
            .strip_prefix(from)
            .map(|rest| format!("{base}/{suffix}{rest}"))
    })
}

/// 部分镜像只改写 libraries 一类 URL。
/// 实测这些大学站的 /assets/<hash> 也是 404（只有 maven 代理是真的），所以不碰 assets。
const PARTIAL_RULES: [(&str, &str); 1] = [("https://libraries.minecraft.net/", "maven/")];

fn map_partial(base: &str, original: &str) -> Option<String> {
    let base = base.trim_end_matches('/');
    PARTIAL_RULES.iter().find_map(|(from, suffix)| {
        original
            .strip_prefix(from)
            .map(|rest| format!("{base}/{suffix}{rest}"))
    })
}

/// 把一个官方 URL 映射到指定源；未命中的 URL 原样返回。
pub fn map_url(source: DownloadSource, original: &str) -> String {
    let mapped = if source.partial_mirror() {
        map_partial(source.base(), original)
    } else if source.is_bmclapi_mirror() {
        map_bmclapi(source.base(), original)
    } else if source == DownloadSource::Sjtu {
        rewrite_with(&SJTU_MAP, original)
    } else {
        None
    };
    mapped.unwrap_or_else(|| original.to_string())
}

/// 资源通道（模组/整合包）：CurseForge / Modrinth 走官方，MCIM 提供镜像。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceChannel {
    Official,
    Mcim,
}

pub const MCIM_BASE: &str = "https://mod.mcimirror.top";

/// 资源 URL 映射：官方 URL → 指定通道。
/// 规则与 0.5.6 一致：MCIM 用 `<base>/<provider>/<path>`，官方原样返回。
pub fn resource_url(channel: ResourceChannel, provider: &str, path: &str) -> String {
    let path = path.trim_start_matches('/');
    match channel {
        ResourceChannel::Official => match provider {
            "modrinth" => format!("https://cdn.modrinth.com/data/{path}"),
            "curseforge" => format!("https://edge.forgecdn.net/files/{path}"),
            _ => format!("https://cdn.modrinth.com/data/{path}"),
        },
        ResourceChannel::Mcim => format!("{MCIM_BASE}/{provider}/{path}"),
    }
}

// 源表（含自定义镜像与 selectable）在 crate::sources 里，因为要读设置。

// --------------------------------------------------------------- 分片下载

/// 一个字节区间（左闭右闭，和 HTTP Range 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

/// 把总长度切成 N 个区间（最后一段吃掉余数）。
pub fn plan_ranges(total: u64, chunk_size: u64) -> Vec<ByteRange> {
    if total == 0 || chunk_size == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0u64;
    while start < total {
        let end = (start + chunk_size - 1).min(total - 1);
        out.push(ByteRange { start, end });
        start = end + 1;
    }
    out
}

// 真正的下载器（多源回退 + 分片并发）在 crate::downloader 里。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_table_has_all_sources() {
        assert_eq!(DownloadSource::ALL.len(), 12, "官方 + bmclapi + 8 镜像 + sjtu + auto");
        assert_eq!(DownloadSource::MIRRORS.len(), 9, "bmclapi + 8 个镜像");
        for source in DownloadSource::MIRRORS {
            assert!(source.base().starts_with("https://"), "{source:?}");
            assert!(
                source.manifest_url().ends_with("/mc/game/version_manifest_v2.json"),
                "{source:?}"
            );
            assert_eq!(DownloadSource::from_id(source.id()), Some(source));
        }
        assert_eq!(DownloadSource::Minguncle.id(), "minguncle");
        assert_eq!(DownloadSource::Ustc.base(), "https://mirrors.ustc.edu.cn/bmclapi");
        assert!(DownloadSource::Lzu.is_bmclapi_mirror());
        assert!(!DownloadSource::Official.is_bmclapi_mirror());
    }

    #[test]
    fn bmclapi_mapping_works_for_every_mirror() {
        let cases = [
            (
                "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json",
                "/mc/game/version_manifest_v2.json",
            ),
            (
                "https://piston-data.mojang.com/v1/objects/abc/client.jar",
                "/v1/objects/abc/client.jar",
            ),
            (
                "https://resources.download.minecraft.net/ab/abcdef",
                "/assets/ab/abcdef",
            ),
            (
                "https://libraries.minecraft.net/com/google/gson/2.10/gson-2.10.jar",
                "/maven/com/google/gson/2.10/gson-2.10.jar",
            ),
        ];
        for source in DownloadSource::MIRRORS {
            for (original, suffix) in cases {
                // 只镜像 maven 的大学站只改写 libraries（assets 实测 404，不改写）
                let library_or_asset = original.contains("libraries.minecraft.net");
                let expected = if source.partial_mirror() && !library_or_asset {
                    original.to_string()
                } else {
                    let suffix = match (source.partial_mirror(), suffix.starts_with("/mc/")) {
                        (true, true) => "",
                        _ => suffix,
                    };
                    if suffix.is_empty() {
                        original.to_string()
                    } else {
                        format!("{}{suffix}", source.base())
                    }
                };
                assert_eq!(
                    map_url(source, original),
                    expected,
                    "{source:?} 映射 {original} 不对"
                );
            }
        }
        assert_eq!(map_url(DownloadSource::Auto, "https://x/y"), "https://x/y");
        assert_eq!(
            map_url(DownloadSource::Ustc, "https://example.com/not-minecraft"),
            "https://example.com/not-minecraft"
        );
    }

    #[test]
    fn partial_mirrors_only_rewrite_libraries_and_assets() {
        // 完整镜像：清单也改写
        assert_eq!(
            map_url(
                DownloadSource::Minguncle,
                "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json"
            ),
            "https://mirror.minguncle.cn/mc/game/version_manifest_v2.json"
        );
        // 部分镜像（大学站只镜像 maven/assets）：清单保持官方
        assert_eq!(
            map_url(
                DownloadSource::Hit,
                "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json"
            ),
            "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json"
        );
        assert_eq!(
            map_url(DownloadSource::Hit, "https://libraries.minecraft.net/a/b.jar"),
            "https://mirrors.hit.edu.cn/bmclapi/maven/a/b.jar"
        );
        // assets 不交给部分镜像（实测 404）
        assert_eq!(
            map_url(DownloadSource::Ustc, "https://resources.download.minecraft.net/ab/cd"),
            "https://resources.download.minecraft.net/ab/cd"
        );
        // 探测地址：完整镜像探清单，部分镜像探 maven 目录
        assert!(DownloadSource::Minguncle.probe_url().contains("/mc/game/"));
        assert_eq!(
            DownloadSource::Ustc.probe_url(),
            "https://mirrors.ustc.edu.cn/bmclapi/maven/"
        );
        assert!(DownloadSource::Minguncle.partial_mirror() == false);
        assert!(DownloadSource::Ustc.partial_mirror());
        assert!(!DownloadSource::Official.partial_mirror());
    }

    #[test]
    fn builtin_source_ids_roundtrip() {
        for source in DownloadSource::ALL {
            let parsed = DownloadSource::from_id(source.id()).expect("id 必须能反解");
            assert_eq!(parsed.id(), source.id());
        }
        assert_eq!(DownloadSource::from_id("nope"), None);
    }

    #[test]
    fn official_and_auto_leave_urls_untouched() {
        let url = "https://piston-data.mojang.com/v1/objects/aa/bb.jar";
        assert_eq!(map_url(DownloadSource::Official, url), url);
        assert_eq!(map_url(DownloadSource::Auto, url), url);
    }

    #[test]
    fn bmclapi_rewrites_known_hosts() {
        assert_eq!(
            map_url(DownloadSource::Bmclapi, OFFICIAL_MANIFEST),
            "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json"
        );
        assert_eq!(
            map_url(
                DownloadSource::Bmclapi,
                "https://resources.download.minecraft.net/ab/abc123"
            ),
            "https://bmclapi2.bangbang93.com/assets/ab/abc123"
        );
        assert_eq!(
            map_url(DownloadSource::Bmclapi, "https://libraries.minecraft.net/net/fabricmc/a.jar"),
            "https://bmclapi2.bangbang93.com/maven/net/fabricmc/a.jar"
        );
    }

    #[test]
    fn sjtu_rewrites_host_and_unknown_urls_pass_through() {
        let mapped = map_url(DownloadSource::Sjtu, OFFICIAL_MANIFEST);
        assert!(mapped.starts_with("https://mirror.sjtu.edu.cn/minecraft/launchermeta/"));
        let unknown = "https://example.com/some/file.jar";
        assert_eq!(map_url(DownloadSource::Sjtu, unknown), unknown);
    }

    #[test]
    fn resource_channel_mapping() {
        assert_eq!(
            resource_url(ResourceChannel::Mcim, "modrinth", "/data/abc/version.jar"),
            "https://mod.mcimirror.top/modrinth/data/abc/version.jar"
        );
        assert_eq!(
            resource_url(ResourceChannel::Official, "modrinth", "abc/version.jar"),
            "https://cdn.modrinth.com/data/abc/version.jar"
        );
    }

    #[test]
    fn plan_ranges_covers_file_exactly() {
        let ranges = plan_ranges(10, 4);
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0], ByteRange { start: 0, end: 3 });
        assert_eq!(ranges[2], ByteRange { start: 8, end: 9 });
        assert_eq!(ranges.iter().map(|r| r.len()).sum::<u64>(), 10);

        assert!(plan_ranges(0, 4).is_empty());
        assert!(plan_ranges(10, 0).is_empty());
        assert_eq!(plan_ranges(4, 8), vec![ByteRange { start: 0, end: 3 }]);
    }
}