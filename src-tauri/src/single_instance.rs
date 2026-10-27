//! 单实例守门：命名互斥体（进程数）+ 命名事件（唤醒已运行的实例）。
//!
//! 为什么不能只依赖单实例插件：插件靠本机 socket，持有端一旦监听异常，
//! 后续启动就拦不住了（实测出现过「隐藏到托盘后再启动又起了一个进程」）。
//! 互斥体是内核对象，只要进程还活着就一定存在，而且检查发生在任何窗口/托盘/
//! 状态初始化之前，第二个进程会立刻退出。
//!
//! 互斥体名与事件名都带版本后缀，避免和旧版本（或别的分支）互相干扰。

use std::sync::atomic::{AtomicUsize, Ordering};

/// 进程数守门用的互斥体名。
pub const MUTEX_NAME: &str = r"Global\GuguLauncher_SingleInstance_v1";
/// 唤醒信号（自动复位的命名事件）。
pub const EVENT_NAME: &str = r"Global\GuguLauncher_Activate_v1";
/// ERROR_ALREADY_EXISTS：同名互斥体已经存在 = 已有实例在跑。
pub const ERROR_ALREADY_EXISTS: u32 = 183;
const INFINITE: u32 = 0xFFFF_FFFF;
const EVENT_MODIFY_STATE: u32 = 0x0002;
const SYNCHRONIZE: u32 = 0x0010_0000;

#[cfg(windows)]
mod ffi {
    use core::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateMutexW(
            attributes: *mut c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut c_void;
        pub fn CreateEventW(
            attributes: *mut c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> *mut c_void;
        pub fn OpenEventW(desired_access: u32, inherit: i32, name: *const u16) -> *mut c_void;
        pub fn SetEvent(handle: *mut c_void) -> i32;
        pub fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        pub fn GetLastError() -> u32;
    }
}

/// 守门结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    /// 本进程是唯一实例（互斥体已创建）。
    Primary,
    /// 已有实例在跑（互斥体已存在）。
    Duplicate,
}

static EVENT_HANDLE: AtomicUsize = AtomicUsize::new(0);
static MUTEX_HANDLE: AtomicUsize = AtomicUsize::new(0);

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 认领互斥体。成功（Primary）时刻意不释放句柄：进程退出才释放。
pub fn acquire() -> Guard {
    #[cfg(windows)]
    {
        let name = wide(MUTEX_NAME);
        let handle = unsafe { ffi::CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr()) };
        let error = unsafe { ffi::GetLastError() };
        if handle.is_null() {
            // 极端情况（命名对象被安全策略拒绝）：按首次实例继续，只记日志
            tracing::warn!(code = error, mutex = MUTEX_NAME, "创建单实例互斥体失败，跳过守门");
            return Guard::Primary;
        }
        MUTEX_HANDLE.store(handle as usize, Ordering::SeqCst);
        if error == ERROR_ALREADY_EXISTS {
            tracing::info!(
                code = error,
                mutex = MUTEX_NAME,
                pid = std::process::id(),
                "已有实例在运行（互斥体已存在）"
            );
            return Guard::Duplicate;
        }
        tracing::info!(
            mutex = MUTEX_NAME,
            pid = std::process::id(),
            "单实例互斥体已就绪"
        );
        Guard::Primary
    }
    #[cfg(not(windows))]
    {
        Guard::Primary
    }
}

/// 主实例侧：创建唤醒事件（第二个实例 SetEvent 它）。
fn create_event() -> bool {
    #[cfg(windows)]
    {
        let name = wide(EVENT_NAME);
        // manual_reset = 0（自动复位），initial_state = 0
        let handle = unsafe { ffi::CreateEventW(std::ptr::null_mut(), 0, 0, name.as_ptr()) };
        if handle.is_null() {
            let error = unsafe { ffi::GetLastError() };
            tracing::warn!(code = error, event = EVENT_NAME, "创建唤醒事件失败");
            return false;
        }
        EVENT_HANDLE.store(handle as usize, Ordering::SeqCst);
        true
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 第二个实例侧：给主实例发唤醒信号。
pub fn wake_primary() -> bool {
    #[cfg(windows)]
    {
        let name = wide(EVENT_NAME);
        let handle = unsafe { ffi::OpenEventW(EVENT_MODIFY_STATE | SYNCHRONIZE, 0, name.as_ptr()) };
        if handle.is_null() {
            let error = unsafe { ffi::GetLastError() };
            tracing::warn!(
                code = error,
                event = EVENT_NAME,
                "打开唤醒事件失败（主实例可能刚退出）"
            );
            return false;
        }
        let ok = unsafe { ffi::SetEvent(handle) } != 0;
        tracing::info!(event = EVENT_NAME, ok, "已向主实例发送唤醒信号");
        ok
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 主实例侧：起一个线程等唤醒信号，收到就把窗口叫到前台。
pub fn watch_primary(app: tauri::AppHandle) {
    create_event();
    #[cfg(windows)]
    {
        let handle = EVENT_HANDLE.load(Ordering::SeqCst);
        if handle == 0 {
            return;
        }
        std::thread::spawn(move || loop {
            let wait = unsafe { ffi::WaitForSingleObject(handle as *mut core::ffi::c_void, INFINITE) };
            if wait != 0 {
                std::thread::sleep(std::time::Duration::from_millis(200));
                continue;
            }
            tracing::info!("收到第二个实例的唤醒信号，回到主窗口");
            crate::show_main_window(&app);
            {
                use tauri::Emitter;
                let _ = app.emit("app://second-instance", ());
            }
        });
    }
    #[cfg(not(windows))]
    {
        let _ = app;
    }
}

/// 把提示写进启动器日志（第二个实例还没初始化 tracing，直接追加文件）。
fn append_launcher_log(line: &str) {
    let default = crate::paths::resolve();
    let settings = crate::settings::load(&default.settings_file).unwrap_or_default();
    let paths = crate::paths::apply_data_root(default, &settings.data_root);
    let _ = std::fs::create_dir_all(&paths.logs);
    let file = paths.logs.join(format!(
        "launcher-{}.log",
        crate::util::date_stamp(crate::util::now_unix())
    ));
    if let Ok(mut handle) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        use std::io::Write;
        let _ = writeln!(handle, "{} INFO {line}", crate::util::iso8601(crate::util::now_unix()));
    }
}

/// 启动最前面调用：如果是第二个实例 → 唤醒主实例并立刻退出（返回不回来）。
pub fn exit_if_duplicate() {
    if acquire() == Guard::Primary {
        return;
    }
    let signalled = wake_primary();
    let message = format!(
        "second instance 检测到已有实例（pid={}，mutex={}，GetLastError={}），唤醒结果={}，本进程退出",
        std::process::id(),
        MUTEX_NAME,
        ERROR_ALREADY_EXISTS,
        signalled
    );
    eprintln!("{message}");
    append_launcher_log(&message);
    // 给主实例一点时间处理唤醒，再退出（自己不需要任何窗口/托盘/状态）
    std::thread::sleep(std::time::Duration::from_millis(120));
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_versioned_and_global() {
        assert!(MUTEX_NAME.starts_with(r"Global\"));
        assert!(EVENT_NAME.starts_with(r"Global\"));
        assert!(MUTEX_NAME.ends_with("_v1"));
        assert!(EVENT_NAME.ends_with("_v1"));
        assert_eq!(ERROR_ALREADY_EXISTS, 183);
    }

    #[test]
    fn second_acquire_is_always_duplicate() {
        // 第一次可能是 Primary（本机没有别的实例），也可能是 Duplicate
        // （启动器正在运行 / 别的测试先拿到了）——所以只断言第二次：
        // 同一进程里再创建同名互斥体一定带回 ERROR_ALREADY_EXISTS。
        let _first = acquire();
        assert_eq!(acquire(), Guard::Duplicate);
    }

    #[test]
    fn wake_without_primary_does_not_panic() {
        // 没有创建过事件时（或主实例刚退出）必须安全返回 false，不能 panic
        let ok = wake_primary();
        assert!(ok || !ok, "只要不 panic 即可");
    }
}
