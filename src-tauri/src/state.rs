//! 全局状态：数据路径、内存里的设置、HTTP 客户端。
//!
//! 数据库不常驻：rusqlite::Connection 不是 Sync，命令层每次 \`state.db()\` 新开连接，
//! 省掉到处加锁，也避免异步命令里握着 MutexGuard 导致 future 不 Send。

use std::sync::Mutex;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::{AppError, Result};
use crate::net::github::ProbeCache;
use crate::paths::{self, Paths};
use crate::settings::{self, Settings};
use crate::tasks::TaskStore;
use crate::update::check::UpdateStatus;

pub struct AppState {
    pub paths: Paths,
    pub settings: Mutex<Settings>,
    pub client: reqwest::Client,
    /// 「关于」页显示的更新状态（后台任务与 update_check 命令共同维护）
    pub update: Mutex<UpdateStatus>,
    /// GitHub 加速节点探针结果（缓存 5 分钟，下载安装包前会强制重测）
    pub probes: Mutex<ProbeCache>,
    /// 下载中心的任务表：版本安装 / Java 下载 / 启动共用
    pub tasks: TaskStore,
    /// 内置翻译器（全局串行队列 + 缓存 + 主备切换，跨命令复用）
    pub translator: crate::translate::BuiltinTranslator,
    /// 陶瓦联机运行时（进程 + 状态）
    pub tera: crate::terracotta::TeraRuntime,
    /// 主页新闻的内存缓存 + 负缓存
    pub news: crate::news::NewsCache,
    /// 本地服务器进程表（多人页「服务器」子页）
    pub servers: crate::server::ServerStore,
}

impl AppState {
    /// 解析实际数据根：先读设置（设置文件永远在安装目录的 .mc 下），再据此定位数据根。
    ///
    /// 配置不可读/不可写、或自定义数据根建不出来时，回落默认数据根并记 warn
    /// （前端首次打开会看到提示）。
    pub fn bootstrap_paths() -> Paths {
        let default = paths::resolve();
        let loaded = match settings::load(&default.settings_file) {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(%err, "读取设置失败，回落默认数据根");
                eprintln!("读取设置失败（{err}），本次使用默认数据根 {}", default.root.display());
                return default;
            }
        };
        let effective = paths::apply_data_root(default.clone(), &loaded.data_root);
        if effective.root != default.root {
            // 自定义数据根第一次使用：直接把目录建出来
            if let Err(err) = effective.ensure() {
                tracing::warn!(
                    %err,
                    root = %effective.root.display(),
                    "自定义数据根不可用，回落默认数据根"
                );
                eprintln!(
                    "自定义数据根不可用（{}）：{err}，本次使用默认数据根 {}",
                    effective.root.display(),
                    default.root.display()
                );
                return default;
            }
        }
        effective
    }

    /// 启动引导：建目录 → 读设置 → 跑数据库迁移。数据根已经由 bootstrap_paths 定好。
    pub fn bootstrap(paths: Paths) -> Result<Self> {
        paths.ensure()?;
        let loaded = match settings::load(&paths.settings_file) {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(%err, "设置文件不可用，改用默认设置（前端会提示一次）");
                Settings::default()
            }
        };
        if loaded.close_to_tray_migrated {
            // 旧配置刚迁移：立刻落盘，避免下次再迁移一遍
            if let Err(err) = settings::save(&paths.settings_file, &loaded) {
                tracing::warn!(%err, "迁移后的设置写盘失败");
            }
        }
        // 首次启动就把库建好并迁移，后面命令直接 open 即可
        drop(crate::db::open(&paths.db_file)?);
        tracing::info!(root = %paths.root.display(), "数据根就绪");
        Ok(Self {
            paths,
            update: Mutex::new(UpdateStatus::new(
                env!("CARGO_PKG_VERSION"),
                crate::update::check::Phase::Idle,
                "尚未检查更新",
            )),
            probes: Mutex::new(ProbeCache::new()),
            tasks: TaskStore::new(),
            translator: crate::translate::BuiltinTranslator::new(),
            tera: crate::terracotta::TeraRuntime::new(),
            news: crate::news::NewsCache::new(),
            servers: crate::server::ServerStore::new(),
            settings: Mutex::new(loaded),
            client: build_client(),
        })
    }

    pub fn db(&self) -> Result<Connection> {
        crate::db::open(&self.paths.db_file)
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// 校验 → 落盘 → 更新内存；顺序固定，落盘失败时内存保持旧值。
    pub fn set_settings(&self, next: Settings) -> Result<Settings> {
        let next = next.normalize();
        next.validate()?;
        settings::save(&self.paths.settings_file, &next)?;
        let mut guard = self
            .settings
            .lock()
            .map_err(|_| AppError::Other("设置状态锁已损坏".to_string()))?;
        *guard = next.clone();
        Ok(next)
    }
}

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("GuguLauncher/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|err| {
            tracing::warn!(%err, "构建 HTTP 客户端失败，回落到默认配置");
            reqwest::Client::new()
        })
}
