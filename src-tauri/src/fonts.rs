//! 系统字体枚举：只挑出「真的能显示中文」的字体，供设置 → 常规 → 显示字体使用。
//!
//! 判定方式必须是查 cmap，不能靠字体名猜：用 ttf-parser 打开每个 face，
//! 检查常用汉字（中 / 国 / 文）三个码点都在 cmap 里才算中文字体。
//! 只查一个字符容易把「带少量 CJK 的西文字体」混进来。
//!
//! 枚举失败（fontdb 加载不到系统字体）时返回空列表，不抛错：界面回落 system-ui。

use serde::Serialize;

/// 默认字体：用户确认用「微软雅黑」。
pub const DEFAULT_FONT: &str = "Microsoft YaHei";

/// 默认值回落链（按顺序找第一个存在的）。
pub const DEFAULT_FONT_CHAIN: [&str; 5] = [
    "Microsoft YaHei",
    "微软雅黑",
    "Microsoft YaHei UI",
    "SimSun",
    "宋体",
];

/// 列表里一个候选字体都没有时的最后兜底（CSS 关键字，不是真实字体名）。
pub const FALLBACK_SYSTEM_UI: &str = "system-ui";

/// 优先排在前面的常见中文 UI 字体（不参与「中文字体」判定，只影响排序）。
pub const PREFERRED_FONTS: [&str; 10] = [
    "Microsoft YaHei UI",
    "Microsoft YaHei",
    "微软雅黑",
    "SimSun",
    "宋体",
    "Noto Sans SC",
    "Source Han Sans SC",
    "DengXian",
    "等线",
    "SimHei",
];

/// 明显的符号 / 图标字体黑名单（小写子串匹配）。这类字体常带几个汉字码点但根本不能读正文。
const SYMBOL_FONT_BLACKLIST: [&str; 11] = [
    "webdings",
    "wingdings",
    "symbol",
    "symbols",
    "segoe fluent icons",
    "material icons",
    "material symbols",
    "font awesome",
    "fa-solid",
    "fa-regular",
    "marlett",
];

/// 判定中文字体用的探针字符。
pub const CJK_PROBE_CHARS: [char; 3] = ['中', '国', '文'];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontChoice {
    /// 浏览器能解析的英文 family（Windows 注册名）
    pub family: String,
    /// 中文显示名（有 zh 名就用，否则等于 family）
    pub display_name: String,
    /// 其它可用名（英文名 + 中文名 + typographic），前端全写进 font-family
    pub aliases: Vec<String>,
    /// 字体来源（System / File 等），仅用于排查
    pub source: String,
    /// 是否系统内置兜底
    pub builtin: bool,
}

/// 内置中文名映射（name 表没给中文名时的兜底显示名；只覆盖 displayName，不动 family）。
pub const BUILTIN_CN_NAMES: [(&str, &str); 9] = [
    ("Microsoft YaHei", "微软雅黑"),
    ("SimSun", "宋体"),
    ("SimHei", "黑体"),
    ("KaiTi", "楷体"),
    ("FangSong", "仿宋"),
    ("DengXian", "等线"),
    ("YouYuan", "幼圆"),
    ("STXihei", "华文细黑"),
    ("NSimSun", "新宋体"),
];

/// 只有位图/宽度变体、切了也没效果的 family 后缀。
pub const BROKEN_FAMILY_SUFFIXES: [&str; 3] = ["-extb", "-exta", "-pua"];

pub fn lookup_builtin_cn_name(family: &str) -> Option<&'static str> {
    BUILTIN_CN_NAMES
        .iter()
        .find(|(english, _)| english.eq_ignore_ascii_case(family))
        .map(|(_, chinese)| *chinese)
}

/// 这一族字体是不是「列出来也没用」的（ExtB / 纯符号等）。
pub fn is_unusable_family(family: &str) -> bool {
    let lower = family.to_lowercase();
    BROKEN_FAMILY_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

/// 从 name 表里挑名字：优先指定 language_id，取不到再退回任意非空记录。
fn name_for(
    face: &ttf_parser::Face<'_>,
    name_ids: &[u16],
    language_ids: &[u16],
) -> Option<String> {
    for name_id in name_ids {
        for language_id in language_ids {
            let mut found: Option<String> = None;
            face.names().into_iter()
                .filter(|name| {
                    name.name_id == ttf_parser::name_id::FAMILY
                        || name.name_id == 16
                        || name.name_id == *name_id
                })
                .for_each(|name| {
                    if found.is_some() {
                        return;
                    }
                    if name.language_id == *language_id {
                        if let Some(text) = name.to_string() {
                            found = Some(text);
                        }
                    }
                });
            if let Some(text) = found {
                return Some(text);
            }
        }
    }
    None
}

/// 从字体文件里读 (英文 family, 中文 family, typographic family)。
pub fn read_font_names(parsed: &ttf_parser::Face<'_>) -> (Option<String>, Option<String>, Option<String>) {
    let english = name_for(parsed, &[ttf_parser::name_id::FAMILY, 16], &[0x0409]);
    let chinese = name_for(
        parsed,
        &[ttf_parser::name_id::FAMILY, 16],
        &[0x0804, 0x0404, 0x0C04, 0x1004],
    );
    let typographic = name_for(parsed, &[16], &[0x0409]);
    (english, chinese, typographic)
}

/// 纯函数：给一个「码点 → 有没有字形」的查询闭包，判断是否支持常用汉字。
pub fn is_cjk_capable(mut has_glyph: impl FnMut(char) -> bool) -> bool {
    CJK_PROBE_CHARS.iter().all(|ch| has_glyph(*ch))
}

pub fn is_blacklisted(family: &str) -> bool {
    let lower = family.to_lowercase();
    SYMBOL_FONT_BLACKLIST.iter().any(|bad| lower.contains(bad))
}

/// 去重（family 大小写不敏感，保留先出现的）+ 排序：
/// 常见中文 UI 字体按 PREFERRED_FONTS 顺序在前，其余按 family 字母序。
pub fn rank_fonts(fonts: Vec<FontChoice>) -> Vec<FontChoice> {
    let mut seen: Vec<String> = Vec::new();
    let mut unique: Vec<FontChoice> = Vec::new();
    for font in fonts {
        let key = font.family.to_lowercase();
        if key.trim().is_empty() || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        unique.push(font);
    }

    let preferred_index = |family: &str| -> Option<usize> {
        let lower = family.to_lowercase();
        PREFERRED_FONTS
            .iter()
            .position(|name| name.to_lowercase() == lower)
    };

    unique.sort_by(|a, b| {
        match (preferred_index(&a.family), preferred_index(&b.family)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.family.to_lowercase().cmp(&b.family.to_lowercase()),
        }
    });
    unique
}

/// 默认字体：按默认回落链找，都没有就用列表第一个，列表为空回落 system-ui。
pub fn resolve_default_font(fonts: &[FontChoice]) -> String {
    for wanted in DEFAULT_FONT_CHAIN {
        if let Some(font) = fonts.iter().find(|font| font.family.eq_ignore_ascii_case(wanted)) {
            return font.family.clone();
        }
    }
    fonts
        .first()
        .map(|font| font.family.clone())
        .unwrap_or_else(|| FALLBACK_SYSTEM_UI.to_string())
}

/// 选择的字体是否还在列表里；不在就回落到默认值。
/// 返回 (最终字体, 是否发生了回落)。
pub fn resolve_ui_font(chosen: &str, fonts: &[FontChoice]) -> (String, bool) {
    if chosen.trim().is_empty() {
        return (resolve_default_font(fonts), true);
    }
    if chosen == FALLBACK_SYSTEM_UI || fonts.iter().any(|font| font.family.eq_ignore_ascii_case(chosen)) {
        return (chosen.to_string(), false);
    }
    (resolve_default_font(fonts), true)
}

/// 枚举系统字体并筛出中文字体。失败时返回空列表（调用方显示「无法读取系统字体」）。
pub fn list_system_fonts() -> Vec<FontChoice> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut out = Vec::new();
    for face in db.faces() {
        let Some((family, _language)) = face.families.first() else {
            continue;
        };
        if is_blacklisted(family) {
            continue;
        }
        if is_unusable_family(family) {
            continue;
        }
        let info = db.with_face_data(face.id, |data, face_index| {
            let parsed = ttf_parser::Face::parse(data, face_index).ok()?;
            if !is_cjk_capable(|ch| parsed.glyph_index(ch).is_some()) {
                return None;
            }
            Some(read_font_names(&parsed))
        });
        let Some(info) = info.flatten() else {
            continue;
        };
        let (english, chinese, typographic) = info;
        // family 必须是 Windows 注册的英文名（浏览器按这个名字解析成功），取不到才退回 fontdb
        let family_name = english
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| family.clone());
        if is_unusable_family(&family_name) || is_blacklisted(&family_name) {
            continue;
        }
        // 显示名：name 表的中文名 → 内置中文名映射 → family
        let display_name = chinese
            .clone()
            .filter(|name| !name.trim().is_empty())
            .or_else(|| lookup_builtin_cn_name(&family_name).map(|name| name.to_string()))
            .unwrap_or_else(|| family_name.clone());
        let mut aliases: Vec<String> = Vec::new();
        for candidate in [
            Some(family_name.clone()),
            english,
            chinese,
            typographic,
            Some(family.clone()),
        ]
        .into_iter()
        .flatten()
        {
            let candidate = candidate.trim().to_string();
            if candidate.is_empty()
                || aliases
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&candidate))
            {
                continue;
            }
            aliases.push(candidate);
        }
        out.push(FontChoice {
            family: family_name,
            display_name,
            aliases,
            builtin: family.eq_ignore_ascii_case(DEFAULT_FONT),
            source: format!("{:?}", face.source),
        });
    }
    rank_fonts(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(family: &str) -> FontChoice {
        FontChoice {
            family: family.to_string(),
            display_name: lookup_builtin_cn_name(family)
                .map(|name| name.to_string())
                .unwrap_or_else(|| family.to_string()),
            aliases: vec![family.to_string()],
            source: "System".to_string(),
            builtin: false,
        }
    }

    fn families(fonts: &[FontChoice]) -> Vec<&str> {
        fonts.iter().map(|font| font.family.as_str()).collect()
    }

    #[test]
    fn builtin_chinese_names_cover_common_ui_fonts() {
        assert_eq!(lookup_builtin_cn_name("Microsoft YaHei"), Some("微软雅黑"));
        assert_eq!(lookup_builtin_cn_name("microsoft yahei"), Some("微软雅黑"));
        assert_eq!(lookup_builtin_cn_name("SimSun"), Some("宋体"));
        assert_eq!(lookup_builtin_cn_name("KaiTi"), Some("楷体"));
        assert_eq!(lookup_builtin_cn_name("FangSong"), Some("仿宋"));
        assert_eq!(lookup_builtin_cn_name("不存在字体"), None);
    }

    #[test]
    fn unusable_families_are_rejected() {
        assert!(is_unusable_family("SimSun-ExtB"));
        assert!(is_unusable_family("NSimSun-ExtB"));
        assert!(!is_unusable_family("SimSun"));
        assert!(!is_unusable_family("Microsoft YaHei"));
    }

    #[test]
    fn display_name_and_aliases_roundtrip() {
        let choice = font("Microsoft YaHei");
        assert_eq!(choice.display_name, "微软雅黑");
        assert_eq!(choice.aliases, vec!["Microsoft YaHei".to_string()]);
        let json = serde_json::to_value(&choice).unwrap();
        assert_eq!(json["displayName"], "微软雅黑");
        assert!(json.get("aliases").is_some());
        // 没有中文名的字体，displayName 等于 family
        let plain = font("Consolas");
        assert_eq!(plain.display_name, "Consolas");
    }

    #[test]
    fn cjk_probe_requires_all_three_characters() {
        // 只有「中」的西文字体不算
        assert!(!is_cjk_capable(|ch| ch == '中'));
        assert!(!is_cjk_capable(|ch| ch == '中' || ch == '国'));
        assert!(is_cjk_capable(|ch| matches!(ch, '中' | '国' | '文')));
        assert!(!is_cjk_capable(|_| false));
    }

    #[test]
    fn symbol_fonts_are_blacklisted() {
        assert!(is_blacklisted("Segoe UI Symbol"));
        assert!(is_blacklisted("Segoe Fluent Icons"));
        assert!(is_blacklisted("Material Icons"));
        assert!(is_blacklisted("Webdings"));
        assert!(!is_blacklisted("Microsoft YaHei"));
        assert!(!is_blacklisted("SimSun"));
    }

    #[test]
    fn rank_puts_common_ui_fonts_first_and_dedupes() {
        let ranked = rank_fonts(vec![
            font("ZongYi"),
            font("Microsoft YaHei"),
            font("microsoft yahei"),
            font("DengXian"),
            font("SimSun"),
            font("KaiTi"),
        ]);
        let names = families(&ranked);
        assert_eq!(names[0], "Microsoft YaHei");
        assert_eq!(names[1], "SimSun");
        assert_eq!(names[2], "DengXian");
        // 其余按字母序
        assert_eq!(names[3], "KaiTi");
        assert_eq!(names[4], "ZongYi");
        assert_eq!(ranked.len(), 5, "family 去重（大小写不敏感）");
    }

    #[test]
    fn default_font_follows_the_chain() {
        // 有雅黑就选雅黑
        let with_yahei = vec![font("SimSun"), font("Microsoft YaHei"), font("Microsoft YaHei UI")];
        assert_eq!(resolve_default_font(&with_yahei), "Microsoft YaHei");

        // 只有中文别名
        let alias_only = vec![font("微软雅黑")];
        assert_eq!(resolve_default_font(&alias_only), "微软雅黑");

        // 只有 YaHei UI
        let ui_only = vec![font("Microsoft YaHei UI")];
        assert_eq!(resolve_default_font(&ui_only), "Microsoft YaHei UI");

        // 都没有 → 第一个可用
        let others = vec![font("ZongYi"), font("KaiTi")];
        assert_eq!(resolve_default_font(&others), "ZongYi");

        // 空列表
        assert_eq!(resolve_default_font(&[]), FALLBACK_SYSTEM_UI);
    }

    /// 真机验证：本机至少要枚举出 3 个中文字体，且包含「微软雅黑」。
    /// 这条依赖运行环境，只在 Windows 上跑；换机器时按实际字体情况调整。
    #[cfg(windows)]
    #[test]
    fn system_fonts_include_chinese_fonts_on_this_machine() {
        let fonts = list_system_fonts();
        println!("找到 {} 个中文字体候选：", fonts.len());
        for font in fonts.iter().take(12) {
            println!("  - {} ({})", font.family, font.source);
        }
        assert!(fonts.len() >= 3, "本机中文字体候选应不少于 3 个，实际 {}", fonts.len());
        assert!(
            fonts
                .iter()
                .any(|font| font.family.eq_ignore_ascii_case("Microsoft YaHei") || font.family == "微软雅黑"),
            "本机应包含 Microsoft YaHei / 微软雅黑"
        );
        assert!(
            !fonts.iter().any(|font| is_blacklisted(&font.family)),
            "符号字体不应出现在候选里"
        );
    }

    #[test]
    fn missing_selected_font_falls_back_with_flag() {
        let fonts = vec![font("Microsoft YaHei"), font("SimSun")];
        assert_eq!(
            resolve_ui_font("SimSun", &fonts),
            ("SimSun".to_string(), false)
        );
        assert_eq!(
            resolve_ui_font("Comic Sans MS", &fonts),
            ("Microsoft YaHei".to_string(), true)
        );
        assert_eq!(
            resolve_ui_font("", &fonts),
            ("Microsoft YaHei".to_string(), true)
        );
        // 列表为空时回落 system-ui 而不是空串
        assert_eq!(resolve_ui_font("SimSun", &[]), (FALLBACK_SYSTEM_UI.to_string(), true));
    }
}
