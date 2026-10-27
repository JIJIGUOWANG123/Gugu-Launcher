//! 数据目录（settings.dataRoot）：统计、目标校验、复制迁移 + 校验 + 回滚。
//!
//! 迁移的语义：把当前数据根整棵树复制到新目录（设置文件指针留在原地），
//! 逐个文件比对文件数并抽样 sha256，全部通过才改 settings.dataRoot；
//! 任何一步失败都把已复制的内容删掉（回滚），原目录一个字节都不动。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::events::Phase;
use crate::paths::Paths;
use crate::tasks::TaskHandle;

/// 抽样校验的文件数上限。
pub const SAMPLE_LIMIT: usize = 8;

/// data_dir_get() 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDirInfo {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub is_default: bool,
}

fn walk_files(root: &Path, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk_files(&path, out),
            Ok(_) => {
                let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                out.push((path, size));
            }
            Err(_) => {}
        }
    }
}

/// 递归列出文件（路径 + 大小），顺序稳定。
pub fn list_files(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    walk_files(root, &mut out);
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

/// 目录体积与文件数（只读，不报错）。
pub fn dir_stats(root: &Path) -> (u64, u64) {
    let files = list_files(root);
    let size = files.iter().map(|(_, size)| *size).sum();
    (size, files.len() as u64)
}

/// data_dir_get：路径 + 体积 + 文件数 + 是否默认。
pub fn info(root: &Path, is_default: bool) -> DataDirInfo {
    let (size_bytes, file_count) = if root.is_dir() {
        dir_stats(root)
    } else {
        (0, 0)
    };
    DataDirInfo {
        path: root.display().to_string(),
        size_bytes,
        file_count,
        is_default,
    }
}

/// 词法规范化（不做 canonicalize：目标目录通常还不存在，而且 Windows 的
/// canonicalize 会加上 \\?\\ 前缀，反而破坏比较）。
fn normalize_path(path: &Path) -> String {
    let text = path.display().to_string().replace('/', "\\");
    let text = text.trim_start_matches(r"\\?\");
    let mut out = String::with_capacity(text.len());
    let mut previous_separator = false;
    for character in text.chars() {
        if character == '\\' {
            if previous_separator {
                continue;
            }
            previous_separator = true;
        } else {
            previous_separator = false;
        }
        out.extend(character.to_lowercase());
    }
    out.trim_end_matches('\\').to_string()
}

/// 是否是驱动根（D:\ 这种，迁移目标不能是它）。
pub fn is_drive_root(path: &Path) -> bool {
    let mut components = path.components();
    match components.next() {
        Some(std::path::Component::Prefix(_)) => {
            matches!(components.next(), Some(std::path::Component::RootDir))
                && components.next().is_none()
        }
        _ => path.components().count() <= 1,
    }
}

/// 迁移目标校验：不能是当前目录、不能非空、不能是驱动根、不能套在自己里面。
pub fn validate_target(current: &Path, target: &Path) -> Result<()> {
    if target.as_os_str().is_empty() {
        return Err(AppError::Invalid("目标目录不能为空".to_string()));
    }
    if !target.is_absolute() {
        return Err(AppError::Invalid(format!(
            "目标目录必须是绝对路径：{}",
            target.display()
        )));
    }
    if is_drive_root(target) {
        return Err(AppError::Invalid(format!(
            "不能把数据目录迁移到驱动根：{}",
            target.display()
        )));
    }
    let current_key = normalize_path(current);
    let target_key = normalize_path(target);
    if current_key == target_key {
        return Err(AppError::Invalid("目标目录就是当前数据目录，无需迁移".to_string()));
    }
    if target_key.starts_with(&format!("{current_key}\\")) {
        return Err(AppError::Invalid(
            "目标目录不能放在当前数据目录里面".to_string(),
        ));
    }
    if current_key.starts_with(&format!("{target_key}\\")) {
        return Err(AppError::Invalid(
            "当前数据目录在目标目录里面，迁移会把源目录删掉".to_string(),
        ));
    }
    if target.exists() {
        let mut entries = std::fs::read_dir(target).map_err(|err| {
            AppError::Other(format!("目标目录不可读（{}）：{err}", target.display()))
        })?;
        if entries.next().is_some() {
            return Err(AppError::Invalid(format!(
                "目标目录不是空的：请选一个空目录或新建一个（{}）",
                target.display()
            )));
        }
    }
    Ok(())
}

/// 抽样下标：首尾 + 均匀分布，最多 SAMPLE_LIMIT 个。
pub fn sample_indices(total: usize) -> Vec<usize> {
    if total == 0 {
        return Vec::new();
    }
    if total <= SAMPLE_LIMIT {
        return (0..total).collect();
    }
    let mut out: Vec<usize> = (0..SAMPLE_LIMIT)
        .map(|index| index * (total - 1) / (SAMPLE_LIMIT - 1))
        .collect();
    out.dedup();
    out
}

/// 复制整棵树（跳过设置文件指针），按文件数推 install://progress。
pub fn copy_tree(src: &Path, dst: &Path, skip: &Path, task: &TaskHandle) -> Result<u64> {
    let files = list_files(src);
    let skip_key = normalize_path(skip);
    let total = files.len() as u64;
    let mut done = 0u64;
    for (source, _) in &files {
        if normalize_path(source) == skip_key {
            continue;
        }
        let relative = source.strip_prefix(src).map_err(|err| {
            AppError::Other(format!("计算相对路径失败（{}）：{err}", source.display()))
        })?;
        let target = dst.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, &target)?;
        done += 1;
        task.progress(
            Phase::Downloading,
            &format!("正在复制 {} 个文件（{done}/{total}）", total),
            done,
            total,
        );
    }
    Ok(done)
}

/// 校验：文件数一致 + 抽样 sha256 一致。
pub fn verify(src: &Path, dst: &Path, skip: &Path) -> Result<()> {
    let skip_key = normalize_path(skip);
    let source_files: Vec<(PathBuf, u64)> = list_files(src)
        .into_iter()
        .filter(|(path, _)| normalize_path(path) != skip_key)
        .collect();
    let target_files = list_files(dst);
    if source_files.len() != target_files.len() {
        return Err(AppError::Other(format!(
            "迁移校验失败：文件数不一致（源 {} / 目标 {}）",
            source_files.len(),
            target_files.len()
        )));
    }
    for index in sample_indices(source_files.len()) {
        let (source, _) = &source_files[index];
        let relative = source
            .strip_prefix(src)
            .map_err(|err| AppError::Other(format!("计算相对路径失败：{err}")))?;
        let target = dst.join(relative);
        let left = crate::util::sha256_file(source)
            .map_err(|err| AppError::Other(format!("读源文件失败（{}）：{err}", source.display())))?;
        let right = crate::util::sha256_file(&target).map_err(|err| {
            AppError::Other(format!("读目标文件失败（{}）：{err}", target.display()))
        })?;
        if left != right {
            return Err(AppError::Other(format!(
                "迁移校验失败：{} 的 sha256 不一致",
                relative.display()
            )));
        }
    }
    Ok(())
}

/// 回滚：删掉本次迁移建出来的目标目录。
pub fn rollback(dst: &Path) {
    if dst.is_dir() {
        if let Err(err) = std::fs::remove_dir_all(dst) {
            tracing::warn!(%err, path = %dst.display(), "回滚删除目标目录失败，请手动清理");
        }
    }
}

/// 迁移本体：复制 → 校验 → 返回布尔成功；失败自行回滚。
pub fn migrate(current: &Paths, target: &Path, task: &TaskHandle) -> Result<()> {
    task.progress_forced(Phase::Downloading, "正在准备迁移", 0, 0);
    std::fs::create_dir_all(target)?;
    if let Err(err) = copy_tree(&current.root, target, &current.settings_file, task) {
        rollback(target);
        return Err(err);
    }
    task.progress_forced(Phase::Verifying, "正在校验迁移结果", 1, 1);
    if let Err(err) = verify(&current.root, target, &current.settings_file) {
        rollback(target);
        return Err(err);
    }
    task.progress_forced(Phase::Finished, "数据目录迁移完成", 1, 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("gugu-datadir-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    #[test]
    fn is_drive_root_only_for_root_paths() {
        assert!(is_drive_root(Path::new("D:\\")));
        assert!(is_drive_root(Path::new("D:/")));
        assert!(!is_drive_root(Path::new("D:\\GuguDataTest")));
        assert!(!is_drive_root(Path::new("D:\\minecraft\\.mc")));
    }

    #[test]
    fn validate_target_rejects_the_four_cases() {
        let current = Path::new("D:\\minecraft\\.mc");
        // 用一个肯定不存在的目标（不能让本机同名目录影响单测）
        assert!(validate_target(current, Path::new("D:\\GuguDataTest-unit-unused")).is_ok());
        // 目标是当前目录
        assert!(validate_target(current, Path::new("D:\\minecraft\\.mc")).is_err());
        assert!(validate_target(current, Path::new("D:\\minecraft\\.mc\\")).is_err());
        // 驱动根
        let err = validate_target(current, Path::new("D:\\")).unwrap_err().to_string();
        assert!(err.contains("驱动根"), "{err}");
        // 空路径 / 相对路径
        assert!(validate_target(current, Path::new("")).is_err());
        assert!(validate_target(current, Path::new("GuguData")).is_err());
        // 套在自己里面
        let err = validate_target(current, Path::new("D:\\minecraft\\.mc\\sub")).unwrap_err().to_string();
        assert!(err.contains("里面"), "{err}");
        // 源在目标里面
        assert!(validate_target(Path::new("D:\\a\\b"), Path::new("D:\\a")).is_err());
    }

    #[test]
    fn validate_target_rejects_non_empty_directory() {
        let root = std::env::temp_dir().join(format!("gugu-datadir-nonempty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // 空目录：允许
        assert!(validate_target(Path::new("D:\\minecraft\\.mc"), &root).is_ok());
        // 塞一个文件后拒绝
        std::fs::write(root.join("x.txt"), b"x").unwrap();
        let err = validate_target(Path::new("D:\\minecraft\\.mc"), &root)
            .unwrap_err()
            .to_string();
        assert!(err.contains("不是空的"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_verify_and_rollback_roundtrip() {
        let source = temp_paths("copy");
        std::fs::write(source.root.join("settings.json"), b"{}").unwrap();
        std::fs::write(source.versions.join("a.txt"), b"alpha").unwrap();
        std::fs::create_dir_all(source.instances.join("i1")).unwrap();
        std::fs::write(source.instances.join("i1").join("b.txt"), b"beta").unwrap();

        let target = std::env::temp_dir().join(format!("gugu-datadir-dst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&target);

        let store = TaskStore::new();
        let task = store.create_with_prefix("migrate", "迁移", "", "migrate:");

        std::fs::create_dir_all(&target).unwrap();
        let copied = copy_tree(&source.root, &target, &source.settings_file, &task).unwrap();
        assert_eq!(copied, 2, "设置文件指针不复制");
        assert!(target.join("versions").join("a.txt").is_file());
        assert!(!target.join("settings.json").exists());
        verify(&source.root, &target, &source.settings_file).unwrap();

        // 源目录一个都没少
        assert!(source.root.join("settings.json").is_file());
        assert_eq!(list_files(&source.root).len(), 3);
        assert_eq!(list_files(&target).len(), 2);

        rollback(&target);
        assert!(!target.exists());

        let _ = std::fs::remove_dir_all(&source.root);
    }

    #[test]
    fn verify_detects_truncated_copy() {
        let source = temp_paths("verify");
        std::fs::write(source.versions.join("a.txt"), b"alpha").unwrap();
        let target = std::env::temp_dir().join(format!("gugu-datadir-vdst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(target.join("versions")).unwrap();
        std::fs::write(target.join("versions").join("a.txt"), b"ALPHA").unwrap();
        let err = verify(&source.root, &target, &source.settings_file)
            .unwrap_err()
            .to_string();
        assert!(err.contains("sha256"), "{err}");
        let _ = std::fs::remove_dir_all(&source.root);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn sample_indices_cover_first_and_last() {
        assert_eq!(sample_indices(0), Vec::<usize>::new());
        assert_eq!(sample_indices(3), vec![0, 1, 2]);
        let big = sample_indices(1000);
        assert_eq!(big.first(), Some(&0));
        assert_eq!(big.last(), Some(&999));
        assert!(big.len() <= SAMPLE_LIMIT);
    }

    #[test]
    fn dir_stats_counts_nested_files() {
        let paths = temp_paths("stats");
        std::fs::write(paths.root.join("one.txt"), b"12345").unwrap();
        std::fs::create_dir_all(paths.logs.join("deep")).unwrap();
        std::fs::write(paths.logs.join("deep").join("two.txt"), b"123").unwrap();
        let (size, count) = dir_stats(&paths.root);
        assert_eq!(count, 2);
        assert!(size >= 8);
        let view = info(&paths.root, false);
        assert_eq!(view.file_count, 2);
        assert!(!view.is_default);
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
