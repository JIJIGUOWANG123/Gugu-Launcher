//! 启动内核：拼 classpath 与 JVM/游戏参数 → 起进程 → 逐行转发日志 → 退出回调。
//!
//! 参数拼装是纯函数（build_command），单测直接断言；进程相关部分只做 spawn + 读流。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use crate::error::{AppError, Result};
use crate::events::Events;
use crate::install::{self, client_jar_path};
use crate::paths::Paths;
use crate::util;
use crate::version::{self, MergedVersion, Platform};

/// 没有实例设置时的默认窗口尺寸。
pub const DEFAULT_WIDTH: u32 = 854;
pub const DEFAULT_HEIGHT: u32 = 480;
/// 默认离线用户名。
pub const DEFAULT_USERNAME: &str = "Player";
pub const LAUNCHER_NAME: &str = "gugu-launcher";
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 内存下限/上限（与 settings 的校验范围一致）。
pub const MEMORY_MIN_MB: u32 = 512;
pub const MEMORY_MAX_MB: u32 = 65536;

/// 启动一个实例所需的全部输入。
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub instance_id: i64,
    pub instance_name: String,
    /// 版本目录名（instances.mc_version）
    pub version_id: String,
    /// 实例的游戏目录（instances/<名字>/.minecraft）
    pub game_dir: PathBuf,
    pub java_path: PathBuf,
    pub memory_mb: u32,
    pub width: u32,
    pub height: u32,
    pub username: String,
    pub uuid: String,
    /// 微软账号的访问令牌；离线账号留空（启动参数里写 0）
    pub access_token: String,
}

impl LaunchOptions {
    pub fn clamped_memory(&self) -> u32 {
        self.memory_mb.clamp(MEMORY_MIN_MB, MEMORY_MAX_MB)
    }
}

/// 拼好的启动指令。
#[derive(Debug, Clone)]
pub struct LaunchCommand {
    pub java: PathBuf,
    pub main_class: String,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub classpath: String,
    pub natives: PathBuf,
    pub game_dir: PathBuf,
}

impl LaunchCommand {
    /// 供日志/排错用的完整命令行（Java 路径 + JVM 参数 + 主类 + 游戏参数）。
    pub fn to_display(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(self.java.display().to_string());
        parts.extend(self.jvm_args.iter().cloned());
        parts.push(self.main_class.clone());
        parts.extend(self.game_args.iter().cloned());
        parts.join(" ")
    }
}

/// 离线账号的 uuid：与 Java 的 nameUUIDFromBytes("OfflinePlayer:<name>") 一致。
pub fn offline_uuid(username: &str) -> String {
    crate::accounts::offline_uuid(username)
}

fn classpath_separator() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

/// 把 ${key} 占位全部替换；未知占位原样留着（宁可 JVM 报错也不要静默丢参数）。
fn substitute(input: &str, vars: &BTreeMap<&str, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match vars.get(key) {
                    Some(value) => out.push_str(value),
                    None => {
                        out.push_str("${");
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// 纯函数：按版本 json + 实例设置拼出启动指令。
pub fn build_command(
    paths: &Paths,
    merged: &MergedVersion,
    options: &LaunchOptions,
) -> Result<LaunchCommand> {
    let platform = Platform::current();
    let features = version::default_features();
    let libraries = version::resolve_libraries(&merged.libraries, &platform, &features)?;

    let natives = install::natives_dir(paths, &options.version_id);
    let client_jar = client_jar_path(paths, &options.version_id);

    // classpath：普通库（按 json 顺序）+ 客户端 jar
    let mut classpath_entries: Vec<String> = libraries
        .iter()
        .filter(|lib| lib.kind == version::LibKind::Jar)
        .map(|lib| {
            paths
                .libraries
                .join(lib.path.replace('/', &std::path::MAIN_SEPARATOR.to_string()))
                .display()
                .to_string()
        })
        .collect();
    classpath_entries.push(client_jar.display().to_string());
    let classpath = classpath_entries.join(classpath_separator());

    let username = if options.username.trim().is_empty() {
        DEFAULT_USERNAME.to_string()
    } else {
        options.username.trim().to_string()
    };
    let uuid = if options.uuid.trim().is_empty() {
        offline_uuid(&username)
    } else {
        options.uuid.trim().to_string()
    };
    let width = if options.width == 0 { DEFAULT_WIDTH } else { options.width };
    let height = if options.height == 0 { DEFAULT_HEIGHT } else { options.height };

    let mut vars: BTreeMap<&str, String> = BTreeMap::new();
    vars.insert("natives_directory", natives.display().to_string());
    vars.insert("launcher_name", LAUNCHER_NAME.to_string());
    vars.insert("launcher_version", LAUNCHER_VERSION.to_string());
    vars.insert("classpath", classpath.clone());
    vars.insert("classpath_separator", classpath_separator().to_string());
    vars.insert("library_directory", paths.libraries.display().to_string());
    vars.insert("game_directory", options.game_dir.display().to_string());
    vars.insert("assets_root", paths.assets.display().to_string());
    vars.insert("assets_index_name", merged.assets.clone());
    vars.insert("auth_player_name", username.clone());
    vars.insert("auth_uuid", uuid.clone());
    let online = !options.access_token.trim().is_empty() && options.access_token.trim() != "0";
    vars.insert(
        "auth_access_token",
        if online {
            options.access_token.trim().to_string()
        } else {
            "0".to_string()
        },
    );
    vars.insert("auth_session", "0".to_string());
    vars.insert("auth_xuid", "0".to_string());
    vars.insert("clientid", "0".to_string());
    vars.insert(
        "user_type",
        if online { "msa" } else { "legacy" }.to_string(),
    );
    vars.insert("user_properties", "{}".to_string());
    vars.insert("version_name", options.version_id.clone());
    vars.insert("version_type", merged.version_type.clone());
    vars.insert("resolution_width", width.to_string());
    vars.insert("resolution_height", height.to_string());
    vars.insert("primary_jar", client_jar.display().to_string());
    vars.insert("game_assets", paths.assets.display().to_string());

    // JVM 参数：-Xmx 放在最前面，其余按 json 顺序
    let mut jvm_args: Vec<String> = Vec::new();
    jvm_args.push(format!("-Xmx{}M", options.clamped_memory()));
    jvm_args.push(format!("-Xms{}M", (options.clamped_memory() / 2).max(MEMORY_MIN_MB)));
    jvm_args.push("-Dfile.encoding=UTF-8".to_string());
    jvm_args.push(format!(
        "-Dminecraft.launcher.brand={LAUNCHER_NAME}"
    ));
    jvm_args.push(format!(
        "-Dminecraft.launcher.version={LAUNCHER_VERSION}"
    ));

    let from_json = version::collect_arguments(&merged.jvm_arguments, &platform, &features);
    if from_json.is_empty() {
        // 老版本没有 arguments 字段：手工补上运行时必需的参数
        jvm_args.push(format!("-Djava.library.path={}", natives.display()));
        jvm_args.push("-cp".to_string());
        jvm_args.push(classpath.clone());
    } else {
        for arg in from_json {
            let expanded = substitute(&arg, &vars);
            // 已经自己给过同类参数就不重复塞
            if expanded.starts_with("-Dminecraft.launcher.brand=")
                || expanded.starts_with("-Dminecraft.launcher.version=")
                || expanded.starts_with("-Xmx")
                || expanded.starts_with("-Xms")
            {
                continue;
            }
            jvm_args.push(expanded);
        }
    }
    // 兜底：确保 java.library.path 一定在
    if !jvm_args
        .iter()
        .any(|arg| arg.starts_with("-Djava.library.path="))
    {
        jvm_args.push(format!("-Djava.library.path={}", natives.display()));
    }

    // 游戏参数
    let mut game_args: Vec<String> = Vec::new();
    if !merged.game_arguments.is_empty() {
        for arg in version::collect_arguments(&merged.game_arguments, &platform, &features) {
            game_args.push(substitute(&arg, &vars));
        }
    } else if let Some(template) = &merged.minecraft_arguments {
        for arg in template.split_whitespace() {
            game_args.push(substitute(arg, &vars));
        }
    }

    // 窗口尺寸：json 里已经有 --width 就不重复；否则补上
    if !game_args.iter().any(|arg| arg == "--width") {
        game_args.push("--width".to_string());
        game_args.push(width.to_string());
        game_args.push("--height".to_string());
        game_args.push(height.to_string());
    }

    let main_class = merged
        .main_class
        .clone()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            AppError::Other(format!(
                "版本 {} 的 json 里没有 mainClass，无法启动",
                options.version_id
            ))
        })?;

    Ok(LaunchCommand {
        java: options.java_path.clone(),
        main_class,
        jvm_args,
        game_args,
        classpath,
        natives,
        game_dir: options.game_dir.clone(),
    })
}

/// 游戏日志文件：.mc\logs\game-<实例>-<时间>.log
pub fn game_log_path(paths: &Paths, instance_name: &str) -> PathBuf {
    let stamp = util::compact_stamp(util::now_unix());
    paths.logs.join(format!(
        "game-{}-{stamp}.log",
        util::sanitize_file_part(instance_name)
    ))
}

/// spawn 出来的游戏进程句柄 + 用于日志转发的元数据。
pub struct GameSession {
    pub instance_id: i64,
    pub pid: Option<u32>,
    pub log_path: PathBuf,
}

/// 起进程：立刻返回，日志与退出在后台任务里转发。
pub async fn start(
    events: Arc<dyn Events>,
    paths: Paths,
    options: &LaunchOptions,
    command: &LaunchCommand,
) -> Result<GameSession> {
    if !command.java.is_file() {
        return Err(AppError::Other(format!(
            "Java 不存在：{}。请到设置里重新选择 Java",
            command.java.display()
        )));
    }
    std::fs::create_dir_all(&command.game_dir)?;
    std::fs::create_dir_all(&command.natives)?;
    let log_path = game_log_path(&paths, &options.instance_name);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut child = tokio::process::Command::new(&command.java)
        .args(&command.jvm_args)
        .arg(&command.main_class)
        .args(&command.game_args)
        .current_dir(&command.game_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            AppError::Other(format!(
                "启动 Java 失败（{}）：{err}。请确认所选 Java 版本与游戏版本匹配",
                command.java.display()
            ))
        })?;

    let pid = child.id();
    tracing::info!(
        instance = options.instance_name,
        pid = ?pid,
        log = %log_path.display(),
        "游戏进程已启动"
    );

    let instance_id = options.instance_id;
    let db_file = paths.db_file.clone();
    let log_for_task = log_path.clone();
    let events_for_task = events.clone();
    let started = Instant::now();

    tokio::spawn(async move {
        let writer = std::sync::Arc::new(std::sync::Mutex::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_for_task)
                .ok(),
        ));

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let mut handles = Vec::new();
        if let Some(stdout) = stdout {
            handles.push(tokio::spawn(pump(
                stdout,
                events_for_task.clone(),
                writer.clone(),
                instance_id,
            )));
        }
        if let Some(stderr) = stderr {
            handles.push(tokio::spawn(pump(
                stderr,
                events_for_task.clone(),
                writer.clone(),
                instance_id,
            )));
        }
        for handle in handles {
            let _ = handle.await;
        }

        let status = child.wait().await;
        let code = match status {
            Ok(status) => status.code(),
            Err(err) => {
                tracing::warn!(%err, "等待游戏进程退出失败");
                None
            }
        };
        let elapsed = started.elapsed().as_secs() as i64;
        events_for_task.game_exit(instance_id, code);
        if let Ok(conn) = crate::db::open(&db_file) {
            if let Err(err) = crate::instance::add_playtime(&conn, instance_id, elapsed) {
                tracing::warn!(%err, "累加游玩时长失败");
            }
        }
        tracing::info!(instance_id, code = ?code, seconds = elapsed, "游戏已退出");
    });

    Ok(GameSession {
        instance_id,
        pid,
        log_path,
    })
}

async fn pump<R>(
    reader: R,
    events: Arc<dyn Events>,
    writer: Arc<std::sync::Mutex<Option<std::fs::File>>>,
    instance_id: i64,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(reader).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if let Ok(mut guard) = writer.lock() {
                    if let Some(file) = guard.as_mut() {
                        use std::io::Write;
                        let _ = writeln!(file, "{line}");
                    }
                }
                // 原样转发：前端浮窗与控制台用同一份日志
                tracing::debug!(instance_id, "{}", line);
                events.game_log(instance_id, &line);
            }
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(%err, "读取游戏输出失败");
                break;
            }
        }
    }
}

/// 日志文件路径存在性（命令层 open_path 用）。
pub fn log_exists(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::merge_chain;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("gugu-launch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    fn modern_version() -> MergedVersion {
        let json: version::VersionJson = serde_json::from_str(
            r#"{
                "id": "1.20.1",
                "type": "release",
                "mainClass": "net.minecraft.client.main.Main",
                "assets": "5",
                "libraries": [
                    {"name": "com.mojang:logging:1.1.1", "downloads": {"artifact": {"path": "com/mojang/logging/1.1.1/logging-1.1.1.jar", "url": "https://example.com/a.jar"}}},
                    {"name": "org.lwjgl:lwjgl:3.3.1", "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar", "url": "https://example.com/b.jar"}}},
                    {"name": "org.lwjgl:lwjgl:3.3.1:natives-windows", "rules": [{"action": "allow", "os": {"name": "windows"}}], "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar", "url": "https://example.com/n.jar"}}}
                ],
                "arguments": {
                    "jvm": [
                        "-Djava.library.path=${natives_directory}",
                        "-Dminecraft.launcher.brand=${launcher_name}",
                        "-cp",
                        "${classpath}"
                    ],
                    "game": [
                        "--username", "${auth_player_name}",
                        "--version", "${version_name}",
                        "--gameDir", "${game_directory}",
                        "--assetsDir", "${assets_root}",
                        "--assetIndex", "${assets_index_name}",
                        "--uuid", "${auth_uuid}",
                        "--accessToken", "${auth_access_token}",
                        {"rules": [{"action": "allow", "features": {"has_custom_resolution": true}}], "value": ["--width", "${resolution_width}", "--height", "${resolution_height}"]}
                    ]
                }
            }"#,
        )
        .unwrap();
        merge_chain(&[json]).unwrap()
    }

    fn options(paths: &Paths) -> LaunchOptions {
        LaunchOptions {
            instance_id: 1,
            instance_name: "测试实例".to_string(),
            version_id: "1.20.1".to_string(),
            game_dir: paths.instances.join("测试实例").join(".minecraft"),
            java_path: PathBuf::from("C:\\Java\\bin\\javaw.exe"),
            memory_mb: 2048,
            width: 1280,
            height: 720,
            username: "Steve".to_string(),
            uuid: String::new(),
            access_token: String::new(),
        }
    }

    #[test]
    fn builds_classpath_memory_natives_and_window_args() {
        let paths = temp_paths("modern");
        let merged = modern_version();
        let command = build_command(&paths, &merged, &options(&paths)).unwrap();

        assert_eq!(command.main_class, "net.minecraft.client.main.Main");
        assert!(command.jvm_args.contains(&"-Xmx2048M".to_string()));
        assert!(command.jvm_args.contains(&"-Xms1024M".to_string()));
        assert!(command
            .jvm_args
            .iter()
            .any(|arg| arg.starts_with("-Djava.library.path=") && arg.contains("natives")));

        // classpath 含两个普通库 + 客户端 jar，natives jar 不进 classpath
        let parts: Vec<&str> = command.classpath.split(';').collect();
        assert_eq!(parts.len(), 3, "classpath 应当是两个库 + client.jar：{:?}", parts);
        assert!(parts[0].ends_with("logging-1.1.1.jar"));
        assert!(parts[1].ends_with("lwjgl-3.3.1.jar"));
        assert!(parts[2].ends_with("1.20.1.jar"));
        assert!(!command.classpath.contains("natives-windows"));

        assert_eq!(command.natives, paths.versions.join("1.20.1").join("natives"));

        // 游戏参数
        let joined = command.game_args.join(" ");
        assert!(joined.contains("--username Steve"));
        assert!(joined.contains("--assetsDir"));
        assert!(joined.contains("--assetIndex 5"));
        assert_eq!(
            command.game_args.iter().filter(|arg| *arg == "--width").count(),
            1,
            "窗口尺寸不能重复"
        );
        assert!(joined.contains("--width 1280"));
        assert!(joined.contains("--height 720"));
        assert!(joined.contains("--gameDir"));
        assert_eq!(command.game_args.iter().filter(|arg| *arg == "--width").count(), 1);

        // 用户名派生 uuid
        let uuid = command
            .game_args
            .iter()
            .skip_while(|arg| arg.as_str() != "--uuid")
            .nth(1)
            .cloned()
            .unwrap();
        assert_eq!(uuid.len(), 32);
        assert_eq!(uuid, offline_uuid("Steve"));

        // 品牌参数不重复（json 里的那条被自己的覆盖）
        let brand: Vec<&String> = command
            .jvm_args
            .iter()
            .filter(|arg| arg.as_str().starts_with("-Dminecraft.launcher.brand="))
            .collect();
        assert_eq!(brand.len(), 1);

        // -cp 的参数里是完整 classpath
        let index = command
            .jvm_args
            .iter()
            .position(|arg| arg == "-cp")
            .expect("应当有 -cp");
        assert_eq!(command.jvm_args[index + 1], command.classpath);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn legacy_minecraft_arguments_template_expands() {
        let paths = temp_paths("legacy");
        let json: version::VersionJson = serde_json::from_str(
            r#"{
                "id": "1.7.10",
                "mainClass": "net.minecraft.client.main.Main",
                "assets": "legacy",
                "minecraftArguments": "--username ${auth_player_name} --gameDir ${game_directory} --assetsDir ${assets_root} --assetIndex ${assets_index_name} --uuid ${auth_uuid} --accessToken ${auth_access_token} --userType ${user_type} --versionType ${version_type}",
                "libraries": []
            }"#,
        )
        .unwrap();
        let merged = merge_chain(&[json]).unwrap();
        let mut opts = options(&paths);
        opts.version_id = "1.7.10".to_string();
        let command = build_command(&paths, &merged, &opts).unwrap();

        assert!(command.jvm_args.contains(&"-cp".to_string()));
        assert!(command
            .jvm_args
            .iter()
            .any(|arg| arg.starts_with("-Djava.library.path=")));
        let joined = command.game_args.join(" ");
        assert!(joined.contains("--username Steve"));
        assert!(joined.contains("--assetIndex legacy"));
        assert!(joined.contains("--userType legacy"));
        assert!(joined.contains("--versionType release"));
        // 老版本没有自定义分辨率 feature，仍然补 --width/--height
        assert!(joined.contains("--width 1280"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn missing_main_class_is_a_readable_error() {
        let paths = temp_paths("nomain");
        let json: version::VersionJson = serde_json::from_str(r#"{"id": "broken", "libraries": []}"#).unwrap();
        let merged = merge_chain(&[json]).unwrap();
        let err = build_command(&paths, &merged, &options(&paths)).unwrap_err();
        assert!(err.to_string().contains("mainClass"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn memory_is_clamped_and_defaults_apply() {
        let paths = temp_paths("clamp");
        let merged = modern_version();
        let mut opts = options(&paths);
        opts.memory_mb = 100;
        opts.width = 0;
        opts.height = 0;
        opts.username = "  ".to_string();
        let command = build_command(&paths, &merged, &opts).unwrap();
        assert!(command.jvm_args.contains(&"-Xmx512M".to_string()));
        let joined = command.game_args.join(" ");
        assert!(joined.contains("--username Player"));
        assert!(joined.contains(&format!("--width {DEFAULT_WIDTH}")));
        assert!(joined.contains(&format!("--height {DEFAULT_HEIGHT}")));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn substitute_keeps_unknown_placeholders() {
        let mut vars = BTreeMap::new();
        vars.insert("a", "1".to_string());
        assert_eq!(substitute("${a}-${b}", &vars), "1-${b}");
        assert_eq!(substitute("plain", &vars), "plain");
        assert_eq!(substitute("${a", &vars), "${a");
    }

    #[test]
    fn offline_uuid_is_stable_and_versioned() {
        let first = offline_uuid("Steve");
        assert_eq!(first.len(), 32);
        assert_eq!(first, offline_uuid("Steve"));
        assert_ne!(first, offline_uuid("Alex"));
        let version_nibble = first.as_bytes()[12] as char;
        assert_eq!(version_nibble, '3');
    }

    #[test]
    fn game_log_path_is_sanitized() {
        let paths = temp_paths("logpath");
        let path = game_log_path(&paths, "a/b:c");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("game-a_b_c-"));
        assert!(name.ends_with(".log"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn display_command_contains_main_class() {
        let paths = temp_paths("display");
        let merged = modern_version();
        let command = build_command(&paths, &merged, &options(&paths)).unwrap();
        let text = command.to_display();
        assert!(text.contains("net.minecraft.client.main.Main"));
        assert!(text.contains("-Xmx2048M"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[tokio::test]
    async fn start_reports_readable_error_when_java_missing() {
        let paths = temp_paths("nojava");
        let merged = modern_version();
        let mut opts = options(&paths);
        opts.java_path = PathBuf::from("C:\\nope\\javaw.exe");
        let command = build_command(&paths, &merged, &opts).unwrap();
        let err = start(
            Arc::new(crate::events::NoopEvents),
            paths.clone(),
            &opts,
            &command,
        )
        .await
        .err()
        .expect("Java 不存在应当报错");
        assert!(err.to_string().contains("Java 不存在"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
