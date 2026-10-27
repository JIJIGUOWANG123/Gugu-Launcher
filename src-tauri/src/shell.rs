//! 系统集成：打开目录、选择图标文件、日志文件列表与读取。
//!
//! 不引 tauri-plugin-dialog：图标选择直接用 PowerShell 的 WinForms 对话框，
//! 拿不到返回值就返回空字符串（前端按“用户取消”处理）。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::paths::Paths;
use crate::util;

/// 单个日志文件读多少字节（超过就只读尾部）。
pub const LOG_READ_LIMIT: usize = 4 * 1024 * 1024;

/// logs_list() 的元素（name/size/modified 与 M2-B 契约一致，另附全量路径与时间）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified_at: String,
    pub modified: String,
}

/// 用系统默认程序打开一个目录。
pub fn open_folder(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(AppError::Invalid(format!("目录不存在：{}", path.display())));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|err| AppError::Other(format!("打开资源管理器失败：{err}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|err| AppError::Other(format!("打开文件管理器失败：{err}")))?;
        Ok(())
    }
}

/// 实例名对应的日志前缀过滤（game-<实例>-*.log）。
pub fn log_matches_instance(name: &str, instance_name: &str) -> bool {
    let prefix = format!("game-{}-", util::sanitize_file_part(instance_name));
    name.starts_with(&prefix) || name.starts_with("launcher-")
}

/// 列出 .mc\logs 下的日志，按修改时间从新到旧。
pub fn list_logs(paths: &Paths, instance_name: Option<&str>) -> Result<Vec<LogFile>> {
    let mut out: Vec<(std::time::SystemTime, LogFile)> = Vec::new();
    let entries = match std::fs::read_dir(&paths.logs) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(AppError::Io(err)),
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) if name.ends_with(".log") => name.to_string(),
            _ => continue,
        };
        if let Some(instance_name) = instance_name {
            if !log_matches_instance(&name, instance_name) {
                continue;
            }
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| util::iso8601(duration.as_secs()))
            .unwrap_or_default();
        out.push((
            metadata
                .modified()
                .unwrap_or(std::time::UNIX_EPOCH),
            LogFile {
                name,
                path: path.display().to_string(),
                size: metadata.len(),
                modified_at: modified_at.clone(),
                modified: modified_at,
            },
        ));
    }
    out.sort_by(|left, right| right.0.cmp(&left.0));
    Ok(out.into_iter().map(|(_, file)| file).collect())
}

/// 校验日志文件名（只允许 logs 目录下的 .log 文件，挡掉路径穿越）。
pub fn validate_log_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("日志文件名不能为空".to_string()));
    }
    if !trimmed.ends_with(".log") {
        return Err(AppError::Invalid("只支持读取 .log 文件".to_string()));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(AppError::Invalid(format!("日志文件名不合法：{trimmed}")));
    }
    Ok(())
}

/// 读一个日志文件（超过 LOG_READ_LIMIT 只返回尾部，免得把前端撑爆）。
pub fn read_log(paths: &Paths, name: &str) -> Result<String> {
    validate_log_name(name)?;
    let path = paths.logs.join(name);
    if !path.is_file() {
        return Err(AppError::Invalid(format!("日志文件不存在：{name}")));
    }
    let bytes = std::fs::read(&path)?;
    if bytes.len() <= LOG_READ_LIMIT {
        return Ok(String::from_utf8_lossy(&bytes).to_string());
    }
    let tail = &bytes[bytes.len() - LOG_READ_LIMIT..];
    Ok(format!(
        "……（文件较大，仅显示最后 {} KB）……\n{}",
        LOG_READ_LIMIT / 1024,
        String::from_utf8_lossy(tail)
    ))
}

/// 从 PowerShell 对话框输出里取第一行存在的文件路径。
pub fn first_existing_path(output: &str) -> Option<String> {
    for line in output.lines() {
        let candidate = line.trim().trim_matches('"');
        if candidate.is_empty() {
            continue;
        }
        if Path::new(candidate).is_file() {
            return Some(candidate.to_string());
        }
    }
    None
}

/// 弹出 WinForms 文件对话框选一张 png，返回用户选中的原始路径。
pub fn pick_png_file() -> Result<String> {
    #[cfg(windows)]
    {
        let script = "Add-Type -AssemblyName System.Windows.Forms; \
$dialog = New-Object System.Windows.Forms.OpenFileDialog; \
$dialog.Filter = 'PNG 图片|*.png'; \
$dialog.Title = '选择实例图标'; \
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $dialog.FileName }";
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-STA", "-Command", script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        crate::util::hidden(&mut cmd);
        let output = cmd
            .output()
            .map_err(|err| AppError::Other(format!("打开文件选择框失败：{err}")))?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(first_existing_path(&text).unwrap_or_default())
    }
    #[cfg(not(windows))]
    {
        Err(AppError::Other("当前系统不支持文件选择框".to_string()))
    }
}

/// 弹出系统目录选择框，返回用户选中的目录；取消返回空字符串。
pub fn pick_directory() -> Result<String> {
    #[cfg(windows)]
    {
        let script = "Add-Type -AssemblyName System.Windows.Forms; \
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog; \
$dialog.Description = '选择下载目录'; \
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $dialog.SelectedPath }";
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-STA", "-Command", script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        crate::util::hidden(&mut cmd);
        let output = cmd
            .output()
            .map_err(|err| AppError::Other(format!("打开目录选择框失败：{err}")))?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(first_existing_dir(&text).unwrap_or_default())
    }
    #[cfg(not(windows))]
    {
        Err(AppError::Other("当前系统不支持目录选择框".to_string()))
    }
}

/// 弹出多选文件框，返回用户选中的文件（一个一行）。取消返回空数组。
pub fn pick_files(filter: &str, title: &str) -> Result<Vec<String>> {
    #[cfg(windows)]
    {
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
$dialog = New-Object System.Windows.Forms.OpenFileDialog; \
$dialog.Filter = '{filter}'; \
$dialog.Title = '{title}'; \
$dialog.Multiselect = $true; \
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ $dialog.FileNames -join [Environment]::NewLine }}",
            filter = filter.replace('\'', ""),
            title = title.replace('\'', ""),
        );
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-STA", "-Command", &script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        crate::util::hidden(&mut cmd);
        let output = cmd
            .output()
            .map_err(|err| AppError::Other(format!("打开文件选择框失败：{err}")))?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(parse_dialog_paths(&text))
    }
    #[cfg(not(windows))]
    {
        let _ = (filter, title);
        Err(AppError::Other("当前系统不支持文件选择框".to_string()))
    }
}

/// 解析多选对话框输出：只保留真实存在的文件。
pub fn parse_dialog_paths(output: &str) -> Vec<String> {
    output
        .lines()
        .map(|line| line.trim().trim_matches('"').to_string())
        .filter(|line| !line.is_empty() && Path::new(line).is_file())
        .collect()
}

/// 从对话框输出里取第一行存在的目录。
pub fn first_existing_dir(output: &str) -> Option<String> {
    for line in output.lines() {
        let candidate = line.trim().trim_matches('"');
        if candidate.is_empty() {
            continue;
        }
        if Path::new(candidate).is_dir() {
            return Some(candidate.to_string());
        }
    }
    None
}

/// 「关闭窗口」三选对话框的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseChoice {
    /// 最小化到后台（继续留在托盘）
    Minimize,
    /// 退出程序
    Exit,
    /// 取消（什么也不做）
    Cancel,
}

impl CloseChoice {
    /// 落盘用的 closeAction 值（Cancel 返回 None）。
    pub fn as_setting(&self) -> Option<&'static str> {
        match self {
            CloseChoice::Minimize => Some("minimize"),
            CloseChoice::Exit => Some("exit"),
            CloseChoice::Cancel => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CloseChoice::Minimize => "minimize",
            CloseChoice::Exit => "exit",
            CloseChoice::Cancel => "cancel",
        }
    }
}

/// 解析对话框输出：GUGU_CLOSE:<DialogResult>:<0|1> → (选择, 是否记住)。
///
/// WinForms 的 DialogResult 数值：Yes = 6（最小化）、No = 7（退出）、Cancel = 2（取消）。
pub fn parse_close_result(output: &str) -> (CloseChoice, bool) {
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("GUGU_CLOSE:") else {
            continue;
        };
        let mut parts = rest.split(':');
        let code = parts.next().unwrap_or("").trim().parse::<i32>().unwrap_or(2);
        let remember = parts.next().unwrap_or("0").trim() == "1";
        let choice = match code {
            6 => CloseChoice::Minimize,
            7 => CloseChoice::Exit,
            _ => CloseChoice::Cancel,
        };
        return (choice, remember);
    }
    // 对话框被关掉（或脚本没跑起来）：当作取消
    (CloseChoice::Cancel, false)
}

/// 关闭窗口时的原生三选对话框（最小化到后台 / 退出程序 / 取消 + 记住我的选择）。
pub fn ask_close_action() -> Result<(CloseChoice, bool)> {
    #[cfg(windows)]
    {
        let script = [
            "Add-Type -AssemblyName System.Windows.Forms",
            "Add-Type -AssemblyName System.Drawing",
            "$form = New-Object System.Windows.Forms.Form",
            "$form.Text = '咕咕启动器'",
            "$form.ClientSize = New-Object System.Drawing.Size(440, 192)",
            "$form.FormBorderStyle = 'FixedDialog'",
            "$form.StartPosition = 'CenterScreen'",
            "$form.MaximizeBox = $false",
            "$form.MinimizeBox = $false",
            "$form.TopMost = $true",
            "$label = New-Object System.Windows.Forms.Label",
            "$label.Text = '关闭窗口后要做什么？'",
            "$label.SetBounds(24, 22, 390, 44)",
            "$form.Controls.Add($label)",
            "$check = New-Object System.Windows.Forms.CheckBox",
            "$check.Text = '记住我的选择'",
            "$check.SetBounds(24, 70, 220, 24)",
            "$form.Controls.Add($check)",
            "$min = New-Object System.Windows.Forms.Button",
            "$min.Text = '最小化到后台'",
            "$min.SetBounds(24, 116, 120, 32)",
            "$min.DialogResult = [System.Windows.Forms.DialogResult]::Yes",
            "$form.Controls.Add($min)",
            "$exit = New-Object System.Windows.Forms.Button",
            "$exit.Text = '退出程序'",
            "$exit.SetBounds(154, 116, 120, 32)",
            "$exit.DialogResult = [System.Windows.Forms.DialogResult]::No",
            "$form.Controls.Add($exit)",
            "$cancel = New-Object System.Windows.Forms.Button",
            "$cancel.Text = '取消'",
            "$cancel.SetBounds(284, 116, 120, 32)",
            "$cancel.DialogResult = [System.Windows.Forms.DialogResult]::Cancel",
            "$form.Controls.Add($cancel)",
            "$form.AcceptButton = $min",
            "$form.CancelButton = $cancel",
            "$result = $form.ShowDialog()",
            "$remember = 0",
            "if ($check.Checked) { $remember = 1 }",
            "Write-Output ('GUGU_CLOSE:' + [int]$result + ':' + $remember)",
        ]
        .join("; ");
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-STA", "-Command", &script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        crate::util::hidden(&mut cmd);
        let output = cmd
            .output()
            .map_err(|err| AppError::Other(format!("打开关闭确认框失败：{err}")))?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(parse_close_result(&text))
    }
    #[cfg(not(windows))]
    {
        Ok((CloseChoice::Minimize, false))
    }
}

/// 日志目录（open_path 用）。
pub fn logs_dir(paths: &Paths) -> PathBuf {
    paths.logs.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("gugu-shell-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_root(&root);
        paths.ensure().unwrap();
        paths
    }

    #[test]
    fn log_name_validation_blocks_traversal() {
        assert!(validate_log_name("latest.log").is_ok());
        assert!(validate_log_name("game-实例-20250101-000000.log").is_ok());
        assert!(validate_log_name("../gugu.db").is_err());
        assert!(validate_log_name("sub/latest.log").is_err());
        assert!(validate_log_name("gugu.db").is_err());
        assert!(validate_log_name("").is_err());
    }

    #[test]
    fn read_log_returns_content_and_rejects_escape() {
        let paths = temp_paths("readlog");
        std::fs::write(paths.logs.join("launcher-2025-01-01.log"), b"[INFO] hi").unwrap();
        let text = read_log(&paths, "launcher-2025-01-01.log").unwrap();
        assert_eq!(text, "[INFO] hi");
        assert!(read_log(&paths, "../gugu.db").is_err());
        assert!(read_log(&paths, "missing.log").is_err());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn read_log_truncates_to_tail() {
        let paths = temp_paths("biglog");
        let mut body = vec![b'a'; LOG_READ_LIMIT + 1024];
        body.extend_from_slice(b"TAIL");
        std::fs::write(paths.logs.join("big.log"), &body).unwrap();
        let text = read_log(&paths, "big.log").unwrap();
        assert!(text.starts_with("……（文件较大"));
        assert!(text.ends_with("TAIL"));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn list_logs_filters_by_instance_and_sorts() {
        let paths = temp_paths("listlog");
        std::fs::write(paths.logs.join("launcher-2025-01-01.log"), b"a").unwrap();
        std::fs::write(paths.logs.join("game-实例A-20250101-000000.log"), b"bb").unwrap();
        std::fs::write(paths.logs.join("game-实例B-20250101-000000.log"), b"ccc").unwrap();
        std::fs::write(paths.logs.join("notes.txt"), b"ignored").unwrap();

        let all = list_logs(&paths, None).unwrap();
        assert_eq!(all.len(), 3, "只要 .log：{:?}", all.iter().map(|f| &f.name).collect::<Vec<_>>());

        let only_a = list_logs(&paths, Some("实例A")).unwrap();
        assert_eq!(only_a.len(), 2, "实例 A 的日志 + 启动器日志");
        assert!(only_a.iter().any(|file| file.name.starts_with("game-实例A-")));
        assert!(!only_a.iter().any(|file| file.name.contains("实例B")));

        // 大小与时间字段都有
        assert!(all.iter().all(|file| !file.modified_at.is_empty()));
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn pick_output_parsing_keeps_only_existing_files() {
        let dir = std::env::temp_dir().join(format!("gugu-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let icon = dir.join("icon.png");
        std::fs::write(&icon, b"png").unwrap();

        let output = format!("\r\n{}\r\n", icon.display());
        assert_eq!(
            first_existing_path(&output),
            Some(icon.display().to_string())
        );
        assert_eq!(first_existing_path("\r\n\r\n"), None);
        assert_eq!(first_existing_path("C:\\nope\\x.png"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pick_directory_output_parsing() {
        let dir = std::env::temp_dir().join(format!("gugu-pickdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            first_existing_dir(&format!("\r\n{}\r\n", dir.display())),
            Some(dir.display().to_string())
        );
        assert_eq!(first_existing_dir("C:\\definitely\\not\\here"), None);
        assert_eq!(first_existing_dir("\r\n"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dialog_paths_keep_only_existing_files() {
        let dir = std::env::temp_dir().join(format!("gugu-pickfiles-{}", util::unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("a.jar");
        let second = dir.join("b.jar");
        std::fs::write(&first, b"1").unwrap();
        std::fs::write(&second, b"2").unwrap();
        let output = format!(
            "\r\n{}\r\n{}\r\nC:\\nope\\c.jar\r\n",
            first.display(),
            second.display()
        );
        let picked = parse_dialog_paths(&output);
        assert_eq!(picked.len(), 2, "{picked:?}");
        assert!(parse_dialog_paths("\r\n").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_matches_instance_helper() {
        assert!(log_matches_instance("game-abc-20250101-000000.log", "abc"));
        assert!(log_matches_instance("launcher-2025-01-01.log", "abc"));
        assert!(!log_matches_instance("game-xyz-20250101-000000.log", "abc"));
    }

    #[test]
    fn open_folder_rejects_missing_directory() {
        let err = open_folder(Path::new("C:\\definitely\\not\\here")).unwrap_err();
        assert!(err.to_string().contains("目录不存在"));
    }

    #[test]
    fn parse_close_result_maps_three_choices() {
        assert_eq!(parse_close_result("noise\r\nGUGU_CLOSE:6:0\r\n"), (CloseChoice::Minimize, false));
        assert_eq!(parse_close_result("GUGU_CLOSE:7:1"), (CloseChoice::Exit, true));
        assert_eq!(parse_close_result("GUGU_CLOSE:2:1"), (CloseChoice::Cancel, true));
        // 关掉对话框 / 脚本没输出 → 取消且不记住
        assert_eq!(parse_close_result(""), (CloseChoice::Cancel, false));
        assert_eq!(parse_close_result("unrelated"), (CloseChoice::Cancel, false));
    }

    #[test]
    fn close_choice_maps_to_setting_values() {
        assert_eq!(CloseChoice::Minimize.as_setting(), Some("minimize"));
        assert_eq!(CloseChoice::Exit.as_setting(), Some("exit"));
        assert_eq!(CloseChoice::Cancel.as_setting(), None);
        assert_eq!(CloseChoice::Cancel.as_str(), "cancel");
    }
}
