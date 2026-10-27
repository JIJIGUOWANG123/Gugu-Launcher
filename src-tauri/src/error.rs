//! 统一错误类型：命令层直接返回 `Result<T, AppError>`，前端拿到的是人话字符串。

use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Db(rusqlite::Error),
    Http(reqwest::Error),
    /// 取值校验失败（设置、实例名等）
    Invalid(String),
    /// M1 只有签名、没有实现的功能（M2 起基本只剩占位）
    NotImplemented(&'static str),
    /// 多源下载全部失败：带已尝试的源与最后一条错误
    Net(String),
    /// 用户主动取消（不算失败，也不弹错误框）
    Cancelled,
    Other(String),
}

pub type Result<T> = std::result::Result<T, AppError>;

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Io(err) => write!(f, "IO 错误：{err}"),
            AppError::Json(err) => write!(f, "JSON 解析失败：{err}"),
            AppError::Db(err) => write!(f, "数据库错误：{err}"),
            AppError::Http(err) => write!(f, "网络请求失败：{err}"),
            AppError::Invalid(msg) => write!(f, "参数不合法：{msg}"),
            AppError::NotImplemented(what) => write!(f, "未实现：{what}（M1 骨架只留接口）"),
            AppError::Net(msg) => write!(f, "下载失败：{msg}"),
            AppError::Cancelled => write!(f, "任务已取消"),
            AppError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Json(err)
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::Db(err)
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Http(err)
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Other(err.to_string())
    }
}

/// 前端拿到的是错误文案字符串（不暴露内部结构）。
impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
