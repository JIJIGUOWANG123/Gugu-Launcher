//! 暴露给前端的 Tauri command。前端只通过 src/lib/tauri-api.ts 调用这些函数，
//! 名字与参数按 M2-B 契约（camelCase JS 对象）保持一致。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::content;
use crate::download::{self, DownloadSource};
use crate::error::{AppError, Result};
use crate::events::{Events, TauriEvents};
use crate::fonts::{self, FontChoice};
use crate::install::{self, Installer};
use crate::instance::{self, Instance, InstanceCreate, InstanceSettingsPatch, InstanceSettingsView};
use crate::java::{self, JavaRuntime, JavaSourceInfo, JavaSourceProbe, JavaVersionInfo};
use crate::launch::{self, LaunchOptions};
use crate::meta::{self, VersionManifestView};
use crate::modpack;
use crate::ms_auth::{self, MsAuthStatus};
use crate::paths::Paths;
use crate::server::{self, ServerCore, ServerState, ServerVersion, ServerView};
use crate::server_content;
use crate::settings::Settings;
use crate::shell::{self, LogFile};
use crate::sources::{self, SourceInfo, SourceProbe};
use crate::state::AppState;
use crate::tasks::DownloadTaskView;
use crate::update::check::{self, UpdateStatus};
use crate::util;

/// 与 tauri.conf.json 的 identifier 保持一致。
pub const IDENTIFIER: &str = "com.gugu.launcher";
pub const PRODUCT_NAME: &str = "咕咕启动器";
pub const PRODUCT_NAME_EN: &str = "Gugu Launcher";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub name_en: &'static str,
    pub version: &'static str,
    pub identifier: &'static str,
    pub tauri_version: &'static str,
    pub data_root: String,
}

fn events_of(app: &AppHandle) -> Arc<dyn Events> {
    Arc::new(TauriEvents::new(app.clone()))
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: PRODUCT_NAME,
        name_en: PRODUCT_NAME_EN,
        version: env!("CARGO_PKG_VERSION"),
        identifier: IDENTIFIER,
        tauri_version: tauri::VERSION,
        data_root: state.paths.root.display().to_string(),
    }
}

/// 首次启动创建 .mc 与全部子目录，返回布局给前端显示。
#[tauri::command]
pub fn paths_init(state: State<'_, AppState>) -> Result<Paths> {
    state.paths.ensure()?;
    Ok(state.paths.clone())
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> Result<Settings> {
    match crate::settings::load(&state.paths.settings_file) {
        Ok(settings) => Ok(settings),
        Err(err) => {
            tracing::warn!(%err, "读取设置失败，改用内存中的设置");
            Ok(state.settings())
        }
    }
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
    state.set_settings(settings)
}

/// 版本清单：默认读缓存，forceRefresh 时联网重拉。
#[tauri::command]
pub async fn versions_list(
    state: State<'_, AppState>,
    force_refresh: Option<bool>,
) -> Result<VersionManifestView> {
    let settings = state.settings();
    let origins = sources::resolve_with_probe(&state.client, &settings).await;
    meta::load_with_origins(
        &state.client,
        &state.paths,
        origins,
        force_refresh.unwrap_or(false),
    )
    .await
}

// ------------------------------------------------------------------ 实例

#[tauri::command]
pub fn instances_list(state: State<'_, AppState>) -> Result<Vec<Instance>> {
    let conn = state.db()?;
    instance::list(&conn)
}

#[tauri::command]
pub fn instance_create(state: State<'_, AppState>, payload: InstanceCreate) -> Result<Instance> {
    let conn = state.db()?;
    instance::create(&conn, &state.paths, &payload)
}

#[tauri::command]
pub fn instance_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    let conn = state.db()?;
    instance::delete(&conn, &state.paths, id)
}

/// 实例全量信息（含设置字段）。
#[tauri::command]
pub fn instance_detail(state: State<'_, AppState>, id: i64) -> Result<Instance> {
    let conn = state.db()?;
    instance::get(&conn, id)
}

/// 实例设置页的数据。
#[tauri::command]
pub fn instance_settings_get(state: State<'_, AppState>, id: i64) -> Result<InstanceSettingsView> {
    let conn = state.db()?;
    Ok(instance::get(&conn, id)?.settings_view())
}

/// 保存实例设置（改名会同步改目录名）。
#[tauri::command]
pub fn instance_settings_set(
    state: State<'_, AppState>,
    id: i64,
    settings: InstanceSettingsPatch,
) -> Result<InstanceSettingsView> {
    let conn = state.db()?;
    Ok(instance::update_settings(&conn, &state.paths, id, &settings)?.settings_view())
}

/// 兼容 M2-A 任务书里的 instance_update 名字。
#[tauri::command]
pub fn instance_update(
    state: State<'_, AppState>,
    id: i64,
    patch: InstanceSettingsPatch,
) -> Result<Instance> {
    let conn = state.db()?;
    instance::update_settings(&conn, &state.paths, id, &patch)
}

#[tauri::command]
pub fn instance_open_folder(state: State<'_, AppState>, id: i64) -> Result<()> {
    let conn = state.db()?;
    let target = instance::get(&conn, id)?;
    shell::open_folder(&instance::instance_dir(&state.paths, &target.name))
}

/// 选择实例图标：复制成 <实例目录>/icon.png，返回新路径；取消返回空字符串。
#[tauri::command]
pub fn instance_pick_icon(state: State<'_, AppState>, id: i64) -> Result<String> {
    let conn = state.db()?;
    let target = instance::get(&conn, id)?;
    let picked = shell::pick_png_file()?;
    if picked.trim().is_empty() {
        return Ok(String::new());
    }
    let icon = instance::icon_file(&state.paths, &target.name);
    if let Some(parent) = icon.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(Path::new(picked.trim()), &icon)
        .map_err(|err| AppError::Other(format!("复制图标失败：{err}")))?;
    let path = icon.display().to_string();
    instance::set_icon(&conn, id, &path)?;
    Ok(path)
}

// ------------------------------------------------------------------ 安装 / 启动

/// 安装一个游戏版本：立刻返回，进度与失败走 install://progress / install://error。
#[tauri::command]
pub async fn version_install(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: i64,
    version_id: String,
) -> Result<()> {
    let version_id = version_id.trim().to_string();
    if version_id.is_empty() {
        return Err(AppError::Invalid("游戏版本不能为空".to_string()));
    }
    {
        let conn = state.db()?;
        let target = instance::get(&conn, instance_id)?;
        if target.mc_version != version_id {
            tracing::info!(
                instance = %target.name,
                instance_version = %target.mc_version,
                install_version = %version_id,
                "实例版本与安装版本不一致，仍按请求的版本安装"
            );
        }
    }

    let paths = state.paths.clone();
    let client = state.client.clone();
    let settings = state.settings();
    let target_dir = paths.versions.join(&version_id);
    let task = state.tasks.create(
        "install",
        &format!("{version_id} 版本安装"),
        &target_dir.display().to_string(),
    );
    let task_bg = task.clone();
    let events = events_of(&app);

    // 立刻返回：安装进度靠事件推给前端
    tauri::async_runtime::spawn(async move {
        let origins = sources::resolve_with_probe(&client, &settings).await;
        let installer = Installer::new(
            client,
            paths,
            origins,
            events,
            settings.effective_concurrency(),
            settings.effective_connections(),
        );
        match installer.install(&version_id, &task_bg).await {
            Ok(report) => {
                tracing::info!("{}", install::summarize(&report));
                task_bg.finish();
            }
            Err(AppError::Cancelled) => task_bg.cancel(),
            Err(err) => {
                tracing::error!(%err, "版本安装失败");
                task_bg.fail(&err.to_string());
            }
        }
    });
    Ok(())
}

/// 启动实例：先自检补齐依赖 → 选 Java → 起进程 → 后台转发日志。
#[tauri::command]
pub async fn instance_launch(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    let paths = state.paths.clone();
    let client = state.client.clone();
    let settings = state.settings();
    let target = {
        let conn = state.db()?;
        instance::get(&conn, id)?
    };
    let task = state.tasks.create(
        "launch",
        &format!("启动 {}", target.name),
        &instance::instance_dir(&paths, &target.name)
            .display()
            .to_string(),
    );
    let events = events_of(&app);

    let result = launch_instance(paths, client, events, &task, target, settings, None).await;
    match result {
        Ok(_pid) => {
            task.finish();
            Ok(())
        }
        Err(AppError::Cancelled) => {
            task.cancel();
            Err(AppError::Cancelled)
        }
        Err(err) => {
            task.fail(&err.to_string());
            Err(err)
        }
    }
}

/// 启动并直接进入某个存档（1.20+ 的 --quickPlaySingleplayer）。
#[tauri::command]
pub async fn instance_launch_with_save(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: i64,
    level_name: String,
) -> Result<LaunchStarted> {
    let paths = state.paths.clone();
    let client = state.client.clone();
    let settings = state.settings();
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    if !crate::updates::supports_quick_play(&target.mc_version) {
        return Err(AppError::Invalid(crate::updates::quick_play_rejection(
            &target.mc_version,
        )));
    }
    let level = level_name.trim().to_string();
    crate::mods::validate_save_name(&level)?;
    let saves = crate::mods::saves_dir(&paths, &target.name);
    if !saves.join(&level).is_dir() {
        return Err(AppError::Invalid(format!("存档不存在：{level}")));
    }
    let task = state.tasks.create(
        "launch",
        &format!("启动 {} 并进入存档 {level}", target.name),
        &instance::instance_dir(&paths, &target.name)
            .display()
            .to_string(),
    );
    let events = events_of(&app);
    let result = launch_instance(
        paths,
        client,
        events,
        &task,
        target,
        settings,
        Some(level.clone()),
    )
    .await;
    match result {
        Ok(pid) => {
            task.finish();
            let pid = pid.ok_or_else(|| AppError::Other("游戏进程没有返回 pid".to_string()))?;
            tracing::info!(%level, pid, "已带存档启动游戏");
            Ok(LaunchStarted { pid })
        }
        Err(AppError::Cancelled) => {
            task.cancel();
            Err(AppError::Cancelled)
        }
        Err(err) => {
            task.fail(&err.to_string());
            Err(err)
        }
    }
}

/// instance_launch_with_save 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchStarted {
    pub pid: u32,
}

async fn launch_instance(
    paths: Paths,
    client: reqwest::Client,
    events: Arc<dyn Events>,
    task: &crate::tasks::TaskHandle,
    target: Instance,
    settings: Settings,
    quick_play: Option<String>,
) -> Result<Option<u32>> {
    let origins = sources::resolve_with_probe(&client, &settings).await;

    // 1) 依赖自检：缺库/缺 assets/缺 natives 先补齐
    let installer = Installer::new(
        client.clone(),
        paths.clone(),
        origins.clone(),
        events.clone(),
        settings.effective_concurrency(),
        settings.effective_connections(),
    );
    installer.ensure_ready(&target.mc_version, task).await?;
    let merged = install::load_merged(&paths, &target.mc_version)?;
    // 版本 json 没写 javaVersion.majorVersion 时按 MC 版本推断（1.20.5+ 需要 21）
    let required_java = merged
        .java_major
        .unwrap_or_else(|| java::infer_major_for_mc_version(&target.mc_version));

    // 2) 选 Java：实例指定 → 设置指定 → 扫描 → 自动下载
    let java_path = pick_java(&paths, &target, &settings, required_java, &client, origins, task).await?;

    // 3) 账号：accounts.json 里的当前账号，没有就用默认离线名
    let (username, uuid, access_token) = crate::accounts::current_credentials(&paths)
        .unwrap_or_else(|| {
            let name = launch::DEFAULT_USERNAME.to_string();
            let uuid = crate::accounts::offline_uuid(&name);
            (name, uuid, String::new())
        });

    let memory_mb = if target.memory_mb > 0 {
        target.memory_mb
    } else {
        settings.memory_mb
    };
    let options = LaunchOptions {
        instance_id: target.id,
        instance_name: target.name.clone(),
        version_id: target.mc_version.clone(),
        game_dir: instance::game_dir(&paths, &target.name),
        java_path,
        memory_mb,
        width: target.window_width,
        height: target.window_height,
        username,
        uuid,
        access_token,
        quick_play,
    };

    // 4) 拼参数 → 起进程
    let command = launch::build_command(&paths, &merged, &options)?;
    tracing::info!(command = %command.to_display(), "即将启动游戏");
    let session = launch::start(events, paths, &options, &command).await?;
    tracing::info!(
        instance = %target.name,
        pid = ?session.pid,
        quick_play = ?options.quick_play,
        log = %session.log_path.display(),
        "已拉起游戏进程"
    );
    Ok(session.pid)
}

/// 选 Java：实例指定 → 设置指定 → 磁盘扫描 → （允许时）自动下载。
async fn pick_java(
    paths: &Paths,
    target: &Instance,
    settings: &Settings,
    required: u32,
    client: &reqwest::Client,
    origins: Vec<sources::Origin>,
    task: &crate::tasks::TaskHandle,
) -> Result<PathBuf> {
    if !target.java_path.trim().is_empty() {
        let path = PathBuf::from(target.java_path.trim());
        if path.is_file() {
            return Ok(path);
        }
        tracing::warn!(path = %path.display(), "实例指定的 Java 不存在，回落到自动探测");
    }
    java::ensure_for_launch(client.clone(), paths, settings, required, origins, task).await
}

// ------------------------------------------------------------------ 下载中心

#[tauri::command]
pub fn downloads_list(state: State<'_, AppState>) -> Vec<DownloadTaskView> {
    state.tasks.list()
}

#[tauri::command]
pub fn download_cancel(state: State<'_, AppState>, id: String) -> Result<()> {
    state.tasks.cancel(&id)
}

#[tauri::command]
pub fn download_open_folder(state: State<'_, AppState>, id: String) -> Result<()> {
    let dir = state
        .tasks
        .target_dir(&id)
        .ok_or_else(|| AppError::Invalid(format!("下载任务没有落地目录：{id}")))?;
    shell::open_folder(Path::new(&dir))
}

// ------------------------------------------------------------------ 下载源

/// 源表：内置 official/bmclapi/sjtu + custom + auto，带 selectable 供 UI 判断。
#[tauri::command]
pub fn download_sources(state: State<'_, AppState>) -> Vec<SourceInfo> {
    sources::sources_table(&state.settings())
}

/// 对 official / bmclapi / custom 各测一次速（并发、8 秒超时、取中位数）。
#[tauri::command]
pub async fn sources_probe(state: State<'_, AppState>) -> Result<Vec<SourceProbe>> {
    let settings = state.settings();
    let candidates = sources::auto_candidates(&settings.custom_mirror_url);
    Ok(sources::probe_all(&state.client, &candidates).await)
}

// ------------------------------------------------------------------ Java

/// java_install 的返回：立刻拿到 taskId，进度走 install://progress。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaInstallStarted {
    pub task_id: String,
}

/// 系统里的 Java（同一 JDK 只出一条，可执行文件统一到 bin\javaw.exe）。
#[tauri::command]
pub async fn java_list(state: State<'_, AppState>) -> Result<Vec<JavaRuntime>> {
    let settings = state.settings();
    Ok(java::list(&state.paths, &settings.java_path).await)
}

/// 下载面板：可用的下载源（Oracle 标 requiresLicense）。
#[tauri::command]
pub fn java_sources() -> Vec<JavaSourceInfo> {
    java::sources_table()
}

/// 下载面板：并发测速（8 秒超时，中位数）。
#[tauri::command]
pub async fn java_probe_sources(state: State<'_, AppState>) -> Result<Vec<JavaSourceProbe>> {
    Ok(java::probe_sources(&state.client).await)
}

/// 下载面板：version 列表（source 不填=所有源，major 不填=21）。
#[tauri::command]
pub async fn java_versions(
    state: State<'_, AppState>,
    source: Option<String>,
    major: Option<u32>,
) -> Result<Vec<JavaVersionInfo>> {
    Ok(java::versions(&state.client, source.as_deref(), major).await)
}

/// 下载面板：装一个指定版本（进度 install://progress，taskId 前缀 java:）。
#[tauri::command]
pub async fn java_install(
    state: State<'_, AppState>,
    source: String,
    version: String,
    major: u32,
) -> Result<JavaInstallStarted> {
    let paths = state.paths.clone();
    let client = state.client.clone();
    let settings = state.settings();
    let target = java::runtime_dir(&paths, &source, major, &version);
    let task = state.tasks.create_with_prefix(
        "java",
        &format!("Java {major}（{source}）"),
        &target.display().to_string(),
        "java:",
    );
    let task_id = task.id().to_string();
    let origins = sources::resolve(&settings);
    tauri::async_runtime::spawn(async move {
        match java::install(client, &paths, &source, &version, major, origins, &task).await {
            Ok(runtime) => {
                tracing::info!(
                    major = runtime.major,
                    vendor = %runtime.vendor,
                    path = %runtime.path,
                    "Java 已安装"
                );
                task.finish();
            }
            Err(AppError::Cancelled) => task.cancel(),
            Err(err) => {
                tracing::error!(%err, "Java 安装失败");
                task.fail(&err.to_string());
            }
        }
    });
    Ok(JavaInstallStarted { task_id })
}

/// 老接口（被 java_install 取代）：自动挑最快源装一个主版本，装完返回运行时。
#[tauri::command]
pub async fn java_download(state: State<'_, AppState>, major: u32) -> Result<JavaRuntime> {
    let paths = state.paths.clone();
    let client = state.client.clone();
    let settings = state.settings();
    let task = state.tasks.create_with_prefix(
        "java",
        &format!("Java {major}"),
        &paths.runtimes.display().to_string(),
        "java:",
    );
    let origins = sources::resolve(&settings);
    let result = java::auto_install(client, &paths, major, origins, &task).await;
    match result {
        Ok((runtime, source)) => {
            tracing::info!(source, major, "Java 下载完成");
            task.finish();
            Ok(runtime)
        }
        Err(AppError::Cancelled) => {
            task.cancel();
            Err(AppError::Cancelled)
        }
        Err(err) => {
            task.fail(&err.to_string());
            Err(err)
        }
    }
}

// ------------------------------------------------------------------ 数据目录

/// data_dir_migrate 的返回：进度走 install://progress（taskId 前缀 migrate:）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDirMigrateStarted {
    pub task_id: String,
    /// 数据根换地方必须重启才生效
    pub restart_required: bool,
}

/// 当前生效的数据根（设置里写了 dataRoot 就用它，重启后真正切过去）。
fn effective_data_root(paths: &Paths, settings: &Settings) -> PathBuf {
    let trimmed = settings.data_root.trim();
    if trimmed.is_empty() {
        paths.root.clone()
    } else {
        PathBuf::from(trimmed)
    }
}

/// data_dir_get：路径 + 体积 + 文件数 + 是否默认。
#[tauri::command]
pub async fn data_dir_get(state: State<'_, AppState>) -> Result<crate::datadir::DataDirInfo> {
    let settings = state.settings();
    let root = effective_data_root(&state.paths, &settings);
    let is_default = settings.data_root.trim().is_empty();
    tauri::async_runtime::spawn_blocking(move || crate::datadir::info(&root, is_default))
        .await
        .map_err(|err| AppError::Other(format!("统计数据目录失败：{err}")))
}

/// data_dir_pick：系统目录选择框（复用 shell 里的实现）。
#[tauri::command]
pub fn data_dir_pick() -> Result<String> {
    shell::pick_directory()
}

/// data_dir_migrate：复制 + 校验（文件数 + 抽样 sha256）→ 成功才改 settings.dataRoot。
#[tauri::command]
pub async fn data_dir_migrate(
    app: AppHandle,
    state: State<'_, AppState>,
    target: String,
) -> Result<DataDirMigrateStarted> {
    if state
        .tasks
        .list()
        .iter()
        .any(|task| task.kind == "migrate" && !task.phase.is_terminal())
    {
        return Err(AppError::Invalid(
            "已经有一个数据目录迁移在进行中，请等它结束".to_string(),
        ));
    }
    let target_path = PathBuf::from(target.trim());
    crate::datadir::validate_target(&state.paths.root, &target_path)?;
    let task = state.tasks.create_with_prefix(
        "migrate",
        &format!("数据目录迁移到 {}", target_path.display()),
        &target_path.display().to_string(),
        "migrate:",
    );
    let task_id = task.id().to_string();
    tracing::info!(
        from = %state.paths.root.display(),
        to = %target_path.display(),
        "开始迁移数据目录"
    );
    tauri::async_runtime::spawn(async move {
        let managed = app.state::<AppState>();
        let current = managed.paths.clone();
        let copy_task = task.clone();
        let copy_target = target_path.clone();
        let joined = tokio::task::spawn_blocking(move || {
            crate::datadir::migrate(&current, &copy_target, &copy_task)
        })
        .await;
        match joined {
            Ok(Ok(())) => {
                let mut next = managed.settings();
                next.data_root = target_path.display().to_string();
                match managed.set_settings(next) {
                    Ok(saved) => {
                        tracing::info!(
                            data_root = %saved.data_root,
                            "数据目录已迁移，重启后生效（原目录保留）"
                        );
                        task.finish();
                    }
                    Err(err) => {
                        tracing::error!(%err, "迁移完成但写入设置失败");
                        task.fail(&format!("迁移完成但写入设置失败：{err}"));
                    }
                }
            }
            Ok(Err(err)) => {
                tracing::error!(%err, "数据目录迁移失败，已回滚");
                task.fail(&err.to_string());
            }
            Err(err) => {
                tracing::error!(%err, "迁移任务异常退出");
                task.fail(&format!("迁移任务异常退出：{err}"));
            }
        }
    });
    Ok(DataDirMigrateStarted {
        task_id,
        restart_required: true,
    })
}

/// data_dir_reset_default：把数据根恢复成 <安装目录>\.mc（重启后生效）。
#[tauri::command]
pub fn data_dir_reset_default(state: State<'_, AppState>) -> Result<()> {
    let mut next = state.settings();
    next.data_root = String::new();
    let saved = state.set_settings(next)?;
    tracing::info!(
        data_root = %saved.data_root,
        "数据目录已复位为默认（重启后生效）"
    );
    Ok(())
}

// ------------------------------------------------------------------ 存储统计 / 清理 / 最近游玩

/// storage_stats：7 项目录占用（spawn_blocking + 60 秒缓存）。
#[tauri::command]
pub async fn storage_stats(state: State<'_, AppState>) -> Result<crate::storage::StorageStats> {
    if let Some(hit) = crate::storage::cached_stats() {
        tracing::debug!("存储统计命中缓存");
        return Ok(hit);
    }
    let paths = state.paths.clone();
    let started = std::time::Instant::now();
    let stats = tauri::async_runtime::spawn_blocking(move || crate::storage::stats(&paths))
        .await
        .map_err(|err| AppError::Other(format!("统计存储占用失败：{err}")))?;
    crate::storage::store_stats(&stats);
    tracing::info!(
        total_bytes = stats.total_bytes,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "存储统计完成"
    );
    Ok(stats)
}

/// storage_clean：有下载任务在跑就拒绝；否则按 key 清理。
#[tauri::command]
pub async fn storage_clean(
    state: State<'_, AppState>,
    key: String,
) -> Result<crate::storage::CleanResult> {
    if let Some(message) = crate::storage::busy_message(&state.tasks) {
        return Err(AppError::Invalid(message));
    }
    let paths = state.paths.clone();
    let referenced: Vec<String> = {
        let conn = state.db()?;
        instance::list(&conn)?
            .into_iter()
            .map(|item| item.mc_version)
            .collect()
    };
    let key_for_task = key.clone();
    let now = util::now_unix();
    let freed = tauri::async_runtime::spawn_blocking(move || {
        crate::storage::clean(&paths, &key_for_task, now, &referenced)
    })
    .await
    .map_err(|err| AppError::Other(format!("清理失败：{err}")))??;
    crate::storage::invalidate_cache();
    tracing::info!(key = %key, freed_bytes = freed, "存储清理完成");
    Ok(crate::storage::CleanResult {
        freed_bytes: freed,
    })
}

/// instances_recent：按最后游玩时间倒序（没玩过的排最后）。
#[tauri::command]
pub fn instances_recent(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<Instance>> {
    let conn = state.db()?;
    instance::recent(&conn, limit.unwrap_or(6).clamp(1, 50))
}

// ------------------------------------------------------------------ 本地开服

/// server_create 的返回：立刻拿到 id + taskId，进度走 install://progress（server: 前缀）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCreateStarted {
    pub id: i64,
    pub task_id: String,
}

/// 端口占用检查（netstat，隐藏窗口）。
fn port_in_use_error(port: u16, message: String) -> AppError {
    AppError::Invalid(format!("{message}（端口 {port}）"))
}

/// 选服务器用的 Java：设置 → 扫描 → 自动下载（失败时回落到 PATH 上的 javaw）。
async fn resolve_server_java(
    paths: &Paths,
    settings: &Settings,
    mc_version: &str,
    client: &reqwest::Client,
    task: &crate::tasks::TaskHandle,
) -> Result<PathBuf> {
    let required = java::infer_major_for_mc_version(mc_version);
    if !settings.java_path.trim().is_empty() {
        let path = PathBuf::from(settings.java_path.trim());
        if path.is_file() {
            return Ok(path);
        }
    }
    let runtimes = java::list(paths, &settings.java_path).await;
    if let Some(found) = java::pick_for_major(&runtimes, required) {
        return Ok(PathBuf::from(&found.java_exe));
    }
    if !settings.auto_download_java {
        return Err(java::missing_java_error(required, "autoDownloadJava 已关闭"));
    }
    let origins = sources::resolve(settings);
    match java::auto_install(client.clone(), paths, required, origins, task).await {
        Ok((runtime, source)) => {
            let name = java::source_display_name(&source);
            tracing::info!(
                "服务端缺少 Java {}，已自动下载 {} 的 Java {}",
                required,
                name,
                required
            );
            Ok(PathBuf::from(runtime.java_exe))
        }
        Err(AppError::Cancelled) => Err(AppError::Cancelled),
        Err(err) => Err(java::missing_java_error(required, &err.to_string())),
    }
}

fn server_events(app: &AppHandle, id: i64, state: ServerState, players: Option<u32>) {
    use tauri::Emitter;
    let _ = app.emit(
        "server://state",
        serde_json::json!({ "id": id, "state": state, "players": players }),
    );
}

/// 读服务器的 Java（start.bat 里要用）。
fn server_java_from_settings(settings: &Settings) -> PathBuf {
    let trimmed = settings.java_path.trim();
    if trimmed.is_empty() {
        PathBuf::from("javaw.exe")
    } else {
        PathBuf::from(trimmed)
    }
}

#[tauri::command]
pub async fn server_cores(
    state: State<'_, AppState>,
    mc_version: Option<String>,
) -> Result<Vec<ServerCore>> {
    let mut table = server::cores_table();
    if let Some(mc_version) = mc_version.filter(|value| !value.trim().is_empty()) {
        let client = state.client.clone();
        let mut handles = Vec::new();
        for core in server::CORE_IDS {
            let client = client.clone();
            let mc = mc_version.clone();
            handles.push(tokio::spawn(async move {
                (core, server::versions(&client, core, Some(&mc), None).await)
            }));
        }
        for handle in handles {
            let Ok((core, list)) = handle.await else { continue };
            let Some(found) = list.into_iter().next() else {
                continue;
            };
            if let Some(item) = table.iter_mut().find(|item| item.id == core) {
                item.latest_version = found.build.clone().or(Some(found.version));
            }
        }
    }
    Ok(table)
}

#[tauri::command]
pub async fn server_versions(
    state: State<'_, AppState>,
    core: String,
    mc_version: Option<String>,
    manifest: Option<String>,
) -> Result<Vec<ServerVersion>> {
    let mc = mc_version.unwrap_or_default();
    Ok(server::versions(&state.client, &core, Some(mc.as_str()), manifest.as_deref()).await)
}

#[tauri::command]
pub async fn server_create(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    core: String,
    mc_version: String,
    memory_mb: u32,
    port: u16,
    eula: bool,
    accept_eula: bool,
) -> Result<ServerCreateStarted> {
    let paths = state.paths.clone();
    let client = state.client.clone();
    let settings = state.settings();
    let mc_version = mc_version.trim().to_string();
    if mc_version.is_empty() {
        return Err(AppError::Invalid("MC 版本不能为空".to_string()));
    }
    if !server::CORE_IDS.contains(&core.as_str()) {
        return Err(AppError::Invalid(format!("未知的服务端核心：{core}")));
    }
    let accepted = eula && accept_eula;
    if !accepted {
        return Err(AppError::Invalid(
            "必须勾选「我已阅读并同意 Minecraft EULA」才能创建服务器".to_string(),
        ));
    }
    if memory_mb < 512 {
        return Err(AppError::Invalid(format!("内存至少 512MB，收到 {memory_mb}")));
    }
    // 名字与端口先在命令层挡住，用户能立刻看到错误
    let _ = server::validate_name(&paths, &name)?;
    if let Some(message) = server::port_conflict(port).await {
        return Err(port_in_use_error(port, message));
    }
    let dir_guess = server::layout(&paths, &name).dir;
    let task = state.tasks.create_with_prefix(
        "server",
        &format!("{} {}", server::core_name(&core), mc_version),
        &dir_guess.display().to_string(),
        "server:",
    );
    let task_id = task.id().to_string();
    let id = crate::util::now_millis() as i64;
    let origins = sources::resolve(&settings);
    let java = server_java_from_settings(&settings);
    let app_for_task = app.clone();
    let name_for_task = name.clone();
    tauri::async_runtime::spawn(async move {
        let managed = app_for_task.state::<AppState>();
        let result = server::create(
            client,
            &paths,
            &task,
            &core,
            &mc_version,
            &name_for_task,
            port,
            memory_mb,
            accepted,
            &java,
            origins,
        )
        .await;
        match result {
            Ok(mut meta) => {
                // 目录里写的是真实创建时间，id 以命令层算出来的为准（前端已经拿到）
                meta.id = id;
                let dir = server::servers_root(&paths).join(util::sanitize_file_part(&meta.name));
                if let Err(err) = server::write_meta(&dir, &meta) {
                    tracing::warn!(%err, "回写服务器 id 失败");
                }
                tracing::info!(id, name = %meta.name, core, mc_version, port, "服务器创建完成");
                task.finish();
            }
            Err(AppError::Cancelled) => task.cancel(),
            Err(err) => {
                tracing::error!(%err, "创建服务器失败");
                task.fail(&err.to_string());
            }
        }
        let _ = managed;
    });
    Ok(ServerCreateStarted { id, task_id })
}

#[tauri::command]
pub async fn server_list(state: State<'_, AppState>) -> Result<Vec<ServerView>> {
    let mut out = Vec::new();
    for meta in server::list(&state.paths) {
        let (server_state, players) = state.servers.state_of(meta.id).await;
        out.push(ServerView {
            id: meta.id,
            name: meta.name.clone(),
            core: meta.core.clone(),
            mc_version: meta.mc_version.clone(),
            port: meta.port,
            memory_mb: meta.memory_mb,
            state: server_state,
            players,
            dir: server::servers_root(&state.paths)
                .join(util::sanitize_file_part(&meta.name))
                .display()
                .to_string(),
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn server_start(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    if state.servers.is_running(id).await {
        return Err(AppError::Invalid("服务器已经在运行".to_string()));
    }
    let (dir, meta) = server::find(&state.paths, id)?;
    let eula_text = std::fs::read_to_string(dir.join("eula.txt")).unwrap_or_default();
    if !eula_text.to_ascii_lowercase().contains("eula=true") {
        return Err(AppError::Invalid(
            "还没有同意 Minecraft EULA：请把 eula.txt 里的 eula 改成 true（或在创建时勾选同意）"
                .to_string(),
        ));
    }
    if let Some(message) = server::port_conflict(meta.port).await {
        return Err(port_in_use_error(meta.port, message));
    }
    let settings = state.settings();
    let task = state.tasks.create_with_prefix(
        "server",
        &format!("启动 {}", meta.name),
        &dir.display().to_string(),
        "server:",
    );
    let java = resolve_server_java(&state.paths, &settings, &meta.mc_version, &state.client, &task)
        .await?;
    tracing::info!(id, java = %java.display(), "正在启动服务器");

    let spawned = server::spawn(&dir, &meta, &java)?;
    let server::SpawnedServer {
        child,
        stdin,
        stdout,
        stderr,
    } = spawned;
    state
        .servers
        .insert(
            id,
            server::LiveServer {
                child,
                stdin,
                state: ServerState::Starting,
                players: 0,
                history: Vec::new(),
            },
        )
        .await;
    server_events(&app, id, ServerState::Starting, None);

    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut out_lines = stdout.map(|stream| tokio::io::BufReader::new(stream).lines());
        let mut err_lines = stderr.map(|stream| tokio::io::BufReader::new(stream).lines());
        loop {
            let line = tokio::select! {
                result = async {
                    match out_lines.as_mut() {
                        Some(lines) => lines.next_line().await,
                        None => std::future::pending().await,
                    }
                } => result,
                result = async {
                    match err_lines.as_mut() {
                        Some(lines) => lines.next_line().await,
                        None => std::future::pending().await,
                    }
                } => result,
            };
            match line {
                Ok(Some(line)) => {
                    use tauri::Emitter;
                    let _ = app_for_task.emit(
                        "server://log",
                        serde_json::json!({ "id": id, "line": line }),
                    );
                    if server::is_server_ready(&line) {
                        let managed = app_for_task.state::<AppState>();
                        managed.servers.set_state(id, ServerState::Running).await;
                        server_events(&app_for_task, id, ServerState::Running, Some(0));
                        tracing::info!(id, "服务器已就绪（Done）");
                    } else if let Some((joined, name)) = server::parse_player_event(&line) {
                        let managed = app_for_task.state::<AppState>();
                        managed.servers.bump_players(id, joined).await;
                        let (state_now, players) = managed.servers.state_of(id).await;
                        server_events(&app_for_task, id, state_now, players);
                        tracing::info!(id, %name, joined, "服务器玩家变化");
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!(id, %err, "读取服务器输出失败");
                    break;
                }
            }
        }
        let managed = app_for_task.state::<AppState>();
        let code = managed.servers.wait_finished(id).await;
        tracing::info!(id, code = ?code, "服务器进程已退出");
        server_events(&app_for_task, id, ServerState::Stopped, None);
    });
    Ok(())
}

#[tauri::command]
pub async fn server_stop(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    if !state.servers.is_running(id).await {
        return Err(AppError::Invalid("服务器没有在运行".to_string()));
    }
    server_events(&app, id, ServerState::Stopping, None);
    let sent = state.servers.request_stop(id).await;
    tracing::info!(id, sent_stdin = sent, "正在停止服务器（stdin: stop）");
    state.servers.wait_exit(id).await;
    server_events(&app, id, ServerState::Stopped, None);
    tracing::info!(id, "服务器已停止");
    Ok(())
}

#[tauri::command]
pub async fn server_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    if state.servers.is_running(id).await {
        return Err(AppError::Invalid("服务器还在运行，先停止再删除".to_string()));
    }
    let (dir, meta) = server::find(&state.paths, id)?;
    std::fs::remove_dir_all(&dir)?;
    tracing::info!(id, name = %meta.name, "已删除服务器目录");
    Ok(())
}

#[tauri::command]
pub fn server_open_folder(state: State<'_, AppState>, id: i64) -> Result<()> {
    let (dir, _meta) = server::find(&state.paths, id)?;
    shell::open_folder(&dir)
}

#[tauri::command]
pub fn server_read_properties(
    state: State<'_, AppState>,
    id: i64,
) -> Result<std::collections::BTreeMap<String, String>> {
    let (dir, _meta) = server::find(&state.paths, id)?;
    let text = std::fs::read_to_string(dir.join("server.properties")).map_err(|err| {
        AppError::Other(format!("读取 server.properties 失败（{}）：{err}", dir.display()))
    })?;
    let set = server::PropertySet::parse(&text);
    let mut out = std::collections::BTreeMap::new();
    for line in set.lines {
        if let server::PropertyLine::Entry { key, value } = line {
            out.insert(key, value);
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn server_write_properties(
    state: State<'_, AppState>,
    id: i64,
    patch: std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let (dir, _meta) = server::find(&state.paths, id)?;
    let path = dir.join("server.properties");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut set = server::PropertySet::parse(&text);
    set.patch(&patch);
    std::fs::write(&path, set.render())?;
    tracing::info!(id, fields = patch.len(), "已写入 server.properties");
    Ok(())
}

// ------------------------------------------------------------------ 日志 / 打开目录

#[tauri::command]
pub fn logs_list(state: State<'_, AppState>, instance_id: Option<i64>) -> Result<Vec<LogFile>> {
    let name = match instance_id {
        Some(id) => {
            let conn = state.db()?;
            Some(instance::get(&conn, id)?.name)
        }
        None => None,
    };
    shell::list_logs(&state.paths, name.as_deref())
}

#[tauri::command]
pub fn log_read(state: State<'_, AppState>, instance_id: Option<i64>, name: String) -> Result<String> {
    let _ = instance_id;
    shell::read_log(&state.paths, &name)
}

#[tauri::command]
pub fn log_open_folder(state: State<'_, AppState>) -> Result<()> {
    shell::open_folder(&state.paths.logs)
}

/// 写剪贴板：前端 navigator.clipboard 不可用时的兜底。
#[tauri::command]
pub fn clipboard_write(text: String) -> Result<()> {
    shell::clipboard_write(&text)
}

/// open_path(kind, id)：实例目录 / 日志目录 / 数据根 / versions。
#[tauri::command]
pub fn open_path(state: State<'_, AppState>, kind: String, id: Option<i64>) -> Result<()> {
    match kind.as_str() {
        "root" => shell::open_folder(&state.paths.root),
        "logs" => shell::open_folder(&state.paths.logs),
        "versions" => shell::open_folder(&state.paths.versions),
        "instance" => {
            let id = id.ok_or_else(|| AppError::Invalid("缺少实例 id".to_string()))?;
            let conn = state.db()?;
            let target = instance::get(&conn, id)?;
            shell::open_folder(&instance::instance_dir(&state.paths, &target.name))
        }
        other => Err(AppError::Invalid(format!("不支持的打开类型：{other}"))),
    }
}

// ------------------------------------------------------------------ 娱乐 / 内容服务 / GitHub 下载器

/// 每日一言：主进程代发，失败返回 ok=false（前端用兜底文案）。
#[tauri::command]
pub async fn fun_saying(state: State<'_, AppState>) -> Result<crate::saying::SayingResult> {
    let settings = state.settings();
    Ok(crate::saying::fetch(&state.client, &settings).await)
}

/// 内容服务：测试自定义翻译接口是否可用（错误信息里 API Key 一律打码）。
#[tauri::command]
pub async fn translation_test(
    state: State<'_, AppState>,
) -> Result<crate::translate::TranslationTestResult> {
    let settings = state.settings();
    Ok(crate::translate::test_connection(&state.client, &settings).await)
}

/// github_download 的返回：立刻拿 taskId，进度走 install://progress。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubDownloadStarted {
    pub task_id: String,
}

/// 从 GitHub 下载一个文件（白名单校验 + 加速节点 + 分片并发）。
#[tauri::command]
pub async fn github_download(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    dir: Option<String>,
    file_name: Option<String>,
) -> Result<GithubDownloadStarted> {
    if !crate::github_dl::is_github_url(&url) {
        return Err(AppError::Invalid(format!(
            "只支持从 GitHub 下载（github.com / raw.githubusercontent.com 等），收到：{url}"
        )));
    }
    let paths = state.paths.clone();
    let client = state.client.clone();
    let settings = state.settings();
    let dir = dir.unwrap_or_default();
    let file_name = file_name.unwrap_or_default();
    let target_dir = crate::github_dl::resolve_dir(&paths, &settings, &dir);
    let title = if file_name.trim().is_empty() {
        crate::github_dl::infer_file_name(&url).unwrap_or_else(|| "GitHub 下载".to_string())
    } else {
        file_name.trim().to_string()
    };
    let task = state.tasks.create_with_prefix(
        "github",
        &title,
        &target_dir.display().to_string(),
        "github",
    );
    let task_id = task.id().to_string();
    let task_bg = task.clone();
    let _events = events_of(&app);

    tauri::async_runtime::spawn(async move {
        match crate::github_dl::download(
            client,
            paths,
            settings,
            task_bg.clone(),
            url,
            dir,
            file_name,
        )
        .await
        {
            Ok(target) => {
                tracing::info!(target = %target.display(), "GitHub 下载完成");
                task_bg.finish();
            }
            Err(AppError::Cancelled) => task_bg.cancel(),
            Err(err) => {
                tracing::error!(%err, "GitHub 下载失败");
                task_bg.fail(&err.to_string());
            }
        }
    });
    Ok(GithubDownloadStarted { task_id })
}

/// 取消一个 GitHub 下载任务（半成品文件由下载流程自己删）。
#[tauri::command]
pub fn github_cancel(state: State<'_, AppState>, task_id: String) -> Result<()> {
    state.tasks.cancel(&task_id)
}

/// 打开 GitHub 下载默认目录。
#[tauri::command]
pub fn github_open_folder(state: State<'_, AppState>) -> Result<()> {
    let settings = state.settings();
    let dir = crate::github_dl::resolve_dir(&state.paths, &settings, "");
    std::fs::create_dir_all(&dir)?;
    shell::open_folder(&dir)
}

/// 系统目录选择框；取消返回空字符串。
#[tauri::command]
pub fn pick_directory() -> Result<String> {
    shell::pick_directory()
}

// ------------------------------------------------------------------ 实例内页（M3）

#[tauri::command]
pub fn mods_list(
    state: State<'_, AppState>,
    instance_id: i64,
    kind: String,
) -> Result<Vec<crate::mods::ModFileItem>> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let kind = crate::mods::ModKind::from_id(&kind)?;
    crate::mods::list_mods(&state.paths, &target.name, kind, Some(target.loader.clone()))
}

#[tauri::command]
pub fn mods_set_enabled(
    state: State<'_, AppState>,
    instance_id: i64,
    kind: String,
    file: String,
    enabled: bool,
) -> Result<Vec<crate::mods::ModFileItem>> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let kind = crate::mods::ModKind::from_id(&kind)?;
    crate::mods::set_enabled(&state.paths, &target.name, kind, &file, enabled)?;
    // 前端直接替换列表：返回改完的整份
    crate::mods::list_mods(&state.paths, &target.name, kind, Some(target.loader.clone()))
}

#[tauri::command]
pub fn mods_delete(
    state: State<'_, AppState>,
    instance_id: i64,
    kind: String,
    file: String,
) -> Result<Vec<crate::mods::ModFileItem>> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let kind = crate::mods::ModKind::from_id(&kind)?;
    crate::mods::delete_file(&state.paths, &target.name, kind, &file)?;
    crate::mods::list_mods(&state.paths, &target.name, kind, Some(target.loader.clone()))
}

/// 导入：paths 为空时由后端弹文件选择框（浏览器环境拿不到绝对路径）。
#[tauri::command]
pub fn mods_import(
    state: State<'_, AppState>,
    instance_id: i64,
    kind: String,
    paths: Vec<String>,
) -> Result<Vec<crate::mods::ModFileItem>> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let kind = crate::mods::ModKind::from_id(&kind)?;
    let sources = if paths.iter().all(|path| path.trim().is_empty()) {
        let filter = match kind {
            crate::mods::ModKind::Mod => "模组文件|*.jar;*.zip",
            crate::mods::ModKind::ResourcePack => "资源包|*.zip",
            crate::mods::ModKind::Shader => "光影包|*.zip",
        };
        shell::pick_files(filter, "选择要导入的文件")?
    } else {
        paths
    };
    if !sources.is_empty() {
        let report = crate::mods::import(&state.paths, &target.name, kind, &sources)?;
        tracing::info!(
            imported = report.imported,
            skipped = report.skipped,
            "实例文件导入完成"
        );
    }
    crate::mods::list_mods(&state.paths, &target.name, kind, Some(target.loader.clone()))
}

/// 批量检查实例内容的更新（记录优先，其次 CurseForge 指纹）。
#[tauri::command]
pub async fn mods_check_updates(
    state: State<'_, AppState>,
    instance_id: i64,
    kind: String,
) -> Result<crate::updates::ModCheckResult> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let kind_id = crate::mods::ModKind::from_id(&kind)?;
    let content_dir = crate::mods::kind_dir(&state.paths, &target.name, kind_id);
    let instance_dir = instance::instance_dir(&state.paths, &target.name);
    let settings = state.settings();
    let key = settings.curseforge_api_key.clone();
    let loader = if target.loader.trim().is_empty() {
        "fabric".to_string()
    } else {
        target.loader.clone()
    };
    crate::updates::check_updates(
        &state.client,
        &instance_dir,
        &content_dir,
        &kind,
        &target.mc_version,
        &loader,
        if key.trim().is_empty() { None } else { Some(key.as_str()) },
    )
    .await
}

/// 应用更新：先备份到 <instance>/.gugu-backup/<时间戳>/，再原子替换。
#[tauri::command]
pub async fn mods_apply_updates(
    state: State<'_, AppState>,
    instance_id: i64,
    kind: String,
    items: Vec<crate::updates::ModUpdateItem>,
) -> Result<crate::updates::ModApplyResult> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let kind_id = crate::mods::ModKind::from_id(&kind)?;
    let content_dir = crate::mods::kind_dir(&state.paths, &target.name, kind_id);
    let instance_dir = instance::instance_dir(&state.paths, &target.name);
    let via_mcim = state.settings().content_download_via_mcim;
    crate::updates::apply_updates(&state.client, &instance_dir, &content_dir, &items, via_mcim).await
}

/// 打开实例内的模组/资源包/光影目录。
#[tauri::command]
pub fn mods_open_folder(
    state: State<'_, AppState>,
    instance_id: i64,
    kind: String,
) -> Result<()> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let kind = crate::mods::ModKind::from_id(&kind)?;
    let dir = crate::mods::kind_dir(&state.paths, &target.name, kind);
    std::fs::create_dir_all(&dir)?;
    shell::open_folder(&dir)
}

/// 打开实例的存档目录。
#[tauri::command]
pub fn saves_open_folder(state: State<'_, AppState>, instance_id: i64) -> Result<()> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let dir = crate::mods::saves_dir(&state.paths, &target.name);
    std::fs::create_dir_all(&dir)?;
    shell::open_folder(&dir)
}

#[tauri::command]
pub fn saves_list(state: State<'_, AppState>, instance_id: i64) -> Result<Vec<crate::mods::SaveItem>> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    crate::mods::list_saves(&state.paths, &target.name)
}

#[tauri::command]
pub fn saves_delete(state: State<'_, AppState>, instance_id: i64, name: String) -> Result<()> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    crate::mods::delete_save(&state.paths, &target.name, &name)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
}

#[tauri::command]
pub fn saves_export(
    state: State<'_, AppState>,
    instance_id: i64,
    name: String,
) -> Result<ExportResult> {
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let path = crate::mods::export_save(&state.paths, &target.name, &name)?;
    Ok(ExportResult {
        path: path.display().to_string(),
    })
}

/// 整实例导出：立刻返回 taskId，打包进度走 install://progress。
#[tauri::command]
pub async fn instance_export(
    state: State<'_, AppState>,
    id: i64,
    target: Option<String>,
) -> Result<GithubDownloadStarted> {
    let paths = state.paths.clone();
    let instance_target = {
        let conn = state.db()?;
        instance::get(&conn, id)?
    };
    let title = format!("导出实例 {}", instance_target.name);
    let export_dir = match target.filter(|value| !value.trim().is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => crate::mods::exports_dir(&paths),
    };
    let task = state.tasks.create_with_prefix(
        "export",
        &title,
        &export_dir.display().to_string(),
        "export",
    );
    let task_id = task.id().to_string();
    let task_bg = task.clone();
    tauri::async_runtime::spawn(async move {
        task_bg.progress_forced(crate::events::Phase::Extracting, "正在打包实例", 0, 0);
        match crate::mods::export_instance(&paths, &instance_target.name) {
            Ok(path) => {
                tracing::info!(path = %path.display(), "实例导出完成");
                task_bg.finish();
            }
            Err(err) => {
                tracing::error!(%err, "实例导出失败");
                task_bg.fail(&err.to_string());
            }
        }
    });
    Ok(GithubDownloadStarted { task_id })
}

// ------------------------------------------------------------------ 服务端详情（M10）

/// serverId 既可能是数字也可能是字符串：统一按 i64 解析。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum ServerIdParam {
    Num(i64),
    Text(String),
}

impl ServerIdParam {
    pub fn value(&self) -> Result<i64> {
        match self {
            ServerIdParam::Num(id) => Ok(*id),
            ServerIdParam::Text(text) => text
                .trim()
                .parse::<i64>()
                .map_err(|_| AppError::Invalid(format!("服务端 id 不合法：{text}"))),
        }
    }
}

#[tauri::command]
pub fn server_content_list(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    kind: String,
) -> Result<Vec<server_content::ServerContentItem>> {
    let (dir, _meta) = server::find(&state.paths, server_id.value()?)?;
    let kind = server_content::ServerContentKind::from_id(&kind)?;
    server_content::list_content(&dir, kind)
}

/// 装一个插件/模组进服务端：立刻返回 taskId，进度走 install://progress。
#[tauri::command]
pub async fn server_content_install(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    provider: String,
    project_id: String,
    version_id: Option<String>,
) -> Result<server_content::InstallStarted> {
    let provider = content::Provider::from_id(&provider)?;
    let (dir, meta) = server::find(&state.paths, server_id.value()?)?;
    let kind = if meta.core.eq_ignore_ascii_case("fabric")
        || meta.core.eq_ignore_ascii_case("forge")
        || meta.core.eq_ignore_ascii_case("neoforge")
    {
        server_content::ServerContentKind::Mod
    } else {
        server_content::ServerContentKind::Plugin
    };
    let settings = state.settings();
    let task = state.tasks.create_with_prefix(
        "content",
        &format!("安装 {} {}", kind.id(), project_id),
        &server_content::kind_dir(&dir, kind).display().to_string(),
        "content",
    );
    let task_id = task.id().to_string();
    let task_bg = task.clone();
    let client = state.client.clone();
    let loader = kind.loader_for(&meta.core);
    let via_mcim = settings.content_download_via_mcim;
    let version_id = version_id.unwrap_or_default();
    tauri::async_runtime::spawn(async move {
        let key = settings.curseforge_api_key.clone();
        match server_content::install(
            &client,
            &dir,
            kind,
            provider,
            &project_id,
            &version_id,
            &meta.mc_version,
            &loader,
            if key.trim().is_empty() { None } else { Some(key.as_str()) },
            via_mcim,
            &task_bg,
        )
        .await
        {
            Ok(path) => {
                tracing::info!(path = %path.display(), "服务端内容安装完成");
                task_bg.finish();
            }
            Err(AppError::Cancelled) => task_bg.cancel(),
            Err(err) => {
                tracing::error!(%err, "服务端内容安装失败");
                task_bg.fail(&err.to_string());
            }
        }
    });
    Ok(server_content::InstallStarted { task_id })
}

#[tauri::command]
pub fn server_content_toggle(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    kind: String,
    file_name: String,
    enabled: bool,
) -> Result<()> {
    let (dir, _meta) = server::find(&state.paths, server_id.value()?)?;
    let kind = server_content::ServerContentKind::from_id(&kind)?;
    server_content::toggle_content(&dir, kind, &file_name, enabled)
}

#[tauri::command]
pub async fn server_content_check(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    kind: String,
) -> Result<server_content::ContentCheckResult> {
    let (dir, meta) = server::find(&state.paths, server_id.value()?)?;
    let kind = server_content::ServerContentKind::from_id(&kind)?;
    let settings = state.settings();
    let key = settings.curseforge_api_key.clone();
    let loader = kind.loader_for(&meta.core);
    server_content::check_content(
        &state.client,
        &dir,
        kind,
        &meta.mc_version,
        &loader,
        if key.trim().is_empty() { None } else { Some(key.as_str()) },
    )
    .await
}

#[tauri::command]
pub async fn server_content_apply_updates(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    kind: String,
    items: Vec<server_content::ContentUpdateItem>,
) -> Result<server_content::ContentApplyResult> {
    let (dir, _meta) = server::find(&state.paths, server_id.value()?)?;
    let kind = server_content::ServerContentKind::from_id(&kind)?;
    let via_mcim = state.settings().content_download_via_mcim;
    server_content::apply_updates(&state.client, &dir, kind, &items, via_mcim).await
}

#[tauri::command]
pub fn server_saves_list(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
) -> Result<Vec<server_content::ServerSaveItem>> {
    let (dir, _meta) = server::find(&state.paths, server_id.value()?)?;
    server_content::list_saves(&dir)
}

#[tauri::command]
pub async fn server_saves_backup(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    name: String,
) -> Result<server_content::BackupResult> {
    let (dir, meta) = server::find(&state.paths, server_id.value()?)?;
    let task = state.tasks.create_with_prefix(
        "backup",
        &format!("备份存档 {name}"),
        &dir.join(server_content::BACKUP_DIR).display().to_string(),
        server_content::BACKUP_PREFIX,
    );
    let task_bg = task.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        server_content::backup_save(&dir, &name, Some(&task_bg))
    })
    .await
    .map_err(|err| AppError::Other(format!("备份任务异常退出：{err}")))?;
    match result {
        Ok(path) => {
            task.finish();
            let path = path.display().to_string();
            tracing::info!(server = meta.name, %path, "服务端存档备份完成");
            Ok(server_content::BackupResult { path })
        }
        Err(err) => {
            task.fail(&err.to_string());
            Err(err)
        }
    }
}

#[tauri::command]
pub fn server_config_read(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
) -> Result<server_content::ServerConfigResult> {
    let (dir, _meta) = server::find(&state.paths, server_id.value()?)?;
    server_content::read_config(&dir)
}

#[tauri::command]
pub fn server_config_write(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    patch: std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let (dir, _meta) = server::find(&state.paths, server_id.value()?)?;
    server_content::write_config(&dir, &patch)
}

/// 服务端控制台：把一条指令写进服务端进程的 stdin。
#[tauri::command]
pub async fn server_send_command(
    app: AppHandle,
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    command: String,
) -> Result<()> {
    let id = server_id.value()?;
    let normalized = server::normalize_command(&command)?;
    state.servers.send_command(id, &normalized).await?;
    // 让前端立刻看到自己发出去的指令
    use tauri::Emitter;
    let _ = app.emit(
        "server://log",
        serde_json::json!({ "id": id, "line": format!("> {normalized}") }),
    );
    Ok(())
}

/// 控制台状态：运行中？stdin 还能写吗？最近 50 条指令。
#[tauri::command]
pub async fn server_console_state(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
) -> Result<server::ConsoleState> {
    Ok(state.servers.console_state(server_id.value()?).await)
}

#[tauri::command]
pub fn server_logs_tail(
    state: State<'_, AppState>,
    server_id: ServerIdParam,
    lines: Option<usize>,
) -> Result<Vec<String>> {
    let (dir, _meta) = server::find(&state.paths, server_id.value()?)?;
    server_content::logs_tail(&dir, lines.unwrap_or(200).clamp(1, 5000))
}

// ------------------------------------------------------------------ 皮肤 / 披风

#[tauri::command]
pub fn skins_list(state: State<'_, AppState>) -> Vec<crate::skins::SkinItem> {
    crate::skins::list_skins(&state.paths)
}

#[tauri::command]
pub fn skin_apply(state: State<'_, AppState>, path: String) -> Result<crate::skins::SkinItem> {
    crate::skins::apply_skin(&state.paths, &path)
}

#[tauri::command]
pub fn skin_upload(
    state: State<'_, AppState>,
    name: String,
    png_base64: String,
) -> Result<crate::skins::SkinItem> {
    crate::skins::upload_skin(&state.paths, &name, &png_base64)
}

/// 删除本地皮肤文件（删的是当前皮肤时回落内置 Steve）。
#[tauri::command]
pub fn skins_delete(state: State<'_, AppState>, name: String) -> Result<()> {
    crate::skins::delete_local_skin(&state.paths, &name)
}

#[tauri::command]
pub fn skin_current(state: State<'_, AppState>) -> Option<crate::skins::SkinItem> {
    crate::skins::current_skin(&state.paths)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultSkin {
    #[serde(flatten)]
    pub item: crate::skins::SkinItem,
    pub sha256: String,
}

/// 随包内置的 9 个官方皮肤（name 缺省是 Steve / Alex；model: classic|slim）。
#[tauri::command]
pub fn skin_default(name: Option<String>, model: Option<String>) -> Result<DefaultSkin> {
    let model = model.unwrap_or_else(|| "classic".to_string());
    let (item, sha256) = crate::skins::default_skin_named(name.as_deref(), &model)?;
    Ok(DefaultSkin { item, sha256 })
}

/// 正版玩家皮肤：走 uapis 的 userinfo 接口，下载后落盘（不自动应用）。
#[tauri::command]
pub async fn skin_fetch_mojang(
    state: State<'_, AppState>,
    username: String,
) -> Result<crate::skins::SkinItem> {
    crate::skins::fetch_mojang_skin(&state.client, &state.paths, &username).await
}

/// 第三方皮肤站查询（LittleSkin / Blessing Skin 的 Yggdrasil 端点）。
#[tauri::command]
pub async fn skin_lookup_thirdparty(
    state: State<'_, AppState>,
    server: String,
    name: String,
) -> Result<crate::skins::ThirdPartySkin> {
    crate::skins::lookup_thirdparty(&state.client, &server, &name).await
}

/// 把皮肤绝对 URL 下载进 .mc/skins（不自动应用）。
#[tauri::command]
pub async fn skin_import_from_url(
    state: State<'_, AppState>,
    url: String,
    name: Option<String>,
) -> Result<crate::skins::SkinItem> {
    crate::skins::import_from_url(&state.client, &state.paths, &url, name.as_deref()).await
}

/// 账号上的披风列表：只有微软账号支持，其余返回 supported=false + 中文原因。
#[tauri::command]
pub async fn capes_list(
    state: State<'_, AppState>,
) -> Result<crate::capes::CapesListResult> {
    let settings = state.settings();
    crate::capes::capes_list(&state.client, &state.paths, &settings).await
}

/// 装备披风；capeId=null 表示不使用披风。
#[tauri::command]
pub async fn cape_set_active(
    state: State<'_, AppState>,
    cape_id: Option<String>,
) -> Result<crate::capes::CapeState> {
    let settings = state.settings();
    crate::capes::cape_set_active(&state.client, &state.paths, &settings, cape_id).await
}

/// 上传皮肤到 Mojang 账号（顺带把披风选回来）。
#[tauri::command]
pub async fn ms_skin_upload(
    state: State<'_, AppState>,
    png_path: Option<String>,
    png_base64: Option<String>,
    variant: String,
) -> Result<crate::capes::MsSkinUploadResult> {
    let settings = state.settings();
    crate::capes::skin_upload(
        &state.client,
        &state.paths,
        &settings,
        png_path.as_deref(),
        png_base64.as_deref(),
        &variant,
    )
    .await
}

/// ms_profile：当前微软账号的真实资料（名字 / uuid / 皮肤 / 披风 / 模型）。
#[tauri::command]
pub async fn ms_profile(
    state: State<'_, AppState>,
) -> Result<crate::capes::AccountProfile> {
    let settings = state.settings();
    crate::capes::ms_profile(&state.client, &state.paths, &settings).await
}

#[tauri::command]
pub fn cape_clear(state: State<'_, AppState>) -> Result<()> {
    crate::skins::clear_cape(&state.paths)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MojangSkin {
    pub skin_url: Option<String>,
    pub cape_url: Option<String>,
}

/// 按 uuid 查正版玩家皮肤/披风地址（皮肤站的 session 端点复用同一套解析）。
#[tauri::command]
pub async fn skin_profile_mojang(state: State<'_, AppState>, uuid: String) -> Result<MojangSkin> {
    let clean: String = uuid.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() != 32 {
        return Err(AppError::Invalid(format!("uuid 不合法：{uuid}")));
    }
    let url = format!(
        "https://sessionserver.mojang.com/session/minecraft/profile/{clean}"
    );
    let text = content::http_text(&state.client, &url, None).await?;
    let (skin, cape) = crate::skins::parse_mojang_profile(&text)?;
    Ok(MojangSkin {
        skin_url: skin,
        cape_url: cape,
    })
}

// ------------------------------------------------------------------ 账号

#[tauri::command]
pub fn accounts_list(state: State<'_, AppState>) -> Vec<crate::accounts::AccountItem> {
    crate::accounts::list(&state.paths)
}

#[tauri::command]
pub fn account_add_offline(
    state: State<'_, AppState>,
    name: String,
) -> Result<crate::accounts::AccountItem> {
    crate::accounts::add_offline(&state.paths, &name)
}

#[tauri::command]
pub fn account_apply(state: State<'_, AppState>, id: i64) -> Result<()> {
    crate::accounts::apply(&state.paths, id)
}

#[tauri::command]
pub fn account_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    crate::accounts::delete(&state.paths, id)
}

#[tauri::command]
pub async fn ms_login_start(
    state: State<'_, AppState>,
) -> Result<crate::ms_login::DeviceCodeStart> {
    let settings = state.settings();
    crate::ms_login::start_device_code(&state.client, &settings).await
}

#[tauri::command]
pub async fn ms_login_poll(
    state: State<'_, AppState>,
    device_code: String,
) -> Result<crate::ms_login::LoginPoll> {
    let settings = state.settings();
    crate::ms_login::poll_once(&state.client, &state.paths, &settings, &device_code).await
}

// ------------------------------------------------------------------ 内容服务

/// 内容卡片的译文：每页只翻前 20 条（命中缓存不重翻，内置通道自己串行化 = 并发 1）。
async fn translate_content_items(state: &AppState, items: &mut [content::ContentItem]) {
    let limit = items.len().min(content::TRANSLATE_PAGE_LIMIT);
    if limit == 0 {
        return;
    }
    let settings = state.settings();
    // 关掉翻译开关就完全不打翻译接口：标题保持原文、translated 恒为 false
    if !settings.translate_enabled {
        content::disable_translation(items);
        return;
    }
    let mut texts: Vec<String> = Vec::with_capacity(limit * 2);
    for item in items.iter().take(limit) {
        texts.push(item.title_original.clone());
        texts.push(item.summary_original.clone());
    }
    let cache_file = state.paths.cache.join(crate::translate::CACHE_FILE);
    let batch = state
        .translator
        .translate(
            &state.client,
            &cache_file,
            &settings.deepl_api_key,
            &texts,
            "zh-CN",
        )
        .await;
    for (index, item) in items.iter_mut().take(limit).enumerate() {
        let mut translated = false;
        if let Some(title) = batch.results.get(index * 2).and_then(|value| value.clone()) {
            if !title.trim().is_empty() {
                item.title = title;
                translated = true;
            }
        }
        if let Some(summary) = batch.results.get(index * 2 + 1).and_then(|value| value.clone()) {
            if !summary.trim().is_empty() {
                item.summary = summary;
                translated = true;
            }
        }
        item.translated = translated;
    }
    let requests = state.translator.request_count().await;
    let note = format!(
        "内容卡片翻译完成：items={limit} requests={requests} limited={}",
        batch.limited
    );
    tracing::debug!("{note}");
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn content_search(
    state: State<'_, AppState>,
    provider: String,
    kind: String,
    query: String,
    mc_version: Option<String>,
    loader: Option<String>,
    categories: Option<Vec<String>>,
    sort: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<content::SearchPage> {
    let provider = content::Provider::from_id(&provider)?;
    let page = page.unwrap_or(1).max(1);
    let page_size = content::clamp_page_size(page_size);
    if !content::kind_supported(provider, &kind) {
        let note = format!("{provider:?}/{kind} 不支持，返回空页");
        tracing::info!("{note}");
        return Ok(content::SearchPage {
            items: Vec::new(),
            total: 0,
            page,
            page_size,
        });
    }
    let offset = content::page_offset(page, page_size);
    let key = state.settings().curseforge_api_key;
    let has_key = !key.trim().is_empty();
    if provider == content::Provider::Curseforge && !has_key {
        tracing::info!("CurseForge 无 Key，自动走 MCIM 镜像");
    }
    let api_key = if provider == content::Provider::Curseforge && has_key {
        Some(key.as_str())
    } else {
        None
    };
    let categories = categories.unwrap_or_default();
    // CurseForge 只认分类 id：名字→id 走 /v1/categories 的 24h 缓存
    let category_ids = if provider == content::Provider::Curseforge {
        content::resolve_category_ids(&state.client, provider, api_key, &categories).await?
    } else {
        Vec::new()
    };
    let url = content::search_url_for(&content::SearchQuery {
        provider,
        kind: &kind,
        query: &query,
        mc_version: mc_version.as_deref(),
        loader: loader.as_deref(),
        categories: &categories,
        sort: sort.as_deref(),
        category_ids: &category_ids,
        offset,
        limit: page_size,
    }, has_key)?;
    let text = content::http_text(&state.client, &url, api_key).await?;
    let mut result = content::parse_search(provider, &text, &kind, page, page_size)?;
    translate_content_items(&state, &mut result.items).await;
    Ok(result)
}

#[tauri::command]
pub async fn content_detail(
    state: State<'_, AppState>,
    provider: String,
    project_id: String,
) -> Result<content::ProjectDetail> {
    let provider = content::Provider::from_id(&provider)?;
    let key = state.settings().curseforge_api_key;
    let has_key = !key.trim().is_empty();
    if provider == content::Provider::Curseforge && !has_key {
        tracing::info!("CurseForge 无 Key，自动走 MCIM 镜像");
    }
    let url = content::detail_url_for(provider, has_key, &project_id)?;
    let text = content::http_text(
        &state.client,
        &url,
        if provider == content::Provider::Curseforge && has_key {
            Some(key.as_str())
        } else {
            None
        },
    )
    .await?;
    content::parse_detail(provider, &text)
}

#[tauri::command]
pub async fn content_versions(
    state: State<'_, AppState>,
    provider: String,
    project_id: String,
    mc_version: Option<String>,
    loader: Option<String>,
) -> Result<Vec<content::VersionItem>> {
    let provider = content::Provider::from_id(&provider)?;
    let key = state.settings().curseforge_api_key;
    let has_key = !key.trim().is_empty();
    if provider == content::Provider::Curseforge && !has_key {
        tracing::info!("CurseForge 无 Key，自动走 MCIM 镜像");
    }
    let url = content::versions_url_for(
        provider,
        has_key,
        &project_id,
        mc_version.as_deref(),
        loader.as_deref(),
    )?;
    let text = content::http_text(
        &state.client,
        &url,
        if provider == content::Provider::Curseforge && has_key {
            Some(key.as_str())
        } else {
            None
        },
    )
    .await?;
    content::parse_versions(provider, &text)
}

/// 安装一个模组到实例：立刻返回 taskId，进度走 install://progress。
#[tauri::command]
pub async fn mod_install(
    state: State<'_, AppState>,
    provider: String,
    project_id: String,
    version_id: String,
    instance_id: i64,
) -> Result<GithubDownloadStarted> {
    let provider = content::Provider::from_id(&provider)?;
    let paths = state.paths.clone();
    let client = state.client.clone();
    let settings = state.settings();
    let target = {
        let conn = state.db()?;
        instance::get(&conn, instance_id)?
    };
    let title = format!("安装模组 {}", project_id);
    let via_mcim = settings.content_download_via_mcim;
    let task = state.tasks.create_with_prefix(
        "content",
        &title,
        &crate::mods::kind_dir(&paths, &target.name, crate::mods::ModKind::Mod)
            .display()
            .to_string(),
        "content",
    );
    let task_id = task.id().to_string();
    let task_bg = task.clone();
    tauri::async_runtime::spawn(async move {
        let key = settings.curseforge_api_key.clone();
        match content::install_mod(
            &client,
            &paths,
            &target.name,
            provider,
            &project_id,
            &version_id,
            if key.trim().is_empty() { None } else { Some(key.as_str()) },
            via_mcim,
            &task_bg,
        )
        .await
        {
            Ok(path) => {
                tracing::info!(path = %path.display(), "模组安装完成");
                task_bg.finish();
            }
            Err(AppError::Cancelled) => task_bg.cancel(),
            Err(err) => {
                tracing::error!(%err, "模组安装失败");
                task_bg.fail(&err.to_string());
            }
        }
    });
    Ok(GithubDownloadStarted { task_id })
}

/// modpack_install 的返回：整合包直接建实例，taskId 前缀 modpack。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackInstallStarted {
    pub task_id: String,
    pub instance_id: i64,
}

/// 安装整合包（Modrinth .mrpack / CurseForge zip）：直接创建一个新实例并装进去。
///
/// 这条命令等到装完才返回（前端靠失败时的错误文案判断是否落到 incomplete）；
/// 进度仍然实时走 install://progress，taskId 前缀 modpack。
#[tauri::command]
pub async fn modpack_install(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: String,
    project_id: String,
    version_id: Option<String>,
    instance_name: Option<String>,
    memory_mb: Option<u32>,
) -> Result<ModpackInstallStarted> {
    let provider = content::Provider::from_id(&provider)?;
    let settings = state.settings();
    let paths = state.paths.clone();
    let origins = sources::resolve(&settings);
    let label = modpack::sanitize_instance_name(
        instance_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(project_id.trim()),
    );
    let task = state.tasks.create_with_prefix(
        modpack::TASK_KIND,
        &format!("安装整合包 {label}"),
        &paths.cache.join("packs").display().to_string(),
        modpack::TASK_PREFIX,
    );
    let context = modpack::PackContext {
        client: state.client.clone(),
        paths: paths.clone(),
        origins,
        connections: settings.effective_connections(),
        concurrency: settings.effective_concurrency(),
        curseforge_api_key: settings.curseforge_api_key.clone(),
        via_mcim: settings.content_download_via_mcim,
    };
    let request = modpack::PackRequest {
        provider,
        project_id: project_id.trim().to_string(),
        version_id: version_id.unwrap_or_default(),
        instance_name: instance_name.unwrap_or_default(),
        memory_mb: modpack::normalize_memory(memory_mb),
    };
    let sink = modpack::DbInstanceSink::new(&paths);

    let result = modpack::install(&context, &request, &sink, &task).await;
    // 成功失败都发一次：失败时前端要靠这条事件把「整合包未装完」小标刷出来
    {
        use tauri::Emitter;
        if let Err(err) = app.emit("instances://changed", ()) {
            tracing::warn!(%err, "发送 instances://changed 失败");
        }
    }
    match result {
        Ok(outcome) => {
            tracing::info!("{}", outcome.summary());
            task.finish();
            Ok(ModpackInstallStarted {
                task_id: task.id().to_string(),
                instance_id: outcome.instance_id,
            })
        }
        Err(AppError::Cancelled) => {
            task.cancel();
            Err(AppError::Cancelled)
        }
        Err(err) => {
            tracing::error!(%err, "整合包安装失败");
            task.fail(&err.to_string());
            Err(err)
        }
    }
}

/// 批量翻译（内容服务展示用）。limited=true 表示内置接口被限流。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateTextsResult {
    pub results: Vec<Option<String>>,
    pub limited: bool,
}

#[tauri::command]
pub async fn translate_texts(
    state: State<'_, AppState>,
    texts: Vec<String>,
    to_lang: Option<String>,
) -> Result<TranslateTextsResult> {
    let settings = state.settings();
    let target = to_lang.unwrap_or_else(|| "zh-CN".to_string());
    if settings.translation_source == "custom" {
        let results = crate::translate::translate_custom(&state.client, &settings, &texts).await;
        let limited = !texts.is_empty() && results.iter().all(|item| item.is_none());
        return Ok(TranslateTextsResult { results, limited });
    }
    let cache_file = state.paths.cache.join(crate::translate::CACHE_FILE);
    let outcome = state
        .translator
        .translate(
            &state.client,
            &cache_file,
            &settings.deepl_api_key,
            &texts,
            &target,
        )
        .await;
    Ok(TranslateTextsResult {
        results: outcome.results,
        limited: outcome.limited,
    })
}

// ------------------------------------------------------------------ 外观背景（M4）

/// 背景列表（分页）：kind=local 扫 .mc/backgrounds，kind=wallpaper 扫壁纸引擎 431960 目录。
///
/// 扫描全程在 spawn_blocking 里，命令本身不持任何锁；同一 kind 的并发请求合并成一次扫描。
#[tauri::command]
pub async fn background_list(
    state: State<'_, AppState>,
    kind: String,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<crate::appearance::BackgroundPage> {
    let kind = crate::appearance::normalize_kind(&kind)?;
    let page = page.unwrap_or(1).max(1);
    let resolved_page_size = crate::appearance::clamp_page_size(page_size);
    let dir = if kind == "local" {
        crate::appearance::backgrounds_dir(&state.paths)
    } else {
        let settings = state.settings();
        if settings.appearance.wallpaper_dir.trim().is_empty() {
            // 探测（注册表 + libraryfolders.vdf）也放后台线程，别堵主线程
            match tokio::task::spawn_blocking(crate::appearance::wallpaper_dir_default).await {
                Ok(Some(dir)) => PathBuf::from(dir),
                _ => {
                    tracing::warn!("没有探测到壁纸引擎目录，返回空列表");
                    return Ok(crate::appearance::BackgroundPage::empty(
                        page,
                        resolved_page_size,
                    ));
                }
            }
        } else {
            PathBuf::from(settings.appearance.wallpaper_dir.trim())
        }
    };
    crate::appearance::list_page(kind, dir, Some(page), Some(resolved_page_size)).await
}

/// 选中并立即生效：只改对应方案的文件，并把 scheme 切过去（四方案互斥）。
#[tauri::command]
pub fn background_apply(
    state: State<'_, AppState>,
    kind: String,
    file: String,
) -> Result<crate::appearance::Appearance> {
    let mut settings = state.settings();
    let kind = crate::appearance::normalize_kind(&kind)?;
    match kind {
        "local" => {
            settings.appearance.scheme = "local".to_string();
            settings.appearance.local_file = file;
        }
        _ => {
            settings.appearance.scheme = "wallpaper".to_string();
            settings.appearance.wallpaper_file = file;
        }
    }
    let saved = state.set_settings(settings)?;
    Ok(saved.appearance)
}

/// 自动探测壁纸引擎目录（Steam 注册表 + libraryfolders.vdf）。
#[tauri::command]
pub fn wallpaper_dir_default() -> Option<String> {
    crate::appearance::wallpaper_dir_default()
}

// ------------------------------------------------------------------ 陶瓦联机（M4）

#[tauri::command]
pub fn tera_status(state: State<'_, AppState>) -> crate::terracotta::TeraStatus {
    state.tera.status(&state.paths)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeraInstalled {
    pub version: String,
    /// bundled（安装包内嵌）/ downloaded（联网下载）
    pub source: String,
    pub license: String,
    pub homepage: String,
}

/// 准备陶瓦联机：优先部署安装包里内嵌的二进制，内嵌缺失才下载官方包。
#[tauri::command]
pub async fn tera_install(state: State<'_, AppState>) -> Result<TeraInstalled> {
    let paths = state.paths.clone();
    let client = state.client.clone();
    let task = state.tasks.create_with_prefix(
        "tera",
        "陶瓦联机",
        &crate::terracotta::terracotta_dir(&paths).display().to_string(),
        "tera",
    );
    let result = state.tera.ensure(&client, &paths, &task).await;
    match result {
        Ok(ready) => {
            task.finish();
            tracing::info!(
                source = %ready.source,
                version = %ready.version,
                "陶瓦联机已就绪"
            );
            Ok(TeraInstalled {
                version: ready.version,
                source: ready.source,
                license: crate::terracotta::TERRACOTTA_LICENSE.to_string(),
                homepage: crate::terracotta::TERRACOTTA_HOMEPAGE.to_string(),
            })
        }
        Err(AppError::Cancelled) => {
            task.cancel();
            Err(AppError::Cancelled)
        }
        Err(err) => {
            task.fail(&err.to_string());
            Err(err)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeraRoom {
    pub room_code: String,
}

/// 主机模式：port 传了就是专用服务器模式（启动器代发局域网广播）。
#[tauri::command]
pub async fn tera_start_host(
    state: State<'_, AppState>,
    room_name: Option<String>,
    port: Option<u16>,
) -> Result<TeraRoom> {
    let player = room_name.unwrap_or_default();
    let status = state
        .tera
        .host(&state.client, &state.paths, &player, port)
        .await?;
    Ok(TeraRoom {
        room_code: status.room_code,
    })
}

#[tauri::command]
pub async fn tera_join(
    state: State<'_, AppState>,
    code: String,
    player: Option<String>,
) -> Result<TeraRoom> {
    let status = state
        .tera
        .join(
            &state.client,
            &state.paths,
            &code,
            player.as_deref().unwrap_or(""),
        )
        .await?;
    Ok(TeraRoom {
        room_code: status.room_code,
    })
}

#[tauri::command]
pub async fn tera_stop(state: State<'_, AppState>) -> Result<()> {
    state.tera.stop(&state.client, &state.paths).await
}

/// 复位回 waiting（异常态的唯一出口）。
#[tauri::command]
pub async fn tera_reset(state: State<'_, AppState>) -> Result<crate::terracotta::TeraStatus> {
    state.tera.reset(&state.client, &state.paths).await
}

// ------------------------------------------------------------------ 其它

/// 系统里能显示中文的字体（按常见 UI 字体优先排序）。
#[tauri::command]
pub fn fonts_list() -> Vec<FontChoice> {
    let list = fonts::list_system_fonts();
    if list.is_empty() {
        tracing::warn!("系统字体枚举为空，界面将使用默认字体");
    }
    list
}

/// 微软登录是否可用：client id 只在打包时注入，没有注入时前端要把登录按钮置灰。
#[tauri::command]
pub fn ms_auth_status(state: State<'_, AppState>) -> MsAuthStatus {
    ms_auth::status(&state.settings())
}

/// 主页新闻：官方 news.json（失败兜底站点网格），默认 5 条、最多 20 条。
#[tauri::command]
pub async fn news_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
    force: Option<bool>,
) -> Result<crate::news::NewsList> {
    let settings = state.settings();
    Ok(
        crate::news::list(
            &state.client,
            &state.paths,
            &state.news,
            Some((&settings, &state.translator)),
            limit.unwrap_or(crate::news::DEFAULT_LIMIT),
            force.unwrap_or(false),
        )
        .await,
    )
}

/// 首页新闻重试：强制绕过缓存与负缓存（前端「重试」按钮用）。
#[tauri::command]
pub async fn news_refresh(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<crate::news::NewsList> {
    let settings = state.settings();
    Ok(
        crate::news::list(
            &state.client,
            &state.paths,
            &state.news,
            Some((&settings, &state.translator)),
            limit.unwrap_or(crate::news::DEFAULT_LIMIT),
            true,
        )
        .await,
    )
}

#[tauri::command]
pub fn update_status(state: State<'_, AppState>) -> UpdateStatus {
    state
        .update
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| {
            UpdateStatus::new(env!("CARGO_PKG_VERSION"), check::Phase::Idle, "状态不可用")
        })
}

#[tauri::command]
pub async fn update_check(state: State<'_, AppState>) -> Result<UpdateStatus> {
    let settings = state.settings();
    Ok(
        check::run(
            &state.client,
            &state.probes,
            &state.update,
            &settings.update_mode,
            env!("CARGO_PKG_VERSION"),
            build_time(),
        )
        .await,
    )
}

/// manual 模式下用户点「立即更新」：重新检查 → 取安装包（静默安装仍留待后续）。
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, AppState>) -> Result<UpdateStatus> {
    let settings = state.settings();
    let current = env!("CARGO_PKG_VERSION");
    if !check::should_request(&settings.update_mode) {
        return Ok(UpdateStatus::new(
            current,
            check::Phase::Idle,
            "已关闭更新检查",
        ));
    }

    // 1) 检查：一次拿到状态与要装的安装包（没有可用更新就什么都不下载）
    let (status, asset) = check::run_with_asset(
        &state.client,
        &state.probes,
        &state.update,
        &settings.update_mode,
        current,
        build_time(),
    )
    .await;
    let Some(asset) = asset.filter(|_| status.can_install) else {
        return Ok(status);
    };
    let remote = status
        .remote_version
        .clone()
        .unwrap_or_else(|| "新版本".to_string());

    // 2) 下载 → 校验 → 写脚本（进度走 install://progress，taskId 前缀 update-）
    let paths = state.paths.clone();
    let client = state.client.clone();
    let task = state
        .tasks
        .create_with_prefix("update", &format!("更新到 {remote}"), "", "update");
    let set_phase = |phase: check::Phase, message: String| {
        let mut next = status.clone();
        next.phase = phase;
        next.message = message;
        if let Ok(mut guard) = state.update.lock() {
            *guard = next.clone();
        }
        next
    };
    set_phase(
        check::Phase::Downloading,
        format!("正在下载 {remote}（{}）", asset.name),
    );

    let launcher = std::env::current_exe()
        .map_err(|err| AppError::Other(format!("取启动器路径失败：{err}")))?;
    let prepared = match crate::update::installer::prepare(
        &client,
        &paths,
        &launcher,
        &asset.browser_download_url,
        asset.size,
        &task,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(AppError::Cancelled) => {
            task.cancel();
            return Err(AppError::Cancelled);
        }
        Err(err) => {
            let message = err.to_string();
            task.fail(&message);
            let failed = set_phase(check::Phase::Failed, message.clone());
            tracing::error!(%err, "更新包下载失败");
            return Err(AppError::Other(format!(
                "{}（失败原因：{message}，半成品保留在 {}）",
                failed.message,
                crate::update::installer::update_dir(&paths).display()
            )));
        }
    };
    tracing::info!(
        setup = %prepared.setup.display(),
        size = prepared.size,
        sha256 = %prepared.sha256,
        script = %prepared.script.display(),
        "更新包已校验，准备静默安装"
    );

    // 3) detached 启动脚本 → 主进程退出，脚本等我们退出后 /S 安装并重启
    set_phase(
        check::Phase::Installing,
        format!("正在安装 {remote}（静默安装，随后自动重启）"),
    );
    if let Err(err) = crate::update::installer::launch_script(&prepared.script) {
        let message = err.to_string();
        task.fail(&message);
        set_phase(check::Phase::Failed, message.clone());
        return Err(err);
    }
    task.finish();
    tracing::info!("更新脚本已启动，主进程即将退出");
    // 给前端 200ms 收尾（把最终状态推出去），然后退出让脚本继续
    let final_status = set_phase(
        check::Phase::Installing,
        format!("{remote} 安装程序已启动，启动器即将重启"),
    );
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        app.exit(0);
    });
    Ok(final_status)
}

/// 构建时间（build.rs 写入），用于「版本号解析不出来时按发布时间兜底」。
fn build_time() -> Option<i64> {
    option_env!("GUGU_BUILD_TIME").and_then(|value| value.parse::<i64>().ok())
}

// download 模块的 DownloadSource 在 java_download 里用到，这里显式留一个引用，
// 避免 unused import 警告（同时说明内置源仍然走同一套映射）。
const _: fn(&str) -> Option<DownloadSource> = DownloadSource::from_id;
const _: fn() -> Vec<download::DownloadSource> = || download::DownloadSource::ALL.to_vec();


// ------------------------------------------------------------------ 种子地图

/// seedmap_versions：种子地图支持的 MC 版本（前端只显示这些）。
#[tauri::command]
pub fn seedmap_versions() -> Vec<crate::seedmap::VersionInfo> {
    crate::seedmap::versions()
}

/// seedmap_biomes：算一块区域（tiles_x × tiles_y 像素）的群系图。
/// rgba 是 PNG 的 base64，biomes 是逐像素群系 id（[z * width + x]）。
#[tauri::command]
pub async fn seedmap_biomes(
    mc: String,
    dimension: i32,
    seed: crate::seedmap::SeedArg,
    scale: i32,
    x0: i32,
    z0: i32,
    tiles_x: u32,
    tiles_y: u32,
) -> Result<crate::seedmap::BiomeMapView> {
    let request = crate::seedmap::BiomeRequest {
        mc: crate::seedmap::resolve_version(&mc)?,
        dim: crate::seedmap::dimension_to_cubiomes(dimension)?,
        seed: seed.to_seed()?,
        scale,
        x0,
        z0,
        tiles_x,
        tiles_y,
    };
    crate::seedmap::biome_map(request).await
}

/// seedmap_biome_at：单个方块坐标的群系（含中文名）。
#[tauri::command]
pub async fn seedmap_biome_at(
    mc: String,
    dimension: i32,
    seed: crate::seedmap::SeedArg,
    x: i32,
    z: i32,
) -> Result<crate::seedmap::BiomeAtView> {
    let mc = crate::seedmap::resolve_version(&mc)?;
    let dim = crate::seedmap::dimension_to_cubiomes(dimension)?;
    crate::seedmap::biome_at(mc, dim, seed.to_seed()?, x, z).await
}

/// seedmap_spawn：主世界出生点。
#[tauri::command]
pub async fn seedmap_spawn(
    mc: String,
    seed: crate::seedmap::SeedArg,
) -> Result<crate::seedmap::SpawnView> {
    let mc = crate::seedmap::resolve_version(&mc)?;
    crate::seedmap::spawn_point(mc, seed.to_seed()?).await
}

/// seedmap_structures：搜索 (x,z) 半径 radius 内的结构，按距离排序。
/// structure：village|temple|monument|mansion|stronghold|fortress|bastion|
/// outpost|ruined_portal|shipwreck|ocean_ruin|treasure
#[tauri::command]
pub async fn seedmap_structures(
    mc: String,
    seed: crate::seedmap::SeedArg,
    structure: String,
    x: Option<i32>,
    z: Option<i32>,
    radius: i32,
) -> Result<Vec<crate::seedmap::StructureHit>> {
    let mc = crate::seedmap::resolve_version(&mc)?;
    let (x, z) = (x.unwrap_or(0), z.unwrap_or(0));
    crate::seedmap::structures(mc, seed.to_seed()?, structure, x, z, radius).await
}

