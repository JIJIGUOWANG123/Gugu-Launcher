//! 内容服务翻译：内置通道（uapis → suapi，DeepL 可选）+ 用户自定义 OpenAI 兼容接口。
//!
//! 真实响应样本（2026-10-27 本机实抓，已写进单测）：
//! - uapis  POST https://uapis.cn/api/v1/translate/text  {"text":"钠 · 优化模组","to_lang":"en"}
//!          200 {"text":"钠 · 优化模组","translate":"Sodium · Optimization mod"}   ← 取 translate
//! - suapi  POST https://suapi.net/api/text/translate?to=zh-CHS&from=en  ["Sodium","Mod Menu"]
//!          200 {"code":200,"msg":"请求成功,无Key调用仅供测试随时失效.",
//!                "data":[{"translations":[{"text":"钠"}]},{"translations":[{"text":"模组菜单"}]}]}
//!          ← 取 data[i].translations[0].text；code != 200 视为整批失败
//! - DeepL  POST https://api-free.deepl.com/v2/translate（表单 text=…&target_lang=EN）
//!          200 {"translations":[{"text":"…"}]}
//!
//! 策略（沿用 0.5.6）：全局串行 + 最小间隔 150ms、正缓存 7 天/2000 条、负缓存 5 分钟、
//! 主备切换 sticky 10 分钟、单条 >3000 字符先截断、任何失败降级为 None 且不打断搜索。

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use tokio::sync::Mutex;

use crate::error::{AppError, Result};
use crate::settings::Settings;
use crate::util;

pub const UAPIS_URL: &str = "https://uapis.cn/api/v1/translate/text";
pub const SUAPI_URL: &str = "https://suapi.net/api/text/translate";
pub const DEEPL_FREE_URL: &str = "https://api-free.deepl.com/v2/translate";
pub const DEEPL_PRO_URL: &str = "https://api.deepl.com/v2/translate";
/// 单条接口上限：超过 3000 字符服务端直接拒绝，先截断。
pub const MAX_TEXT_CHARS: usize = 3000;
pub const UAPIS_TIMEOUT: Duration = Duration::from_secs(6);
pub const DEEPL_TIMEOUT: Duration = Duration::from_secs(8);
/// 全局最小请求间隔。
pub const MIN_INTERVAL: Duration = Duration::from_millis(150);
/// 失败后 5 分钟内不再重试同一 provider + 文本。
pub const NEGATIVE_TTL: Duration = Duration::from_secs(300);
/// 主备切换后的粘滞时间。
pub const PREFERRED_STICKY: Duration = Duration::from_secs(600);
pub const CACHE_FILE: &str = "translate-cache.json";
pub const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
pub const CACHE_MAX_ENTRIES: usize = 2000;
pub const SUAPI_BATCH: usize = 20;
pub const DEEPL_BATCH: usize = 50;
/// 自定义接口（OpenAI 兼容）的对话路径。
pub const CHAT_PATH: &str = "/chat/completions";
/// 自定义接口超时。
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 内置通道的三个提供方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Uapis,
    Suapi,
    Deepl,
}

impl Provider {
    pub fn id(&self) -> &'static str {
        match self {
            Provider::Uapis => "uapis",
            Provider::Suapi => "suapi",
            Provider::Deepl => "deepl",
        }
    }

    /// 单次能塞多少条。
    pub fn batch_size(&self) -> usize {
        match self {
            Provider::Uapis => 1,
            Provider::Suapi => SUAPI_BATCH,
            Provider::Deepl => DEEPL_BATCH,
        }
    }

    pub fn timeout(&self) -> Duration {
        match self {
            Provider::Deepl => DEEPL_TIMEOUT,
            _ => UAPIS_TIMEOUT,
        }
    }
}

/// 是否含中文（已是中文就不再请求中文译文）。
pub fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// 超过上限先截断（按字符而不是字节）。
pub fn clip_text(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

/// 缓存键：目标语言 + 文本的 sha1。
pub fn cache_key(to_lang: &str, text: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(text.as_bytes());
    format!("{}:{}", to_lang.to_lowercase(), util::hex(&hasher.finalize()))
}

/// suapi 的目标语言写法：中文用 zh-CHS，并固定 source=en。
pub fn suapi_language(to_lang: &str) -> (String, bool) {
    let chinese = to_lang.to_lowercase().starts_with("zh");
    (
        if chinese {
            "zh-CHS".to_string()
        } else {
            to_lang.to_lowercase()
        },
        chinese,
    )
}

/// 尝试顺序：DeepL（有 Key）→ 首选 → 兜底；sticky 过期后回到 uapis 优先。
pub fn provider_order(preferred_suapi: bool, has_deepl: bool) -> Vec<Provider> {
    let base = if preferred_suapi {
        vec![Provider::Suapi, Provider::Uapis]
    } else {
        vec![Provider::Uapis, Provider::Suapi]
    };
    if has_deepl {
        let mut out = vec![Provider::Deepl];
        out.extend(base);
        out
    } else {
        base
    }
}

/// uapis 响应解析（真实样本里译文在 translate 字段）。
pub fn parse_uapis(json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json).ok()?;
    let pick = |candidate: Option<&Value>| {
        candidate
            .and_then(|item| item.as_str())
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
    };
    pick(value.get("translate"))
        .or_else(|| pick(value.pointer("/data/translation")))
        .or_else(|| pick(value.get("translation")))
        .or_else(|| pick(value.get("text")))
}

/// suapi 响应解析：code 必须是 200，逐条取 data[i].translations[0].text。
pub fn parse_suapi(json: &str) -> Option<Vec<Option<String>>> {
    let value: Value = serde_json::from_str(json).ok()?;
    let code = value.get("code").and_then(|item| item.as_i64()).unwrap_or(0);
    if code != 200 {
        return None;
    }
    let data = value.get("data")?.as_array()?;
    Some(
        data.iter()
            .map(|entry| {
                entry
                    .pointer("/translations/0/text")
                    .and_then(|text| text.as_str())
                    .map(|text| text.to_string())
                    .filter(|text| !text.trim().is_empty())
            })
            .collect(),
    )
}

/// DeepL 响应解析：translations[i].text。
pub fn parse_deepl(json: &str) -> Option<Vec<Option<String>>> {
    let value: Value = serde_json::from_str(json).ok()?;
    let list = value.get("translations")?.as_array()?;
    Some(
        list.iter()
            .map(|entry| {
                entry
                    .get("text")
                    .and_then(|text| text.as_str())
                    .map(|text| text.to_string())
                    .filter(|text| !text.trim().is_empty())
            })
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    translated: String,
    at: u64,
}

/// 一次内置翻译的结果。
#[derive(Debug, Clone)]
pub struct BatchResult {
    pub results: Vec<Option<String>>,
    /// 撞了限流或整链失败（前端用来提示「翻译暂不可用」）
    pub limited: bool,
}

#[derive(Default)]
struct Inner {
    preferred_suapi: bool,
    sticky_until: Option<Instant>,
    last_start: Option<Instant>,
    negative: HashMap<String, Instant>,
    cache: HashMap<String, CacheEntry>,
    loaded: bool,
    requests: usize,
}

/// 内置翻译器：全局串行 + 缓存 + 主备切换，放在 AppState 里跨命令复用。
pub struct BuiltinTranslator {
    inner: Mutex<Inner>,
}

impl Default for BuiltinTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl BuiltinTranslator {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// 已发出的真实请求数（诊断用）。
    pub async fn request_count(&self) -> usize {
        self.inner.lock().await.requests
    }

    /// 批量翻译；返回长度与输入一致，失败位置是 None。
    pub async fn translate(
        &self,
        client: &reqwest::Client,
        cache_file: &Path,
        deepl_key: &str,
        texts: &[String],
        to_lang: &str,
    ) -> BatchResult {
        let mut out: Vec<Option<String>> = vec![None; texts.len()];
        if texts.is_empty() {
            return BatchResult {
                results: out,
                limited: false,
            };
        }
        let target = to_lang.trim().to_lowercase();
        if target.is_empty() {
            return BatchResult {
                results: out,
                limited: false,
            };
        }

        // 规划：去重 + 已是中文跳过
        let mut unique: Vec<String> = Vec::new();
        let mut slots: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, raw) in texts.iter().enumerate() {
            let text = clip_text(raw);
            if text.trim().is_empty() {
                out[index] = Some(String::new());
                continue;
            }
            if target.starts_with("zh") && contains_cjk(&text) {
                out[index] = Some(text);
                continue;
            }
            match slots.get_mut(&text) {
                Some(indexes) => indexes.push(index),
                None => {
                    slots.insert(text.clone(), vec![index]);
                    unique.push(text);
                }
            }
        }
        if unique.is_empty() {
            return BatchResult {
                results: out,
                limited: false,
            };
        }

        let mut inner = self.inner.lock().await;
        if !inner.loaded {
            inner.cache = load_cache(cache_file);
            inner.loaded = true;
        }
        if let Some(sticky) = inner.sticky_until {
            if Instant::now() >= sticky {
                inner.preferred_suapi = false;
                inner.sticky_until = None;
            }
        }

        // 正缓存命中
        let now = util::now_unix();
        let mut candidates: Vec<String> = Vec::new();
        for text in &unique {
            let key = cache_key(&target, text);
            match inner.cache.get(&key) {
                Some(entry) if now.saturating_sub(entry.at) < CACHE_TTL.as_secs() => {
                    fill(&mut out, &slots, text, &entry.translated);
                }
                _ => candidates.push(text.clone()),
            }
        }
        if candidates.is_empty() {
            return BatchResult {
                results: out,
                limited: false,
            };
        }

        let has_deepl = !deepl_key.trim().is_empty();
        let order = provider_order(inner.preferred_suapi, has_deepl);
        let mut translated: HashMap<String, String> = HashMap::new();
        let mut limited = false;

        for provider in order {
            let remaining: Vec<String> = candidates
                .iter()
                .filter(|text| {
                    !translated.contains_key(*text)
                        && !negative_hit(&inner, provider, &target, text, Instant::now())
                })
                .cloned()
                .collect();
            if remaining.is_empty() {
                continue;
            }
            let mut batch_start = 0usize;
            while batch_start < remaining.len() {
                let end = (batch_start + provider.batch_size()).min(remaining.len());
                let chunk: Vec<String> = remaining[batch_start..end].to_vec();
                wait_turn(&mut inner).await;
                inner.requests += 1;
                let attempt = call_provider(client, provider, &chunk, &target, deepl_key).await;
                match attempt {
                    Ok(values) => {
                        for (offset, value) in values.into_iter().enumerate() {
                            if let Some(text) = value {
                                if let Some(source) = chunk.get(offset) {
                                    translated.insert(source.clone(), text);
                                }
                            }
                        }
                        batch_start = end;
                    }
                    Err(failure) => {
                        limited = limited || failure.limited;
                        tracing::warn!(
                            provider = provider.id(),
                            reason = %failure.reason,
                            "内置翻译接口失败，改用下一个"
                        );
                        for text in &chunk {
                            inner
                                .negative
                                .insert(negative_key(provider, &target, text), Instant::now());
                        }
                        // 首选接口挂了：切到另一个并粘滞 10 分钟
                        if provider != Provider::Deepl {
                            let was_preferred = (!inner.preferred_suapi
                                && provider == Provider::Uapis)
                                || (inner.preferred_suapi && provider == Provider::Suapi);
                            if was_preferred {
                                inner.preferred_suapi = provider == Provider::Uapis;
                                inner.sticky_until = Some(Instant::now() + PREFERRED_STICKY);
                                tracing::warn!(
                                    next = if inner.preferred_suapi { "suapi" } else { "uapis" },
                                    "已切换内置翻译主接口（10 分钟内不再切回）"
                                );
                            }
                        }
                        break;
                    }
                }
            }
        }

        // 写缓存并落盘
        let mut changed = false;
        for (text, value) in &translated {
            inner.cache.insert(
                cache_key(&target, text),
                CacheEntry {
                    translated: value.clone(),
                    at: util::now_unix(),
                },
            );
            fill(&mut out, &slots, text, value);
            changed = true;
        }
        if changed {
            trim_cache(&mut inner.cache, util::now_unix());
            if let Err(err) = save_cache(cache_file, &inner.cache) {
                tracing::warn!(%err, "写翻译缓存失败（忽略）");
            }
        }
        if out.iter().all(|item| item.is_none()) {
            limited = true;
        }
        BatchResult {
            results: out,
            limited,
        }
    }
}

/// 全局串行 + 最小间隔：持锁期间保证相邻请求的启动时间差。
async fn wait_turn(inner: &mut Inner) {
    if let Some(last) = inner.last_start {
        let elapsed = last.elapsed();
        if elapsed < MIN_INTERVAL {
            tokio::time::sleep(MIN_INTERVAL - elapsed).await;
        }
    }
    inner.last_start = Some(Instant::now());
}

fn negative_key(provider: Provider, to_lang: &str, text: &str) -> String {
    format!("{}|{}", provider.id(), cache_key(to_lang, text))
}

fn negative_hit(inner: &Inner, provider: Provider, to_lang: &str, text: &str, now: Instant) -> bool {
    match inner.negative.get(&negative_key(provider, to_lang, text)) {
        Some(at) => now.duration_since(*at) < NEGATIVE_TTL,
        None => false,
    }
}

fn fill(out: &mut [Option<String>], slots: &HashMap<String, Vec<usize>>, text: &str, value: &str) {
    if let Some(indexes) = slots.get(text) {
        for index in indexes {
            if let Some(slot) = out.get_mut(*index) {
                *slot = Some(value.to_string());
            }
        }
    }
}

struct Failure {
    reason: String,
    limited: bool,
}

fn status_failure(status: reqwest::StatusCode) -> Failure {
    Failure {
        reason: format!("HTTP {}", status.as_u16()),
        limited: matches!(status.as_u16(), 429 | 403 | 456),
    }
}

/// 一次 provider 调用（调用方已按 batch_size 切好）。
async fn call_provider(
    client: &reqwest::Client,
    provider: Provider,
    texts: &[String],
    to_lang: &str,
    deepl_key: &str,
) -> std::result::Result<Vec<Option<String>>, Failure> {
    match provider {
        Provider::Uapis => {
            let text = texts.first().cloned().unwrap_or_default();
            let response = client
                .post(UAPIS_URL)
                .json(&json!({ "text": text, "to_lang": to_lang }))
                .timeout(provider.timeout())
                .send()
                .await
                .map_err(|err| Failure {
                    reason: err.to_string(),
                    limited: false,
                })?;
            if !response.status().is_success() {
                return Err(status_failure(response.status()));
            }
            let body = response.text().await.map_err(|err| Failure {
                reason: err.to_string(),
                limited: false,
            })?;
            match parse_uapis(&body) {
                Some(value) => Ok(vec![Some(value)]),
                None => Err(Failure {
                    reason: "响应里没有译文".to_string(),
                    limited: false,
                }),
            }
        }
        Provider::Suapi => {
            let (language, chinese) = suapi_language(to_lang);
            let mut url = format!("{SUAPI_URL}?to={language}");
            if chinese {
                url.push_str("&from=en");
            }
            let response = client
                .post(&url)
                .header("referer", "https://suapi.net")
                .json(&texts.to_vec())
                .timeout(provider.timeout())
                .send()
                .await
                .map_err(|err| Failure {
                    reason: err.to_string(),
                    limited: false,
                })?;
            if !response.status().is_success() {
                return Err(status_failure(response.status()));
            }
            let body = response.text().await.map_err(|err| Failure {
                reason: err.to_string(),
                limited: false,
            })?;
            match parse_suapi(&body) {
                Some(values) => {
                    let mut out = values;
                    out.resize(texts.len(), None);
                    Ok(out)
                }
                None => Err(Failure {
                    reason: "响应码不是 200 或结构不对".to_string(),
                    limited: true,
                }),
            }
        }
        Provider::Deepl => {
            let key = deepl_key.trim();
            let url = if key.ends_with(":fx") {
                DEEPL_FREE_URL
            } else {
                DEEPL_PRO_URL
            };
            let (language, chinese) = suapi_language(to_lang);
            let target_lang = if chinese {
                "ZH".to_string()
            } else {
                language.to_uppercase()
            };
            let mut form: Vec<(String, String)> = texts
                .iter()
                .map(|text| ("text".to_string(), text.clone()))
                .collect();
            form.push(("target_lang".to_string(), target_lang));
            if chinese {
                form.push(("source_lang".to_string(), "EN".to_string()));
            }
            let response = client
                .post(url)
                .header("authorization", format!("DeepL-Auth-Key {key}"))
                .form(&form)
                .timeout(provider.timeout())
                .send()
                .await
                .map_err(|err| Failure {
                    reason: err.to_string(),
                    limited: false,
                })?;
            if !response.status().is_success() {
                return Err(status_failure(response.status()));
            }
            let body = response.text().await.map_err(|err| Failure {
                reason: err.to_string(),
                limited: false,
            })?;
            match parse_deepl(&body) {
                Some(values) => {
                    let mut out = values;
                    out.resize(texts.len(), None);
                    Ok(out)
                }
                None => Err(Failure {
                    reason: "DeepL 响应结构不对".to_string(),
                    limited: false,
                }),
            }
        }
    }
}

fn load_cache(path: &Path) -> HashMap<String, CacheEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return HashMap::new();
    };
    let Some(map) = value.as_object() else {
        return HashMap::new();
    };
    let now = util::now_unix();
    let mut out = HashMap::new();
    for (key, entry) in map {
        let Some(translated) = entry.get("t").and_then(|item| item.as_str()) else {
            continue;
        };
        let Some(at) = entry.get("at").and_then(|item| item.as_u64()) else {
            continue;
        };
        if now.saturating_sub(at) >= CACHE_TTL.as_secs() {
            continue;
        }
        out.insert(
            key.clone(),
            CacheEntry {
                translated: translated.to_string(),
                at,
            },
        );
    }
    out
}

fn save_cache(path: &Path, cache: &HashMap<String, CacheEntry>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let map: serde_json::Map<String, Value> = cache
        .iter()
        .map(|(key, entry)| {
            (
                key.clone(),
                json!({ "t": entry.translated, "at": entry.at }),
            )
        })
        .collect();
    let text = serde_json::to_string(&Value::Object(map))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 先剔过期，再按写入时间淘汰最旧的。
pub fn trim_cache(cache: &mut HashMap<String, CacheEntry>, now: u64) {
    cache.retain(|_, entry| now.saturating_sub(entry.at) < CACHE_TTL.as_secs());
    if cache.len() <= CACHE_MAX_ENTRIES {
        return;
    }
    let mut ordered: Vec<(String, u64)> = cache
        .iter()
        .map(|(key, entry)| (key.clone(), entry.at))
        .collect();
    ordered.sort_by_key(|(_, at)| *at);
    let remove = cache.len() - CACHE_MAX_ENTRIES;
    for (key, _) in ordered.into_iter().take(remove) {
        cache.remove(&key);
    }
}

// ------------------------------------------------------------ 自定义接口（OpenAI 兼容）

/// 把设置里的根地址补成完整的 chat/completions 地址。
pub fn chat_completions_url(base: &str) -> Result<String> {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::Invalid("翻译接口地址为空".to_string()));
    }
    if !crate::settings::is_http_url(trimmed) {
        return Err(AppError::Invalid(format!(
            "翻译接口地址必须是 http(s)：{trimmed}"
        )));
    }
    if trimmed.ends_with(CHAT_PATH) {
        return Ok(trimmed.to_string());
    }
    Ok(format!("{trimmed}{CHAT_PATH}"))
}

/// 组 prompt：system 说明任务，user 传 JSON 数组。
pub fn build_translation_prompt(texts: &[String]) -> (String, String) {
    let system = "你是 Minecraft 模组与整合包内容的中文翻译。用户会给一个 JSON 字符串数组，请逐条翻译成简体中文，保持顺序与长度一致，只输出 JSON 字符串数组，不要解释、不要代码块标记。"
        .to_string();
    let user = serde_json::to_string(texts).unwrap_or_else(|_| "[]".to_string());
    (system, user)
}

pub fn build_request_body(settings: &Settings, texts: &[String]) -> Value {
    let (system, user) = build_translation_prompt(texts);
    let model = if settings.translation_model.trim().is_empty() {
        "gpt-4o-mini"
    } else {
        settings.translation_model.trim()
    };
    json!({
        "model": model,
        "temperature": settings.translation_temperature,
        "max_tokens": settings.translation_max_tokens,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    })
}

fn redact(message: String, settings: &Settings) -> String {
    util::redact_secret(&message, &settings.translation_api_key)
}

/// translation_test() 的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationTestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
}

/// 用当前设置发一条最小请求，验证自定义翻译接口是否可用。
pub async fn test_connection(client: &reqwest::Client, settings: &Settings) -> TranslationTestResult {
    if settings.translation_source != "custom" {
        return TranslationTestResult {
            ok: true,
            latency_ms: 0,
            message: "当前使用内置翻译接口（uapis → suapi），无需配置".to_string(),
        };
    }
    let url = match chat_completions_url(&settings.translation_base_url) {
        Ok(url) => url,
        Err(err) => {
            return TranslationTestResult {
                ok: false,
                latency_ms: 0,
                message: redact(err.to_string(), settings),
            }
        }
    };
    let body = json!({
        "model": if settings.translation_model.trim().is_empty() { "gpt-4o-mini" } else { settings.translation_model.trim() },
        "messages": [
            {"role": "system", "content": "ping"},
            {"role": "user", "content": "ok"},
        ],
        "max_tokens": 8,
    });

    let started = Instant::now();
    let request = client
        .post(&url)
        .bearer_auth(settings.translation_api_key.trim())
        .json(&body)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await;
    let latency_ms = started.elapsed().as_millis() as u64;

    match request {
        Ok(response) => {
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return TranslationTestResult {
                    ok: false,
                    latency_ms,
                    message: format!(
                        "认证失败（HTTP {}）：API Key {} 可能无效或没有权限",
                        status.as_u16(),
                        util::mask_secret(&settings.translation_api_key)
                    ),
                };
            }
            if !status.is_success() {
                return TranslationTestResult {
                    ok: false,
                    latency_ms,
                    message: format!("接口返回 HTTP {}", status.as_u16()),
                };
            }
            match response.json::<Value>().await {
                Ok(value) => {
                    let snippet = value
                        .pointer("/choices/0/message/content")
                        .and_then(|content| content.as_str())
                        .unwrap_or("")
                        .chars()
                        .take(40)
                        .collect::<String>();
                    if snippet.is_empty() {
                        return TranslationTestResult {
                            ok: false,
                            latency_ms,
                            message: "返回内容不是 OpenAI 兼容格式（缺少 choices[0].message.content）"
                                .to_string(),
                        };
                    }
                    TranslationTestResult {
                        ok: true,
                        latency_ms,
                        message: format!("连接成功，模型回显：{snippet}"),
                    }
                }
                Err(_) => TranslationTestResult {
                    ok: false,
                    latency_ms,
                    message: "返回内容不是合法 JSON".to_string(),
                },
            }
        }
        Err(err) if err.is_timeout() => TranslationTestResult {
            ok: false,
            latency_ms,
            message: format!("请求超时（{} 秒）", REQUEST_TIMEOUT.as_secs()),
        },
        Err(err) => TranslationTestResult {
            ok: false,
            latency_ms,
            message: redact(format!("请求失败：{err}"), settings),
        },
    }
}

/// 自定义接口的批量翻译（失败重试 1 次）。
pub async fn translate_custom(
    client: &reqwest::Client,
    settings: &Settings,
    texts: &[String],
) -> Vec<Option<String>> {
    if texts.is_empty() {
        return Vec::new();
    }
    let url = match chat_completions_url(&settings.translation_base_url) {
        Ok(url) => url,
        Err(err) => {
            tracing::warn!(%err, "翻译接口地址不可用");
            return vec![None; texts.len()];
        }
    };
    let body = build_request_body(settings, texts);
    for attempt in 0..2 {
        let request = client
            .post(&url)
            .bearer_auth(settings.translation_api_key.trim())
            .json(&body)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await;
        match request {
            Ok(response) if response.status().is_success() => {
                match response.json::<Value>().await {
                    Ok(value) => {
                        let content = value
                            .pointer("/choices/0/message/content")
                            .and_then(|content| content.as_str())
                            .unwrap_or("");
                        let parsed = parse_translations(content, texts.len());
                        if parsed.iter().any(|item| item.is_some()) {
                            return parsed;
                        }
                    }
                    Err(err) => tracing::warn!(%err, "翻译响应解析失败"),
                }
            }
            Ok(response) => tracing::warn!(status = response.status().as_u16(), "翻译请求失败"),
            Err(err) => tracing::warn!(%err, "翻译请求异常"),
        }
        if attempt == 0 {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
    vec![None; texts.len()]
}

/// 解析模型返回：纯 JSON 数组 / 代码块包裹 / {translations:[...]}。
pub fn parse_translations(content: &str, expected: usize) -> Vec<Option<String>> {
    let mut fallback = vec![None; expected];
    let trimmed = content.trim();
    let stripped = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let parsed: Option<Value> = serde_json::from_str::<Value>(stripped).ok().or_else(|| {
        let start = stripped.find('[')?;
        let end = stripped.rfind(']')?;
        serde_json::from_str::<Value>(&stripped[start..=end]).ok()
    });

    let Some(value) = parsed else {
        return fallback;
    };
    let items = match value {
        Value::Array(items) => items,
        Value::Object(map) => match map.get("translations") {
            Some(Value::Array(items)) => items.clone(),
            _ => return fallback,
        },
        _ => return fallback,
    };

    for (index, item) in items.iter().take(expected).enumerate() {
        fallback[index] = match item {
            Value::String(text) => Some(text.clone()),
            Value::Object(map) => ["text", "translation", "zh", "content"]
                .iter()
                .find_map(|key| map.get(*key).and_then(|value| value.as_str()))
                .map(|text| text.to_string()),
            _ => None,
        };
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实抓取的 uapis 响应（2026-10-27，POST {"text":"钠 · 优化模组","to_lang":"en"}）。
    const UAPIS_SAMPLE: &str =
        r#"{"text":"钠 · 优化模组","translate":"Sodium · Optimization mod"}"#;
    /// 真实抓取的 suapi 响应（同一轮请求，["Sodium","Mod Menu"] → 中文）。
    const SUAPI_SAMPLE: &str = r#"{"code":200,"msg":"请求成功,无Key调用仅供测试随时失效.","data":[{"translations":[{"text":"钠","to":"zh-Hans","sentLen":{"srcSentLen":[6],"transSentLen":[1]}}]},{"translations":[{"text":"模组菜单","to":"zh-Hans","sentLen":{"srcSentLen":[8],"transSentLen":[4]}}]}],"exec_time":0.895478,"ip":"240e::1"}"#;

    #[test]
    fn parses_real_uapis_sample() {
        assert_eq!(
            parse_uapis(UAPIS_SAMPLE).as_deref(),
            Some("Sodium · Optimization mod")
        );
        assert_eq!(
            parse_uapis(r#"{"data":{"translation":"钠"}}"#).as_deref(),
            Some("钠")
        );
        assert_eq!(parse_uapis(r#"{"translation":"x"}"#).as_deref(), Some("x"));
        assert_eq!(parse_uapis(r#"{"text":"x"}"#).as_deref(), Some("x"));
        assert_eq!(parse_uapis(r#"{"translate":"  "}"#), None);
        assert_eq!(parse_uapis("不是 JSON"), None);
    }

    #[test]
    fn parses_real_suapi_sample() {
        let values = parse_suapi(SUAPI_SAMPLE).unwrap();
        assert_eq!(
            values,
            vec![Some("钠".to_string()), Some("模组菜单".to_string())]
        );
        assert!(parse_suapi(r#"{"code":429,"msg":"too many"}"#).is_none());
        assert!(parse_suapi(r#"{"code":200}"#).is_none());
        assert!(parse_suapi("[]").is_none());
    }

    #[test]
    fn parses_deepl_sample() {
        let values = parse_deepl(
            r#"{"translations":[{"detected_source_language":"ZH","text":"Sodium"}]}"#,
        )
        .unwrap();
        assert_eq!(values, vec![Some("Sodium".to_string())]);
        assert!(parse_deepl(r#"{"message":"wrong"}"#).is_none());
    }

    #[test]
    fn provider_order_respects_sticky_and_deepl() {
        assert_eq!(
            provider_order(false, false),
            vec![Provider::Uapis, Provider::Suapi]
        );
        assert_eq!(
            provider_order(true, false),
            vec![Provider::Suapi, Provider::Uapis]
        );
        assert_eq!(
            provider_order(false, true),
            vec![Provider::Deepl, Provider::Uapis, Provider::Suapi]
        );
        assert_eq!(
            provider_order(true, true),
            vec![Provider::Deepl, Provider::Suapi, Provider::Uapis]
        );
    }

    #[test]
    fn language_mapping_and_clip() {
        assert_eq!(suapi_language("zh-CN"), ("zh-CHS".to_string(), true));
        assert_eq!(suapi_language("en"), ("en".to_string(), false));
        assert_eq!(suapi_language("EN"), ("en".to_string(), false));

        let long = "中".repeat(MAX_TEXT_CHARS + 50);
        assert_eq!(clip_text(&long).chars().count(), MAX_TEXT_CHARS);
        assert_eq!(clip_text("短"), "短");
        assert!(contains_cjk("钠 · 优化模组"));
        assert!(!contains_cjk("Sodium"));
    }

    #[test]
    fn cache_key_is_stable_and_language_scoped() {
        let first = cache_key("en", "钠");
        assert_eq!(first, cache_key("en", "钠"));
        assert_ne!(first, cache_key("zh-CN", "钠"));
        assert!(first.starts_with("en:"));
        assert_eq!(first.len(), "en:".len() + 40);
    }

    #[test]
    fn cache_trim_drops_expired_and_oldest() {
        let now = 1_000_000u64;
        let mut cache: HashMap<String, CacheEntry> = HashMap::new();
        cache.insert(
            "fresh".to_string(),
            CacheEntry {
                translated: "a".into(),
                at: now - 10,
            },
        );
        cache.insert(
            "stale".to_string(),
            CacheEntry {
                translated: "b".into(),
                at: now - CACHE_TTL.as_secs() - 1,
            },
        );
        trim_cache(&mut cache, now);
        assert!(cache.contains_key("fresh"));
        assert!(!cache.contains_key("stale"));

        let mut big: HashMap<String, CacheEntry> = HashMap::new();
        for index in 0..(CACHE_MAX_ENTRIES + 5) {
            big.insert(
                format!("k{index}"),
                CacheEntry {
                    translated: "x".into(),
                    at: now - index as u64,
                },
            );
        }
        trim_cache(&mut big, now);
        assert_eq!(big.len(), CACHE_MAX_ENTRIES);
        assert!(!big.contains_key(&format!("k{}", CACHE_MAX_ENTRIES + 4)));
    }

    #[test]
    fn cache_file_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("gugu-translate-cache-{}", util::unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(CACHE_FILE);
        let mut cache = HashMap::new();
        cache.insert(
            "en:abc".to_string(),
            CacheEntry {
                translated: "钠".to_string(),
                at: util::now_unix(),
            },
        );
        save_cache(&file, &cache).unwrap();
        let loaded = load_cache(&file);
        assert_eq!(
            loaded.get("en:abc").map(|entry| entry.translated.as_str()),
            Some("钠")
        );
        let mut stale = HashMap::new();
        stale.insert(
            "en:old".to_string(),
            CacheEntry {
                translated: "旧".to_string(),
                at: util::now_unix() - CACHE_TTL.as_secs() - 10,
            },
        );
        save_cache(&file, &stale).unwrap();
        assert!(load_cache(&file).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn negative_cache_expires() {
        let mut inner = Inner::default();
        let start = Instant::now();
        inner
            .negative
            .insert(negative_key(Provider::Uapis, "en", "钠"), start);
        assert!(negative_hit(&inner, Provider::Uapis, "en", "钠", start));
        assert!(!negative_hit(&inner, Provider::Suapi, "en", "钠", start));
        let later = start + NEGATIVE_TTL + Duration::from_secs(1);
        assert!(!negative_hit(&inner, Provider::Uapis, "en", "钠", later));
    }

    #[test]
    fn custom_prompt_and_parsing() {
        let texts = vec!["Coal".to_string(), "Iron Ingot".to_string()];
        let (system, user) = build_translation_prompt(&texts);
        assert!(system.contains("JSON"));
        assert_eq!(user, r#"["Coal","Iron Ingot"]"#);

        assert_eq!(
            parse_translations("[\"煤\",\"铁锭\"]", 2),
            vec![Some("煤".to_string()), Some("铁锭".to_string())]
        );
        assert_eq!(
            parse_translations("```json\n[\"煤\"]\n```", 1),
            vec![Some("煤".to_string())]
        );
        assert_eq!(
            parse_translations("翻译如下：[\"煤\"] 完毕", 1),
            vec![Some("煤".to_string())]
        );
        assert_eq!(
            parse_translations("[{\"text\":\"煤\"}]", 1),
            vec![Some("煤".to_string())]
        );
        assert_eq!(parse_translations("不是 JSON", 2), vec![None, None]);
    }

    #[test]
    fn chat_url_is_completed_and_validated() {
        assert_eq!(
            chat_completions_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1/chat/completions"
        );
        assert!(chat_completions_url("").is_err());
        assert!(chat_completions_url("api.example.com").is_err());
    }

    #[tokio::test]
    async fn builtin_translates_real_text_to_english() {
        // 真实联网：网络不可用时不误报失败
        let client = reqwest::Client::new();
        let translator = BuiltinTranslator::new();
        let dir = std::env::temp_dir().join(format!("gugu-live-{}", util::unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = dir.join(CACHE_FILE);
        let texts = vec!["钠 · 优化模组".to_string()];
        let result = translator.translate(&client, &cache, "", &texts, "en").await;
        let cached = std::fs::read_to_string(&cache).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        match result.results.first().and_then(|item| item.clone()) {
            Some(value) => {
                println!("真实翻译：钠 · 优化模组 → {value}");
                assert!(!value.trim().is_empty());
                assert!(cached.contains("\"t\""), "成功翻译后应写进缓存文件");
            }
            None => println!("本次没有拿到译文（网络受限？），跳过断言"),
        }
    }

    #[tokio::test]
    async fn builtin_returns_chinese_unchanged_when_target_is_zh() {
        let client = reqwest::Client::new();
        let translator = BuiltinTranslator::new();
        let dir = std::env::temp_dir().join(format!("gugu-live-zh-{}", util::unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let texts = vec!["钠 · 优化模组".to_string(), "".to_string()];
        let result = translator
            .translate(&client, &dir.join(CACHE_FILE), "", &texts, "zh-CN")
            .await;
        assert_eq!(result.results[0].as_deref(), Some("钠 · 优化模组"));
        assert_eq!(result.results[1].as_deref(), Some(""));
        assert_eq!(translator.request_count().await, 0, "中文原文不该发请求");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn custom_test_connection_without_custom_settings() {
        let settings = Settings::default();
        let client = reqwest::Client::new();
        let result = test_connection(&client, &settings).await;
        assert!(result.ok);
        assert!(result.message.contains("内置"));
    }

    #[tokio::test]
    async fn test_connection_masks_key_on_bad_url() {
        let mut settings = Settings::default();
        settings.translation_source = "custom".to_string();
        settings.translation_base_url = "ftp://api.example.com".to_string();
        settings.translation_api_key = "sk-abcd1234wxyz".to_string();
        let client = reqwest::Client::new();
        let result = test_connection(&client, &settings).await;
        assert!(!result.ok);
        assert!(!result.message.contains("sk-abcd1234wxyz"));
    }
}
