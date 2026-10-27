//! 咕咕启动器 · Gugu Launcher — M2-A：能真正下载并启动游戏。
//!
//! 基础层：paths/settings/db/meta/download/instance/fonts
//! 下载与启动内核：version/install/downloader/java/launch/tasks/events/sources/shell
//! 只做纯函数与探针：update/net。界面只提供简体中文。

pub mod accounts;
pub mod appearance;
pub mod capes;
pub mod changelog;
pub mod contentsource;
pub mod commands;
pub mod content;
pub mod datadir;
pub mod db;
pub mod download;
pub mod downloader;
pub mod error;
pub mod events;
pub mod fonts;
pub mod github_dl;
pub mod install;
pub mod instance;
pub mod java;
pub mod launch;
pub mod meta;
pub mod modpack;
pub mod mods;
pub mod ms_auth;
pub mod ms_login;
pub mod net;
pub mod news;
pub mod paths;
pub mod png;
pub mod saying;
pub mod seedmap;
pub mod server;
pub mod server_content;
pub mod settings;
pub mod shell;
pub mod single_instance;
pub mod skins;
pub mod sources;
pub mod state;
pub mod storage;
pub mod sysinfo;
pub mod tasks;
pub mod terracotta;
pub mod translate;
pub mod tracing_file;
pub mod update;
pub mod updates;
pub mod util;
pub mod version;
pub mod yggdrasil;

use tauri::Manager;

/// 日志同时写 stdout 与 .mc\logs\launcher-<date>.log。
fn init_tracing(paths: &paths::Paths) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "gugu_launcher_lib=info,warn".to_string());
    let env_filter = tracing_subscriber::EnvFilter::try_new(&filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("gugu_launcher_lib=info,warn"));
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_filter(env_filter.clone());

    let log_path = paths
        .logs
        .join(format!("launcher-{}.log", util::date_stamp(util::now_unix())));
    match tracing_file::FileWriter::open(&log_path) {
        Ok(writer) => {
            let file_layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_writer(writer)
                .with_filter(env_filter);
            let _ = tracing_subscriber::registry()
                .with(stdout_layer)
                .with(file_layer)
                .try_init();
        }
        Err(err) => {
            let _ = tracing_subscriber::registry().with(stdout_layer).try_init();
            eprintln!("启动器日志文件创建失败（{}）：{err}", log_path.display());
        }
    }
}

/// 后台更新检查：启动后延迟 STARTUP_DELAY 查一次，之后每 CHECK_INTERVAL 一次。
/// 全程不弹窗、不发通知；结果只写进 AppState.update，由「关于」页读取。
fn spawn_update_checker(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(update::check::STARTUP_DELAY).await;
        loop {
            {
                let state = handle.state::<state::AppState>();
                let mode = state.settings().update_mode;
                if update::check::should_request(&mode) {
                    let build_time =
                        option_env!("GUGU_BUILD_TIME").and_then(|value| value.parse::<i64>().ok());
                    let _ = update::check::run(
                        &state.client,
                        &state.probes,
                        &state.update,
                        &mode,
                        env!("CARGO_PKG_VERSION"),
                        build_time,
                    )
                    .await;
                }
            }
            tokio::time::sleep(update::check::CHECK_INTERVAL).await;
        }
    });
}

/// 显示并聚焦主窗口（托盘菜单、二次启动都走这里）。
///
/// Windows 上光调 set_focus 有时抢不到前台，这里用 always_on_top 闪一下再置回。
pub fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = window.set_always_on_top(true);
        let _ = window.set_always_on_top(false);
        let _ = window.set_focus();
    }
}

/// 创建系统托盘：左键唤起窗口，右键出菜单（显示 / 打开 .mc / 退出）。
fn create_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let open_mc = MenuItem::with_id(app, "open_mc", "打开 .mc 目录", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = MenuBuilder::new(app)
        .item(&show)
        .item(&open_mc)
        .separator()
        .item(&quit)
        .build()?;

    // 图标来源：优先用 bundle 里注册的窗口图标（icons/icon.ico 的新 logo，多分辨率），
    // 高 DPI 下若发现发虚，可改用 include_bytes! 的 icons/32x32.png。
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_string()))?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("咕咕启动器")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "open_mc" => {
                let state = app.state::<state::AppState>();
                if let Err(err) = shell::open_folder(&state.paths.root) {
                    tracing::warn!(%err, "托盘打开 .mc 目录失败");
                }
            }
            "quit" => {
                tracing::info!("托盘菜单退出");
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    tracing::info!("系统托盘已创建");
    Ok(())
}

/// Tauri 入口：初始化数据根 → 注册命令 → 跑窗口。
pub fn run() {
    // 单实例守门：命名互斥体，第二个进程在任何窗口/托盘/状态初始化之前就唤醒主实例并退出
    single_instance::exit_if_duplicate();

    // 先读设置拿到数据根（settings.json 永远在安装目录的 .mc 下），再决定目录布局
    let paths = state::AppState::bootstrap_paths();
    if let Err(err) = paths.ensure() {
        eprintln!("咕咕启动器初始化失败：{err}");
        std::process::exit(1);
    }
    init_tracing(&paths);
    tracing::info!(root = %paths.root.display(), "数据根就绪");

    let state = match state::AppState::bootstrap(paths) {
        Ok(state) => state,
        Err(err) => {
            tracing::error!(%err, "初始化失败");
            eprintln!("咕咕启动器初始化失败：{err}");
            std::process::exit(1);
        }
    };

    let builder = tauri::Builder::default()
        // 单实例必须是第一个注册的插件：第二次启动由它拦下并回调
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            tracing::info!(?argv, %cwd, "second instance argv 已收到，切回主窗口");
            show_main_window(app);
            {
                use tauri::Emitter;
                let _ = app.emit("app://second-instance", ());
            }
        }))
        .setup(|app| {
            let handle = app.handle().clone();
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("咕咕启动器 · Gugu Launcher");
                // 关闭按钮 → 三态：ask（原生三选对话框）/ minimize（隐藏到托盘）/ exit（直接退出）
                let handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let action = handle
                            .try_state::<state::AppState>()
                            .map(|state| state.settings().close_action)
                            .unwrap_or_else(|| settings::DEFAULT_CLOSE_ACTION.to_string());
                        match action.as_str() {
                            "exit" => {
                                api.prevent_close();
                                tracing::info!(close_action = "exit", "关闭窗口：按设置直接退出程序");
                                handle.exit(0);
                            }
                            "ask" => {
                                api.prevent_close();
                                match shell::ask_close_action() {
                                    Ok((choice, remember)) => {
                                        tracing::info!(
                                            close_action = "ask",
                                            choice = choice.as_str(),
                                            remember,
                                            "关闭窗口：三选对话框结果"
                                        );
                                        if remember {
                                            if let Some(managed) = handle.try_state::<state::AppState>() {
                                                let mut next = managed.settings();
                                                if let Some(value) = choice.as_setting() {
                                                    next.close_action = value.to_string();
                                                    match managed.set_settings(next) {
                                                        Ok(saved) => tracing::info!(
                                                            close_action = %saved.close_action,
                                                            "已记住关闭窗口的行为"
                                                        ),
                                                        Err(err) => tracing::warn!(%err, "记住关闭行为失败"),
                                                    }
                                                }
                                            }
                                        }
                                        match choice {
                                            shell::CloseChoice::Minimize => {
                                                if let Some(window) = handle.get_webview_window("main") {
                                                    let _ = window.hide();
                                                }
                                                tracing::info!("窗口已隐藏到托盘（用户选择最小化到后台）");
                                            }
                                            shell::CloseChoice::Exit => {
                                                tracing::info!("关闭窗口：用户选择退出程序");
                                                handle.exit(0);
                                            }
                                            shell::CloseChoice::Cancel => {
                                                tracing::info!("关闭窗口：用户选择取消，继续留在主界面");
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        tracing::warn!(%err, "关闭确认框打不开，按最小化处理");
                                        if let Some(window) = handle.get_webview_window("main") {
                                            let _ = window.hide();
                                        }
                                    }
                                }
                            }
                            other => {
                                api.prevent_close();
                                if let Some(window) = handle.get_webview_window("main") {
                                    let _ = window.hide();
                                }
                                tracing::info!(close_action = other, "窗口已隐藏到托盘");
                            }
                        }
                    }
                });
            }
            // 任务表接上事件总线：安装/下载进度从这里推给前端
            {
                let managed = app.state::<state::AppState>();
                let bus: std::sync::Arc<dyn events::Events> =
                    std::sync::Arc::new(events::TauriEvents::new(handle.clone()));
                managed.tasks.bind_events(bus.clone());
                managed.tera.bind_events(bus);
            }
            create_tray(app)?;
            // 主实例：等第二个实例发唤醒信号（插件只做 argv 转发，进程数由互斥体守门）
            single_instance::watch_primary(handle.clone());
            spawn_update_checker(handle);
            Ok(())
        })
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::instance_ensure_dependencies,
            commands::resource_sources,
            commands::resource_probe,
            commands::system_info,
            commands::instance_set_icon,
            commands::instance_icon_upload,
            commands::instance_icon_remove,
            commands::app_changelog,
            commands::paths_init,
            commands::settings_get,
            commands::settings_set,
            commands::versions_list,
            commands::loader_versions,
            commands::loaders_list,
            commands::instances_list,
            commands::instance_create,
            commands::instance_delete,
            commands::instance_detail,
            commands::instance_update,
            commands::instance_settings_get,
            commands::instance_settings_set,
            commands::instance_open_folder,
            commands::instance_pick_icon,
            commands::version_install,
            commands::instance_launch,
            commands::instance_launch_with_save,
            commands::downloads_list,
            commands::download_cancel,
            commands::download_open_folder,
            commands::download_sources,
            commands::sources_probe,
            commands::java_list,
            commands::java_sources,
            commands::java_probe_sources,
            commands::java_versions,
            commands::java_install,
            commands::java_download,
            commands::data_dir_get,
            commands::data_dir_pick,
            commands::data_dir_migrate,
            commands::data_dir_reset_default,
            commands::storage_stats,
            commands::storage_clean,
            commands::instances_recent,
            commands::server_cores,
            commands::server_versions,
            commands::server_create,
            commands::server_list,
            commands::server_start,
            commands::server_stop,
            commands::server_delete,
            commands::server_open_folder,
            commands::server_read_properties,
            commands::server_write_properties,
            commands::server_content_list,
            commands::server_content_install,
            commands::server_content_toggle,
            commands::server_content_check,
            commands::server_content_apply_updates,
            commands::server_saves_list,
            commands::server_saves_backup,
            commands::server_config_read,
            commands::server_config_write,
            commands::server_logs_tail,
            commands::server_send_command,
            commands::server_console_state,
            commands::logs_list,
            commands::log_read,
            commands::log_open_folder,
            commands::open_path,
            commands::clipboard_write,
            commands::fonts_list,
            commands::fun_saying,
            commands::translation_test,
            commands::github_download,
            commands::github_cancel,
            commands::github_open_folder,
            commands::pick_directory,
            commands::mods_list,
            commands::mods_set_enabled,
            commands::mods_delete,
            commands::mods_import,
            commands::mods_open_folder,
            commands::mods_check_updates,
            commands::mods_apply_updates,
            commands::saves_open_folder,
            commands::skin_current,
            commands::saves_list,
            commands::saves_delete,
            commands::saves_export,
            commands::instance_export,
            commands::skins_list,
            commands::skins_delete,
            commands::skin_apply,
            commands::skin_upload,
            commands::skin_lookup_thirdparty,
            commands::skin_import_from_url,
            commands::capes_list,
            commands::cape_set_active,
            commands::ms_skin_upload,
            commands::ms_profile,
            commands::cape_clear,
            commands::skin_fetch_mojang,
            commands::skin_profile_mojang,
            commands::skin_default,
            commands::news_list,
            commands::news_refresh,
            commands::accounts_list,
            commands::account_add_offline,
            commands::account_apply,
            commands::account_delete,
            commands::account_add_thirdparty,
            commands::account_refresh_thirdparty,
            commands::authlib_status,
            commands::authlib_ensure,
            commands::thirdparty_skin,
            commands::ms_login_start,
            commands::ms_login_poll,
            commands::content_search,
            commands::content_detail,
            commands::content_versions,
            commands::mod_install,
            commands::modpack_install,
            commands::translate_texts,
            commands::background_list,
            commands::background_apply,
            commands::wallpaper_dir_default,
            commands::tera_status,
            commands::tera_install,
            commands::tera_start_host,
            commands::tera_join,
            commands::tera_stop,
            commands::tera_reset,
            commands::ms_auth_status,
            commands::update_status,
            commands::update_check,
            commands::update_install,
            commands::seedmap_versions,
            commands::seedmap_biomes,
            commands::seedmap_biome_at,
            commands::seedmap_spawn,
            commands::seedmap_structures,
        ]);

    if let Err(err) = builder.run(tauri::generate_context!()) {
        tracing::error!(%err, "启动器退出");
        eprintln!("咕咕启动器启动失败：{err}");
        std::process::exit(1);
    }
}
