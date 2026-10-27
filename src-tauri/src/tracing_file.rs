//! tracing 的文件输出层：日志同时写 .mc\\logs\\launcher-<date>.log。
//!
//! 自己实现 MakeWriter 而不是引 tracing-appender：只需要「进程内一个追加文件」，
//! 少一个依赖，也不会因为跨天滚动把句柄搞复杂。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// 可克隆的追加文件句柄（每次 make_writer 拿到的都是同一个文件）。
#[derive(Clone)]
pub struct FileWriter {
    file: Arc<Mutex<std::fs::File>>,
}

impl FileWriter {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }
}

impl Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.file.lock() {
            Ok(mut file) => file.write(buf),
            Err(_) => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.lock() {
            Ok(mut file) => file.flush(),
            Err(_) => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileWriter {
    type Writer = FileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_append_to_the_log_file() {
        let dir = std::env::temp_dir().join(format!("gugu-trace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("logs").join("launcher-test.log");
        {
            let mut writer = FileWriter::open(&path).expect("文件应当能创建");
            writer.write_all(b"first\n").unwrap();
            writer.flush().unwrap();
        }
        {
            let mut writer = FileWriter::open(&path).unwrap();
            writer.write_all(b"second\n").unwrap();
            writer.flush().unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "first\nsecond\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
