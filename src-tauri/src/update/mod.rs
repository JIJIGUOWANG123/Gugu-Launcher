//! 更新相关：版本号解析/比较/资产选择（version.rs）+ 检查状态机（check.rs）。
//!
//! 1.0.0 保留检测更新能力（发布 1.0.1 时要用）：启动后延迟检查 + 每 6 小时一次，
//! 结果只显示在「关于」页，不弹窗、不发系统通知。

pub mod check;
pub mod installer;
pub mod version;
