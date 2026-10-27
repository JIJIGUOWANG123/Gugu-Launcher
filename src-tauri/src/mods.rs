//! 实例内页的模组 / 资源包 / 光影 / 存档 / 导出。
//!
//! 目录规则（相对实例的游戏目录 .minecraft）：
//! mod -> mods、resourcepack -> resourcepacks、shader -> shaderpacks、存档 -> saves。
//! 停用是把 xxx.jar 改名成 xxx.jar.disabled（重名冲突要报错）。

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::instance;
use crate::paths::Paths;
use crate::util;

/// 实例导出默认跳过的目录（缓存/大文件/日志）。
pub const EXPORT_EXCLUDES: [&str; 6] = [
    "versions",
    "libraries",
    "assets",
    "logs",
    "crash-reports",
    "natives",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKind {
    Mod,
    ResourcePack,
    Shader,
}

impl ModKind {
    pub fn from_id(id: &str) -> Result<Self> {
        match id {
            "mod" => Ok(ModKind::Mod),
            "resourcepack" => Ok(ModKind::ResourcePack),
            "shader" => Ok(ModKind::Shader),
            other => Err(AppError::Invalid(format!("不支持的模组类型：{other}"))),
        }
    }

    pub fn dir_name(&self) -> &'static str {
        match self {
            ModKind::Mod => "mods",
            ModKind::ResourcePack => "resourcepacks",
            ModKind::Shader => "shaderpacks",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFileItem {
    pub file: String,
    pub size: u64,
    pub enabled: bool,
    pub mtime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loader_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveItem {
    pub name: String,
    pub size: u64,
    pub mtime: String,
    pub version: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: usize,
}

/// 停用后缀。
pub const DISABLED_SUFFIX: &str = ".disabled";

pub fn kind_dir(paths: &Paths, instance_name: &str, kind: ModKind) -> PathBuf {
    instance::game_dir(paths, instance_name).join(kind.dir_name())
}

pub fn saves_dir(paths: &Paths, instance_name: &str) -> PathBuf {
    instance::game_dir(paths, instance_name).join("saves")
}

pub fn exports_dir(paths: &Paths) -> PathBuf {
    paths.root.join("exports")
}

/// 判断文件名是否处于停用态。
pub fn is_disabled(file: &str) -> bool {
    file.to_lowercase().ends_with(DISABLED_SUFFIX)
}

/// 只接受纯文件名（挡掉路径穿越）。
pub fn validate_file_name(file: &str) -> Result<()> {
    let trimmed = file.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("文件名不能为空".to_string()));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(AppError::Invalid(format!("文件名不合法：{file}")));
    }
    Ok(())
}

fn modified_stamp(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| util::iso8601(duration.as_secs()))
        .unwrap_or_default()
}

pub fn list_mods(
    paths: &Paths,
    instance_name: &str,
    kind: ModKind,
    loader_hint: Option<String>,
) -> Result<Vec<ModFileItem>> {
    let dir = kind_dir(paths, instance_name, kind);
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(err) => return Err(AppError::Io(err)),
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let lowered = file.to_lowercase();
        if !(lowered.ends_with(".jar")
            || lowered.ends_with(".zip")
            || lowered.ends_with(&format!(".jar{DISABLED_SUFFIX}")))
        {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        out.push(ModFileItem {
            enabled: !is_disabled(&file),
            size: metadata.len(),
            mtime: modified_stamp(&metadata),
            loader_hint: loader_hint.clone(),
            file,
        });
    }
    out.sort_by(|left, right| left.file.cmp(&right.file));
    Ok(out)
}

/// 启用/停用：改扩展名，目标已存在时报错（不覆盖玩家文件）。
pub fn set_enabled(
    paths: &Paths,
    instance_name: &str,
    kind: ModKind,
    file: &str,
    enabled: bool,
) -> Result<String> {
    validate_file_name(file)?;
    let dir = kind_dir(paths, instance_name, kind);
    let current = dir.join(file);
    if !current.is_file() {
        return Err(AppError::Invalid(format!("文件不存在：{file}")));
    }
    let target_name = if enabled {
        if !is_disabled(file) {
            return Ok(file.to_string());
        }
        file.trim_end_matches(DISABLED_SUFFIX).to_string()
    } else {
        if is_disabled(file) {
            return Ok(file.to_string());
        }
        format!("{file}{DISABLED_SUFFIX}")
    };
    let target = dir.join(&target_name);
    if target.exists() {
        return Err(AppError::Invalid(format!(
            "目标文件已存在，无法改名：{target_name}"
        )));
    }
    std::fs::rename(&current, &target)?;
    Ok(target_name)
}

/// 删除：主文件与 .disabled 变体都删掉。
pub fn delete_file(paths: &Paths, instance_name: &str, kind: ModKind, file: &str) -> Result<()> {
    validate_file_name(file)?;
    let dir = kind_dir(paths, instance_name, kind);
    let base = file.trim_end_matches(DISABLED_SUFFIX).to_string();
    let candidates = [dir.join(&base), dir.join(format!("{base}{DISABLED_SUFFIX}"))];
    let mut removed = false;
    for candidate in candidates {
        if candidate.is_file() {
            std::fs::remove_file(&candidate)?;
            removed = true;
        }
    }
    if !removed {
        return Err(AppError::Invalid(format!("文件不存在：{file}")));
    }
    Ok(())
}

/// 导入：把外部文件复制进目录，同名跳过。
pub fn import(
    paths: &Paths,
    instance_name: &str,
    kind: ModKind,
    sources: &[String],
) -> Result<ImportReport> {
    let dir = kind_dir(paths, instance_name, kind);
    std::fs::create_dir_all(&dir)?;
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for source in sources {
        let source_path = Path::new(source.trim());
        if !source_path.is_file() {
            skipped += 1;
            continue;
        }
        let Some(name) = source_path.file_name() else {
            skipped += 1;
            continue;
        };
        let target = dir.join(name);
        if target.exists() {
            skipped += 1;
            continue;
        }
        std::fs::copy(source_path, &target)?;
        imported += 1;
    }
    Ok(ImportReport { imported, skipped })
}

/// 目录大小（递归）。
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() {
            total += dir_size(&path);
        } else if let Ok(metadata) = entry.metadata() {
            total += metadata.len();
        }
    }
    total
}

pub fn list_saves(paths: &Paths, instance_name: &str) -> Result<Vec<SaveItem>> {
    let dir = saves_dir(paths, instance_name);
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(err) => return Err(AppError::Io(err)),
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let icon = path.join("icon.png");
        let version = std::fs::read_to_string(path.join("levelname.txt")).ok();
        out.push(SaveItem {
            name,
            size: dir_size(&path),
            mtime: modified_stamp(&metadata),
            version,
            icon: if icon.is_file() {
                Some(icon.display().to_string())
            } else {
                None
            },
        });
    }
    out.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(out)
}

pub fn validate_save_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("存档名不能为空".to_string()));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(AppError::Invalid(format!("存档名不合法：{name}")));
    }
    Ok(())
}

pub fn delete_save(paths: &Paths, instance_name: &str, name: &str) -> Result<()> {
    validate_save_name(name)?;
    let dir = saves_dir(paths, instance_name).join(name);
    if !dir.is_dir() {
        return Err(AppError::Invalid(format!("存档不存在：{name}")));
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// 实例导出/存档导出是否要跳过这个相对路径。
pub fn should_exclude(relative: &str) -> bool {
    let lowered = relative.to_lowercase();
    EXPORT_EXCLUDES
        .iter()
        .any(|name| lowered == *name || lowered.starts_with(&format!("{name}/")))
}

fn add_dir_to_zip<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    root: &Path,
    current: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<usize> {
    let mut count = 0usize;
    let Ok(entries) = std::fs::read_dir(current) else {
        return Ok(0);
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map(|item| item.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if relative.is_empty() || should_exclude(&relative) {
            continue;
        }
        if path.is_dir() {
            count += add_dir_to_zip(zip, root, &path, options)?;
        } else {
            zip.start_file(relative, options)
                .map_err(|err| AppError::Other(format!("写入压缩包失败：{err}")))?;
            let bytes = std::fs::read(&path)?;
            zip.write_all(&bytes)?;
            count += 1;
        }
    }
    Ok(count)
}

/// 存档导出：zip 到 .mc/exports/<存档名>.zip。
pub fn export_save(paths: &Paths, instance_name: &str, name: &str) -> Result<PathBuf> {
    validate_save_name(name)?;
    let source = saves_dir(paths, instance_name).join(name);
    if !source.is_dir() {
        return Err(AppError::Invalid(format!("存档不存在：{name}")));
    }
    let exports = exports_dir(paths);
    std::fs::create_dir_all(&exports)?;
    let target = exports.join(format!("{}.zip", util::sanitize_file_part(name)));
    let file = std::fs::File::create(&target)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    // 存档内部不套 excludes 规则（玩家要完整备份），用空实现绕开
    add_save_dir(&mut zip, &source, &source, options)?;
    zip.finish()
        .map_err(|err| AppError::Other(format!("关闭压缩包失败：{err}")))?;
    Ok(target)
}

fn add_save_dir<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    root: &Path,
    current: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<usize> {
    let mut count = 0usize;
    let Ok(entries) = std::fs::read_dir(current) else {
        return Ok(0);
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() {
            count += add_save_dir(zip, root, &path, options)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map(|item| item.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            zip.start_file(relative, options)
                .map_err(|err| AppError::Other(format!("写入压缩包失败：{err}")))?;
            let bytes = std::fs::read(&path)?;
            zip.write_all(&bytes)?;
            count += 1;
        }
    }
    Ok(count)
}

/// 整实例导出：只打游戏目录，跳过缓存目录与日志。
pub fn export_instance(paths: &Paths, instance_name: &str) -> Result<PathBuf> {
    let source = instance::game_dir(paths, instance_name);
    if !source.is_dir() {
        return Err(AppError::Invalid(format!(
            "实例目录不存在：{}",
            source.display()
        )));
    }
    let exports = exports_dir(paths);
    std::fs::create_dir_all(&exports)?;
    let stamp = util::compact_stamp(util::now_unix());
    let target = exports.join(format!(
        "{}-{stamp}.zip",
        util::sanitize_file_part(instance_name)
    ));
    let file = std::fs::File::create(&target)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    add_dir_to_zip(&mut zip, &source, &source, options)?;
    // 单独补上实例根目录下的 icon.png / options.txt 一类文件
    let instance_root = instance::instance_dir(paths, instance_name);
    if let Ok(entries) = std::fs::read_dir(&instance_root) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            zip.start_file(format!("__instance__/{name}"), options)
                .map_err(|err| AppError::Other(format!("写入压缩包失败：{err}")))?;
            let bytes = std::fs::read(&path)?;
            zip.write_all(&bytes)?;
        }
    }
    zip.finish()
        .map_err(|err| AppError::Other(format!("关闭压缩包失败：{err}")))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(tag: &str) -> (Paths, String) {
        let root = std::env::temp_dir().join(format!(
            "gugu-mods-{tag}-{}-{}",
            std::process::id(),
            util::unique_suffix()
        ));
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        let name = "实例A".to_string();
        std::fs::create_dir_all(instance::game_dir(&paths, &name)).unwrap();
        (paths, name)
    }

    #[test]
    fn kind_dirs_follow_instance_layout() {
        let (paths, name) = setup("dirs");
        assert_eq!(kind_dir(&paths, &name, ModKind::Mod).file_name().unwrap(), "mods");
        assert_eq!(
            kind_dir(&paths, &name, ModKind::ResourcePack).file_name().unwrap(),
            "resourcepacks"
        );
        assert_eq!(kind_dir(&paths, &name, ModKind::Shader).file_name().unwrap(), "shaderpacks");
        assert!(kind_dir(&paths, &name, ModKind::Mod).ends_with(".minecraft/mods"));
        assert_eq!(ModKind::from_id("mod").unwrap(), ModKind::Mod);
        assert!(ModKind::from_id("shaderpack").is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn enable_disable_renames_and_detects_conflicts() {
        let (paths, name) = setup("toggle");
        let dir = kind_dir(&paths, &name, ModKind::Mod);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jar"), b"jar").unwrap();

        let disabled = set_enabled(&paths, &name, ModKind::Mod, "a.jar", false).unwrap();
        assert_eq!(disabled, "a.jar.disabled");
        assert!(dir.join("a.jar.disabled").is_file());
        assert!(!dir.join("a.jar").exists());

        let items = list_mods(&paths, &name, ModKind::Mod, Some("Fabric".to_string())).unwrap();
        assert_eq!(items.len(), 1);
        assert!(!items[0].enabled);
        assert_eq!(items[0].loader_hint.as_deref(), Some("Fabric"));

        let enabled = set_enabled(&paths, &name, ModKind::Mod, "a.jar.disabled", true).unwrap();
        assert_eq!(enabled, "a.jar");
        assert!(dir.join("a.jar").is_file());

        // 冲突：a.jar 与 a.jar.disabled 同时存在时改名要报错
        std::fs::write(dir.join("a.jar.disabled"), b"other").unwrap();
        let err = set_enabled(&paths, &name, ModKind::Mod, "a.jar", false).unwrap_err();
        assert!(err.to_string().contains("目标文件已存在"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn delete_removes_both_variants() {
        let (paths, name) = setup("delete");
        let dir = kind_dir(&paths, &name, ModKind::Mod);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.jar.disabled"), b"jar").unwrap();
        delete_file(&paths, &name, ModKind::Mod, "b.jar").unwrap();
        assert!(!dir.join("b.jar.disabled").exists());
        assert!(delete_file(&paths, &name, ModKind::Mod, "b.jar").is_err());
        assert!(delete_file(&paths, &name, ModKind::Mod, "../evil.jar").is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn import_copies_and_skips_duplicates() {
        let (paths, name) = setup("import");
        let source_dir = paths.root.join("incoming");
        std::fs::create_dir_all(&source_dir).unwrap();
        let first = source_dir.join("mod1.jar");
        std::fs::write(&first, b"jar1").unwrap();
        let second = source_dir.join("mod2.jar");
        std::fs::write(&second, b"jar2").unwrap();

        let report = import(
            &paths,
            &name,
            ModKind::Mod,
            &[first.display().to_string(), second.display().to_string()],
        )
        .unwrap();
        assert_eq!(report.imported, 2);
        assert_eq!(report.skipped, 0);

        let again = import(&paths, &name, ModKind::Mod, &[first.display().to_string()]).unwrap();
        assert_eq!(again.imported, 0);
        assert_eq!(again.skipped, 1);
        let missing = import(&paths, &name, ModKind::Mod, &["C:\\nope\\x.jar".to_string()]).unwrap();
        assert_eq!(missing.skipped, 1);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn saves_list_delete_and_export() {
        let (paths, name) = setup("saves");
        let saves = saves_dir(&paths, &name);
        let world = saves.join("新的世界");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("level.dat"), b"data").unwrap();
        std::fs::write(world.join("region").join("r.0.0.mca"), b"chunk").unwrap();

        let items = list_saves(&paths, &name).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "新的世界");
        assert!(items[0].size >= 9);

        let zipped = export_save(&paths, &name, "新的世界").unwrap();
        assert!(zipped.is_file());
        assert!(zipped.starts_with(exports_dir(&paths)));
        assert!(zipped.file_name().unwrap().to_string_lossy().ends_with(".zip"));

        delete_save(&paths, &name, "新的世界").unwrap();
        assert!(list_saves(&paths, &name).unwrap().is_empty());
        assert!(delete_save(&paths, &name, "不存在").is_err());
        assert!(export_save(&paths, &name, "../x").is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn export_excludes_caches_and_logs() {
        assert!(should_exclude("logs"));
        assert!(should_exclude("logs/latest.log"));
        assert!(should_exclude("versions/1.20.1/1.20.1.jar"));
        assert!(should_exclude("natives/lwjgl.dll"));
        assert!(!should_exclude("mods/a.jar"));
        assert!(!should_exclude("saves/world/level.dat"));
        assert!(!should_exclude("options.txt"));
    }

    #[test]
    fn instance_export_writes_zip_without_caches() {
        let (paths, name) = setup("export");
        let game = instance::game_dir(&paths, &name);
        std::fs::create_dir_all(game.join("mods")).unwrap();
        std::fs::create_dir_all(game.join("logs")).unwrap();
        std::fs::write(game.join("mods").join("a.jar"), b"jar").unwrap();
        std::fs::write(game.join("logs").join("latest.log"), b"log").unwrap();
        std::fs::write(game.join("options.txt"), b"fov:70").unwrap();

        let zipped = export_instance(&paths, &name).unwrap();
        assert!(zipped.is_file());
        let file = std::fs::File::open(&zipped).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect();
        assert!(names.iter().any(|name| name.ends_with("mods/a.jar")), "{names:?}");
        assert!(names.iter().any(|name| name.ends_with("options.txt")), "{names:?}");
        assert!(!names.iter().any(|name| name.starts_with("logs/")), "{names:?}");
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
