//! 主页新闻：Minecraft 官方 launchercontent 的 news.json（含站点文章网格兜底）。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::settings::Settings;
use crate::translate::BuiltinTranslator;
use crate::util;

pub const NEWS_URL: &str = "https://launchercontent.mojang.com/v2/news.json";
pub const NEWS_IMAGE_BASE: &str = "https://launchercontent.mojang.com";
pub const FALLBACK_URL: &str = "https://www.minecraft.net/content/minecraftnet/language-masters/en-us/jcr:content/root/container/image_grid_a_copy_64.articles.page-1.json";
pub const FALLBACK_IMAGE_BASE: &str = "https://www.minecraft.net";
pub const DEFAULT_LIMIT: usize = 5;
pub const MAX_LIMIT: usize = 20;
/// 成功结果在内存里保留多久。
pub const MEMORY_TTL: Duration = Duration::from_secs(30 * 60);
/// 失败后多久不重试。
pub const NEGATIVE_TTL: Duration = Duration::from_secs(5 * 60);
/// 只翻前几条，别把 100 条都发出去。
pub const TRANSLATE_LIMIT: usize = 5;
pub const CACHE_FILE: &str = "news.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub category: String,
    pub date: String,
    pub cover_url: String,
    pub read_more_url: String,
    pub translated: bool,
}

/// news_list 的返回（stale=true 表示用的是上次缓存）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsList {
    pub items: Vec<NewsItem>,
    pub stale: bool,
}

pub fn cache_file(paths: &Paths) -> PathBuf {
    paths.cache.join(CACHE_FILE)
}

/// 常见官方站点语言段（用于把 /en-us/article/... 换成 /zh-hans/article/...）。
pub const LOCALES: [&str; 30] = [
    "en-us", "en-gb", "de-de", "es-es", "es-mx", "fr-fr", "fr-ca", "it-it", "ja-jp", "ko-kr",
    "nl-nl", "pl-pl", "pt-br", "pt-pt", "ru-ru", "sv-se", "tr-tr", "zh-cn", "zh-tw", "zh-hant",
    "cs-cz", "da-dk", "el-gr", "fi-fi", "hu-hu", "id-id", "nb-no", "ro-ro", "sk-sk", "uk-ua",
];

/// 中文文章地址：把 minecraft.net 的文章链接换成简体中文站（阅读全文用）。
///
/// 规则（实测：给 /article/... 插上 /zh-hans 就命中中文站，全部 200）：
/// - 域名必须是 www.minecraft.net 或 minecraft.net；
/// - 路径 /article/x → /zh-hans/article/x；
/// - 路径 /<locale>/article/x → /zh-hans/article/x（列表数据给的是 /en-us/...）；
/// - 已经含 /zh-hans 的原样返回；
/// - 其它域名 / 非文章路径 / 解析失败原样返回；查询串原样保留。
pub fn to_chinese_article_url(url: &str) -> String {
    let trimmed = url.trim();
    let Some(scheme_end) = trimmed.find("://") else {
        return url.to_string();
    };
    let rest = &trimmed[scheme_end + 3..];
    let Some(path_start) = rest.find('/') else {
        return url.to_string();
    };
    let host = &rest[..path_start];
    let host_lower = host.to_ascii_lowercase();
    if host_lower != "www.minecraft.net" && host_lower != "minecraft.net" {
        return url.to_string();
    }
    let path_and_tail = &rest[path_start..];
    let tail_start = path_and_tail
        .find(['?', '#'])
        .unwrap_or(path_and_tail.len());
    let path = &path_and_tail[..tail_start];
    let tail = &path_and_tail[tail_start..];
    if path.contains("/zh-hans") {
        return url.to_string();
    }
    let converted = if let Some(article) = path.strip_prefix("/article/") {
        format!("/zh-hans/article/{article}")
    } else {
        let mut segments = path.trim_start_matches('/').splitn(3, '/');
        let locale = segments.next().unwrap_or("").to_ascii_lowercase();
        let second = segments.next().unwrap_or("");
        let article = segments.next().unwrap_or("");
        if second == "article" && !article.is_empty() && LOCALES.contains(&locale.as_str()) {
            format!("/zh-hans/article/{article}")
        } else {
            return url.to_string();
        }
    };
    // 裸 minecraft.net 会 308 跳到 www，这里统一补成 www（少一次跳转）
    let host = if host_lower == "minecraft.net" {
        "www.minecraft.net"
    } else {
        host
    };
    format!("{}://{host}{converted}{tail}", &trimmed[..scheme_end])
}

/// 图片地址拼绝对（相对路径按给定基址补）。
pub fn absolute_image(url: &str, base: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    let base = base.trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix('/') {
        format!("{base}/{rest}")
    } else {
        format!("{base}/{trimmed}")
    }
}

/// 稳定 id：优先取 readMoreLink 末段，否则标题 sha1。
pub fn news_id(read_more: &str, title: &str) -> String {
    let tail = read_more
        .trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_string();
    if !tail.is_empty() && tail != "www.minecraft.net" && tail.contains('-') {
        return tail;
    }
    let digest = util::sha1_hex(title.as_bytes());
    format!("news-{}", &digest[..12])
}

/// 解析官方 v2 news.json 的 entries 前 limit 条。
pub fn parse_v2(json: &str, limit: usize) -> Result<Vec<NewsItem>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let entries = value
        .get("entries")
        .and_then(|item| item.as_array())
        .ok_or_else(|| AppError::Other("news.json 缺少 entries 字段".to_string()))?;
    Ok(entries
        .iter()
        .take(limit.clamp(1, MAX_LIMIT))
        .filter_map(|entry| {
            let title = entry.get("title").and_then(|i| i.as_str())?.to_string();
            let cover = entry
                .pointer("/newsPageImage/url")
                .and_then(|i| i.as_str())
                .or_else(|| entry.pointer("/playPageImage/url").and_then(|i| i.as_str()))
                .map(|url| absolute_image(url, NEWS_IMAGE_BASE))
                .unwrap_or_default();
            let read_more = entry
                .get("readMoreLink")
                .and_then(|i| i.as_str())
                .filter(|url| !url.trim().is_empty())
                .unwrap_or("https://www.minecraft.net/")
                .to_string();
            Some(NewsItem {
                id: news_id(&read_more, &title),
                title,
                summary: entry
                    .get("text")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string(),
                category: entry
                    .get("category")
                    .and_then(|i| i.as_str())
                    .or_else(|| entry.get("tag").and_then(|i| i.as_str()))
                    .unwrap_or("")
                    .to_string(),
                date: entry
                    .get("date")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string(),
                cover_url: cover,
                read_more_url: to_chinese_article_url(&read_more),
                translated: false,
            })
        })
        .collect())
}

/// 解析站点文章网格（兜底源）。
pub fn parse_fallback(json: &str, limit: usize) -> Result<Vec<NewsItem>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let grid = value
        .get("article_grid")
        .and_then(|item| item.as_array())
        .ok_or_else(|| AppError::Other("兜底新闻源缺少 article_grid".to_string()))?;
    Ok(grid
        .iter()
        .take(limit.clamp(1, MAX_LIMIT))
        .filter_map(|entry| {
            let tile = entry.get("default_tile")?;
            let title = tile.get("title").and_then(|i| i.as_str())?.to_string();
            let read_more = entry
                .get("article_url")
                .and_then(|i| i.as_str())
                .map(|url| absolute_image(url, FALLBACK_IMAGE_BASE))
                .filter(|url| !url.is_empty())
                .unwrap_or_else(|| "https://www.minecraft.net/".to_string());
            Some(NewsItem {
                id: news_id(&read_more, &title),
                title,
                summary: tile
                    .get("sub_header")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string(),
                category: entry
                    .get("primary_category")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string(),
                date: String::new(),
                cover_url: tile
                    .pointer("/image/imageURL")
                    .and_then(|i| i.as_str())
                    .map(|url| absolute_image(url, FALLBACK_IMAGE_BASE))
                    .unwrap_or_default(),
                read_more_url: to_chinese_article_url(&read_more),
                translated: false,
            })
        })
        .collect())
}

/// 磁盘缓存（带抓取时间）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    #[serde(rename = "fetchedAt")]
    fetched_at: u64,
    items: Vec<NewsItem>,
}

pub fn read_cache(path: &Path) -> Option<(u64, Vec<NewsItem>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let parsed: CacheFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    Some((parsed.fetched_at, parsed.items))
}

pub fn write_cache(path: &Path, items: &[NewsItem]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = CacheFile {
        fetched_at: util::now_unix(),
        items: items.to_vec(),
    };
    std::fs::write(path, serde_json::to_string(&payload)?)?;
    Ok(())
}

/// 内存缓存 + 负缓存（每个进程一份）。
#[derive(Default)]
pub struct NewsCache {
    hit: std::sync::Mutex<Option<(Instant, Vec<NewsItem>)>>,
    failed_at: std::sync::Mutex<Option<Instant>>,
}

impl NewsCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn fresh(&self) -> Option<Vec<NewsItem>> {
        self.hit
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|(at, items)| (*at, items.clone())))
            .filter(|(at, _)| at.elapsed() < MEMORY_TTL)
            .map(|(_, items)| items)
    }

    fn remember(&self, items: Vec<NewsItem>) {
        if let Ok(mut guard) = self.hit.lock() {
            *guard = Some((Instant::now(), items));
        }
    }

    fn blocked(&self) -> bool {
        self.failed_at
            .lock()
            .ok()
            .and_then(|guard| *guard)
            .map(|at| at.elapsed() < NEGATIVE_TTL)
            .unwrap_or(false)
    }

    fn mark_failed(&self) {
        if let Ok(mut guard) = self.failed_at.lock() {
            *guard = Some(Instant::now());
        }
    }
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|err| AppError::Net(format!("拉取新闻失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Net(format!(
            "新闻接口返回 HTTP {}",
            response.status().as_u16()
        )));
    }
    response
        .text()
        .await
        .map_err(|err| AppError::Net(format!("读取新闻响应失败：{err}")))
}

/// 拿新闻：内存 30 分钟 → 网络（v2，再兜底站点网格）→ 旧磁盘缓存（stale）。
pub async fn list(
    client: &reqwest::Client,
    paths: &Paths,
    cache: &NewsCache,
    translator: Option<(&Settings, &BuiltinTranslator)>,
    limit: usize,
) -> NewsList {
    let limit = limit.clamp(1, MAX_LIMIT);
    if let Some(items) = cache.fresh() {
        return NewsList {
            items: items.into_iter().take(limit).collect(),
            stale: false,
        };
    }
    let disk = read_cache(&cache_file(paths));
    if cache.blocked() {
        return match disk {
            Some((_, items)) => NewsList {
                items: items.into_iter().take(limit).collect(),
                stale: true,
            },
            None => NewsList {
                items: Vec::new(),
                stale: true,
            },
        };
    }

    let mut items: Vec<NewsItem> = Vec::new();
    match fetch_text(client, NEWS_URL).await {
        Ok(text) => match parse_v2(&text, limit) {
            Ok(parsed) if !parsed.is_empty() => items = parsed,
            Ok(_) => tracing::warn!("news.json 解析为空"),
            Err(err) => tracing::warn!(%err, "news.json 解析失败"),
        },
        Err(err) => tracing::warn!(%err, "拉取官方新闻失败，改用兜底源"),
    }
    if items.is_empty() {
        match fetch_text(client, FALLBACK_URL).await {
            Ok(text) => match parse_fallback(&text, limit) {
                Ok(parsed) => items = parsed,
                Err(err) => tracing::warn!(%err, "兜底新闻源解析失败"),
            },
            Err(err) => tracing::warn!(%err, "兜底新闻源也失败"),
        }
    }
    if items.is_empty() {
        cache.mark_failed();
        return match disk {
            Some((_, items)) => NewsList {
                items: items.into_iter().take(limit).collect(),
                stale: true,
            },
            None => NewsList {
                items: Vec::new(),
                stale: true,
            },
        };
    }

    let _ = write_cache(&cache_file(paths), &items);
    cache.remember(items.clone());

    // 翻译前 N 条（失败保留原文）
    if let Some((settings, translator)) = translator {
        let mut texts: Vec<String> = Vec::new();
        for item in items.iter().take(TRANSLATE_LIMIT) {
            texts.push(item.title.clone());
            texts.push(item.summary.clone());
        }
        let keys = cache_key(settings);
        let outcome = translator
            .translate(
                client,
                &paths.cache.join(crate::translate::CACHE_FILE),
                &keys,
                &texts,
                "zh-CN",
            )
            .await;
        for (index, item) in items.iter_mut().take(TRANSLATE_LIMIT).enumerate() {
            let title = outcome.results.get(index * 2).cloned().flatten();
            let summary = outcome.results.get(index * 2 + 1).cloned().flatten();
            let mut changed = false;
            if let Some(title) = title {
                item.title = title;
                changed = true;
            }
            if let Some(summary) = summary {
                item.summary = summary;
                changed = true;
            }
            item.translated = changed;
        }
    }

    NewsList {
        items: items.into_iter().take(limit).collect(),
        stale: false,
    }
}

fn cache_key(settings: &Settings) -> String {
    settings.deepl_api_key.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"version":1,"entries":[
        {"title":"故事","tag":"Story","category":"Minecraft","date":"2026-10-01","text":"一段介绍",
         "playPageImage":{"url":"/v2/images/play.jpg"},
         "newsPageImage":{"url":"/v2/images/news.jpg","dimensions":{"width":700,"height":466}},
         "readMoreLink":"https://www.minecraft.net/en-us/article/holiday-update"},
        {"title":"没有新闻图的条目","tag":"News","category":"Minecraft","date":"2026-10-02","text":"x",
         "playPageImage":{"url":"/v2/images/play2.jpg"}},
        {"title":"三","text":"y"},
        {"title":"四","text":"z"},
        {"title":"五","text":"w"},
        {"title":"六","text":"v"}
    ]}"#;

    #[test]
    fn parses_v2_with_image_fallbacks() {
        let items = parse_v2(SAMPLE, 3).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "故事");
        assert_eq!(items[0].category, "Minecraft");
        assert_eq!(
            items[0].cover_url,
            "https://launchercontent.mojang.com/v2/images/news.jpg"
        );
        // 缺 newsPageImage 时回落到 playPageImage
        assert_eq!(
            items[1].cover_url,
            "https://launchercontent.mojang.com/v2/images/play2.jpg"
        );
        // 完全没有图 → 空串
        assert_eq!(items[2].cover_url, "");
        assert_eq!(
            items[0].read_more_url,
            "https://www.minecraft.net/zh-hans/article/holiday-update",
            "阅读全文要指向中文站"
        );
        assert!(!items[0].translated);
    }

    #[test]
    fn read_more_defaults_and_id_stability() {
        let items = parse_v2(SAMPLE, 6).unwrap();
        assert_eq!(items[2].read_more_url, "https://www.minecraft.net/", "缺链接要回落官网");
        // 同一标题与链接 → 同一 id
        let again = parse_v2(SAMPLE, 6).unwrap();
        assert_eq!(items[0].id, again[0].id);
        assert_eq!(items[0].id, "holiday-update", "取链接末段");
        assert!(items[2].id.starts_with("news-"), "没链接时用标题哈希：{}", items[2].id);
        assert_ne!(items[2].id, items[3].id);
    }

    #[test]
    fn limit_is_bounded() {
        assert_eq!(parse_v2(SAMPLE, 0).unwrap().len(), 1, "0 也要给一条");
        assert_eq!(parse_v2(SAMPLE, 99).unwrap().len(), 6, "最多返回实际条数");
        let many = format!(
            r#"{{"entries":[{}]}}"#,
            (0..30)
                .map(|i| format!(r#"{{"title":"t{i}","text":"x"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(parse_v2(&many, 99).unwrap().len(), MAX_LIMIT);
    }

    #[test]
    fn parses_fallback_grid() {
        let json = r#"{"article_grid":[
            {"default_tile":{"title":"标题","sub_header":"副标题","image":{"imageURL":"/content/dam/x.jpg"}},
             "primary_category":"News","article_url":"/en-us/article/abc-def"}
        ]}"#;
        let items = parse_fallback(json, 5).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "标题");
        assert_eq!(items[0].summary, "副标题");
        assert_eq!(
            items[0].cover_url,
            "https://www.minecraft.net/content/dam/x.jpg"
        );
        assert_eq!(
            items[0].read_more_url,
            "https://www.minecraft.net/zh-hans/article/abc-def"
        );
        assert_eq!(items[0].id, "abc-def");
        assert!(parse_fallback("{}", 5).is_err());
    }


    #[test]
    fn chinese_article_url_inserts_zh_hans() {
        assert_eq!(
            to_chinese_article_url("https://www.minecraft.net/article/dominos-minecraft-dungeons-ii"),
            "https://www.minecraft.net/zh-hans/article/dominos-minecraft-dungeons-ii"
        );
        // 列表数据里的 /en-us/ 段要换成 zh-hans
        assert_eq!(
            to_chinese_article_url("https://www.minecraft.net/en-us/article/abc-def"),
            "https://www.minecraft.net/zh-hans/article/abc-def"
        );
        // 不带 www 的域名也认，并补成 www（裸域会 308 跳转）
        assert_eq!(
            to_chinese_article_url("https://minecraft.net/en-us/article/avatar-legends-add-on"),
            "https://www.minecraft.net/zh-hans/article/avatar-legends-add-on"
        );
    }

    #[test]
    fn chinese_article_url_keeps_query_string() {
        assert_eq!(
            to_chinese_article_url(
                "https://www.minecraft.net/article/marketplace-spotlight-october-2026?OCID=Launcher"
            ),
            "https://www.minecraft.net/zh-hans/article/marketplace-spotlight-october-2026?OCID=Launcher"
        );
    }

    #[test]
    fn chinese_article_url_is_idempotent_for_zh_hans() {
        let already = "https://www.minecraft.net/zh-hans/article/spooky-showcase-challenge-winners";
        assert_eq!(to_chinese_article_url(already), already);
        let already_query = "https://www.minecraft.net/zh-hans/article/x?OCID=Launcher";
        assert_eq!(to_chinese_article_url(already_query), already_query);
    }

    #[test]
    fn chinese_article_url_leaves_other_hosts_alone() {
        for url in [
            "https://launchercontent.mojang.com/v2/news.json",
            "https://example.com/article/x",
            "https://help.minecraft.net/article/x",
        ] {
            assert_eq!(to_chinese_article_url(url), url);
        }
    }

    #[test]
    fn chinese_article_url_leaves_non_article_paths_alone() {
        for url in [
            "https://www.minecraft.net/",
            "https://www.minecraft.net/zh-hans",
            "https://www.minecraft.net/store/article-grid",
            "https://www.minecraft.net/en-us/news",
        ] {
            assert_eq!(to_chinese_article_url(url), url);
        }
    }

    #[test]
    fn chinese_article_url_survives_malformed_input() {
        for url in ["", "   ", "not a url", "https://", "https://www.minecraft.net", "//host/article/x"] {
            assert_eq!(to_chinese_article_url(url), url);
        }
        // 前后空白不影响判断
        assert_eq!(
            to_chinese_article_url("  https://www.minecraft.net/article/x  "),
            "https://www.minecraft.net/zh-hans/article/x"
        );
    }

    #[test]
    fn image_absolutization() {
        assert_eq!(
            absolute_image("/v2/images/a.jpg", NEWS_IMAGE_BASE),
            "https://launchercontent.mojang.com/v2/images/a.jpg"
        );
        assert_eq!(
            absolute_image("v2/images/a.jpg", NEWS_IMAGE_BASE),
            "https://launchercontent.mojang.com/v2/images/a.jpg"
        );
        assert_eq!(absolute_image("https://cdn/a.jpg", NEWS_IMAGE_BASE), "https://cdn/a.jpg");
        assert_eq!(absolute_image("", NEWS_IMAGE_BASE), "");
    }

    #[test]
    fn cache_roundtrip_and_bom_tolerance() {
        let root = std::env::temp_dir().join(format!("gugu-news-{}", util::unique_suffix()));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        let file = cache_file(&paths);
        assert!(read_cache(&file).is_none());
        let items = parse_v2(SAMPLE, 2).unwrap();
        write_cache(&file, &items).unwrap();
        let (at, loaded) = read_cache(&file).unwrap();
        assert!(at > 0);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].title, "故事");
        // BOM 也要能读
        let text = format!("\u{feff}{}", std::fs::read_to_string(&file).unwrap());
        std::fs::write(&file, text).unwrap();
        assert!(read_cache(&file).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn memory_and_negative_cache_behaviour() {
        let cache = NewsCache::new();
        assert!(cache.fresh().is_none());
        assert!(!cache.blocked());
        let items = parse_v2(SAMPLE, 2).unwrap();
        cache.remember(items.clone());
        assert_eq!(cache.fresh().unwrap().len(), 2);
        cache.mark_failed();
        assert!(cache.blocked(), "失败后 5 分钟内不再请求");
    }

    #[test]
    fn expired_memory_cache_is_miss() {
        let cache = NewsCache::new();
        if let Ok(mut guard) = cache.hit.lock() {
            // 手工塞一个「31 分钟前」的条目
            *guard = Some((
                Instant::now() - MEMORY_TTL - Duration::from_secs(1),
                parse_v2(SAMPLE, 1).unwrap(),
            ));
        }
        assert!(cache.fresh().is_none());
    }
}
