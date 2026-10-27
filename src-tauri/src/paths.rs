//! 数据根与子目录布局。
//!
//! 约定：数据根＝ `<安装目录>\.mc`；开发期（debug 构建）固定落在仓库根 `D:\minecraft\.mc`，
//! 方便和 `备份\`、`所有版本备份\` 这些参考目录并排检查，不会污染 exe 所在目录。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;

/// 数据根目录名（安装目录下的隐藏目录）。
pub const DATA_DIR_NAME: &str = ".mc";

/// 允许通过环境变量整体改数据根（测试/便携模式用）。
pub const ENV_DATA_ROOT: &str = "GUGU_DATA_ROOT";

/// 全部路径。序列化给前端时用 camelCase。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paths {
    pub root: PathBuf,
    pub assets: PathBuf,
    pub libraries: PathBuf,
    pub versions: PathBuf,
    pub instances: PathBuf,
    pub runtimes: PathBuf,
    pub skins: PathBuf,
    pub game_icons: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub diagnostics: PathBuf,
    pub settings_file: PathBuf,
    pub db_file: PathBuf,
    pub version_manifest_file: PathBuf,
}

impl Paths {
    /// 由「安装目录」推出完整布局：数据根 = 安装目录/.mc。
    pub fn from_install_dir(install_dir: impl AsRef<Path>) -> Self {
        let root = install_dir.as_ref().join(DATA_DIR_NAME);
        Self::from_root(root)
    }

    /// 由数据根本身推出完整布局（环境变量与测试走这个入口）。
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let sub = |name: &str| root.join(name);
        Self {
            assets: sub("assets"),
            libraries: sub("libraries"),
            versions: sub("versions"),
            instances: sub("instances"),
            runtimes: sub("runtimes"),
            skins: sub("skins"),
            game_icons: sub("game-icons"),
            cache: sub("cache"),
            logs: sub("logs"),
            diagnostics: sub("diagnostics"),
            settings_file: root.join("settings.json"),
            db_file: root.join("gugu.db"),
            version_manifest_file: root.join("version_manifest.json"),
            root,
        }
    }

    /// 把设置文件固定在原地（数据根可以搬家，但「指针」文件永远在安装目录的 .mc 下）。
    pub fn with_settings_file(mut self, settings_file: PathBuf) -> Self {
        self.settings_file = settings_file;
        self
    }

    /// 首次启动：把数据根与所有子目录建出来（幂等）。
    pub fn ensure(&self) -> Result<()> {
        for dir in self.all_dirs() {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn all_dirs(&self) -> [&Path; 11] {
        [
            &self.root,
            &self.assets,
            &self.libraries,
            &self.versions,
            &self.instances,
            &self.runtimes,
            &self.skins,
            &self.game_icons,
            &self.cache,
            &self.logs,
            &self.diagnostics,
        ]
    }
}

/// 安装目录下的默认数据根（<安装目录>\.mc）。
pub fn resolve() -> Paths {
    Paths::from_install_dir(install_dir())
}

fn install_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        // CARGO_MANIFEST_DIR = <repo>/src-tauri，上一级就是仓库根
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// 按设置里的 dataRoot 推出实际数据根。
///
/// 规则：环境变量 GUGU_DATA_ROOT > settings.dataRoot > 默认 <安装目录>\.mc。
/// 设置文件永远留在默认数据根里——它是「数据根指针」，迁移时不能跟着搬家，
/// 否则重启后无从得知新位置。
pub fn apply_data_root(default: Paths, data_root: &str) -> Paths {
    let custom = std::env::var(ENV_DATA_ROOT)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| data_root.trim().to_string());
    if custom.trim().is_empty() {
        return default;
    }
    let settings_file = default.settings_file.clone();
    Paths::from_root(PathBuf::from(custom.trim())).with_settings_file(settings_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_root_override_keeps_settings_file_in_place() {
        let default = Paths::from_install_dir("D:\\minecraft");
        let effective = apply_data_root(default.clone(), "D:\\GuguDataTest");
        assert_eq!(effective.root, PathBuf::from("D:\\GuguDataTest"));
        assert_eq!(effective.versions, PathBuf::from("D:\\GuguDataTest").join("versions"));
        assert_eq!(
            effective.settings_file, default.settings_file,
            "设置文件是指针，必须留在默认数据根"
        );

        // 空值 = 用默认
        let same = apply_data_root(default.clone(), "   ");
        assert_eq!(same.root, default.root);
        assert_eq!(same.settings_file, default.settings_file);
    }

    #[test]
    fn data_root_is_mc_under_install_dir() {
        let paths = Paths::from_install_dir("D:\\minecraft");
        assert!(paths.root.ends_with(DATA_DIR_NAME));
        assert_eq!(paths.root, PathBuf::from("D:\\minecraft").join(".mc"));
    }

    #[test]
    fn subdirs_follow_layout_spec() {
        let paths = Paths::from_root("C:\\tmp\\.mc");
        for (dir, name) in [
            (&paths.assets, "assets"),
            (&paths.libraries, "libraries"),
            (&paths.versions, "versions"),
            (&paths.instances, "instances"),
            (&paths.runtimes, "runtimes"),
            (&paths.skins, "skins"),
            (&paths.game_icons, "game-icons"),
            (&paths.cache, "cache"),
            (&paths.logs, "logs"),
            (&paths.diagnostics, "diagnostics"),
        ] {
            assert_eq!(dir.file_name().unwrap().to_str().unwrap(), name);
        }
        assert!(paths.settings_file.ends_with("settings.json"));
        assert!(paths.db_file.ends_with("gugu.db"));
        assert!(paths.version_manifest_file.ends_with("version_manifest.json"));
    }

    #[test]
    fn ensure_creates_every_directory() {
        let root = std::env::temp_dir().join(format!("gugu-paths-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().expect("ensure 应当成功");
        for dir in paths.all_dirs() {
            assert!(dir.is_dir(), "目录未创建：{}", dir.display());
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
