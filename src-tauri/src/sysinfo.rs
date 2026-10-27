//! 本机信息：物理内存、可用内存、CPU 核数、系统版本（实例内存滑块上限用）。

use serde::Serialize;

/// system_info() 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    /// 物理内存总量（MB，取不到为 0，前端按 8192 处理）
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub cpu_count: u32,
    pub os: String,
}

/// 拿不到物理内存时的兜底上限。
pub const FALLBACK_MEMORY_MB: u64 = 8192;

/// bytes → MB（四舍五入）。
pub fn bytes_to_mb(bytes: u64) -> u64 {
    (bytes + 1024 * 1024 / 2) / (1024 * 1024)
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    pub struct MemoryStatusEx {
        pub length: u32,
        pub memory_load: u32,
        pub total_phys: u64,
        pub avail_phys: u64,
        pub total_page_file: u64,
        pub avail_page_file: u64,
        pub total_virtual: u64,
        pub avail_virtual: u64,
        pub avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }

    pub fn memory() -> (u64, u64) {
        let mut status = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: 结构体按 Win32 布局定义，length 已按 sizeof 填好
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        if ok == 0 {
            (0, 0)
        } else {
            (status.total_phys, status.avail_phys)
        }
    }
}

/// 取本机信息（拿不到的字段给 0）。
pub fn info() -> SystemInfo {
    #[cfg(windows)]
    {
        let (total, available) = win::memory();
        SystemInfo {
            total_memory_mb: bytes_to_mb(total),
            available_memory_mb: bytes_to_mb(available),
            cpu_count: std::thread::available_parallelism()
                .map(|value| value.get() as u32)
                .unwrap_or(0),
            os: format!("Windows {}", std::env::consts::ARCH),
        }
    }
    #[cfg(not(windows))]
    {
        SystemInfo {
            total_memory_mb: 0,
            available_memory_mb: 0,
            cpu_count: std::thread::available_parallelism()
                .map(|value| value.get() as u32)
                .unwrap_or(0),
            os: std::env::consts::OS.to_string(),
        }
    }
}

/// 实例内存上限：本机物理内存（拿不到就用 8192）。
pub fn memory_ceiling_mb() -> u64 {
    let total = info().total_memory_mb;
    if total == 0 {
        FALLBACK_MEMORY_MB
    } else {
        total
    }
}

/// 按上限夹紧实例内存；超了返回 (夹紧值, true)。
pub fn clamp_instance_memory(requested: u32, ceiling_mb: u64) -> (u32, bool) {
    let ceiling = if ceiling_mb == 0 {
        FALLBACK_MEMORY_MB
    } else {
        ceiling_mb
    };
    let ceiling_u32 = ceiling.min(u32::MAX as u64) as u32;
    if requested as u64 > ceiling {
        (ceiling_u32.max(crate::settings::MEMORY_MB_MIN), true)
    } else {
        (requested, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_mb_rounds() {
        assert_eq!(bytes_to_mb(0), 0);
        assert_eq!(bytes_to_mb(1024 * 1024), 1);
        assert_eq!(bytes_to_mb(1024 * 1024 + 524_287), 1);
        assert_eq!(bytes_to_mb(1024 * 1024 + 524_288), 2);
        assert_eq!(bytes_to_mb(16 * 1024 * 1024 * 1024), 16 * 1024);
    }

    #[test]
    fn clamp_uses_ceiling_and_reports_change() {
        assert_eq!(clamp_instance_memory(4096, 16_384), (4096, false));
        assert_eq!(clamp_instance_memory(32_768, 16_384), (16_384, true));
        assert_eq!(
            clamp_instance_memory(16_384, 0),
            (FALLBACK_MEMORY_MB as u32, true)
        );
        assert_eq!(
            clamp_instance_memory(4096, 256),
            (crate::settings::MEMORY_MB_MIN, true)
        );
    }

    #[test]
    fn info_reports_real_machine() {
        let info = info();
        assert!(info.cpu_count >= 1, "{info:?}");
        #[cfg(windows)]
        {
            assert!(info.total_memory_mb > 0, "Windows 上必须拿得到物理内存");
            assert!(info.available_memory_mb <= info.total_memory_mb);
            assert!(info.os.starts_with("Windows"));
            assert_eq!(memory_ceiling_mb(), info.total_memory_mb);
        }
    }
}
