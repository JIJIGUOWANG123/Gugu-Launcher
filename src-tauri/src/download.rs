//! 下载源表 + URL 映射（纯函数，全部带单测）+ 分片下载器接口。
//!
//! M1 只做「源表 + 映射 + 分片规划」，真正的并发分片下载留到 M2。

use serde::{Deserialize, Serialize};

/// 下载源。`auto` 表示运行时按测速结果选择，映射阶段不改变 URL。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadSource {
    Official,
    Bmclapi,
    Sjtu,
    Auto,
}

impl DownloadSource {
    pub const ALL: [DownloadSource; 4] = [
        DownloadSource::Official,
        DownloadSource::Bmclapi,
        DownloadSource::Sjtu,
        DownloadSource::Auto,
    ];

    pub fn id(&self) -> &'static str {
        match self {
            DownloadSource::Official => "official",
            DownloadSource::Bmclapi => "bmclapi",
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
            DownloadSource::Sjtu => "https://mirror.sjtu.edu.cn/minecraft",
            DownloadSource::Auto => "",
        }
    }

    /// 版本清单地址。
    pub fn manifest_url(&self) -> String {
        match self {
            DownloadSource::Official | DownloadSource::Auto => OFFICIAL_MANIFEST.to_string(),
            DownloadSource::Bmclapi => "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json".to_string(),
            DownloadSource::Sjtu => {
                "https://mirror.sjtu.edu.cn/minecraft/launchermeta/mc/game/version_manifest_v2.json".to_string()
            }
        }
    }
}

pub const OFFICIAL_MANIFEST: &str = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";

/// 官方域名 → 镜像前缀。顺序敏感：先匹配更长的前缀。
///
/// 注意：SJTU 的前缀是按镜像站公开的 `/minecraft/...` 目录结构写的，
/// 具体前缀需要在 M2 接入真实下载时实测核对（这里只保证映射机制正确）。
const BMCLAPI_MAP: [(&str, &str); 5] = [
    ("https://launchermeta.mojang.com/", "https://bmclapi2.bangbang93.com/"),
    ("https://piston-meta.mojang.com/", "https://bmclapi2.bangbang93.com/"),
    ("https://piston-data.mojang.com/", "https://bmclapi2.bangbang93.com/"),
    ("https://resources.download.minecraft.net/", "https://bmclapi2.bangbang93.com/assets/"),
    ("https://libraries.minecraft.net/", "https://bmclapi2.bangbang93.com/maven/"),
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

/// 把一个官方 URL 映射到指定源；未命中的 URL 原样返回（未知资源不能因为映射失败就丢）。
pub fn map_url(source: DownloadSource, original: &str) -> String {
    let mapped = match source {
        DownloadSource::Official | DownloadSource::Auto => None,
        DownloadSource::Bmclapi => rewrite_with(&BMCLAPI_MAP, original),
        DownloadSource::Sjtu => rewrite_with(&SJTU_MAP, original),
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
