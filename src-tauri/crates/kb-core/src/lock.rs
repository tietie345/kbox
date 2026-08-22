use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::{KbError, Result};

/// 文件锁，用于防止多个进程同时操作数据库
/// 锁文件路径: {vault_path}/.kb/index.db.lock
pub struct FileLock {
    lock_path: PathBuf,
    /// 是否持有锁
    owned: bool,
}

impl FileLock {
    /// 尝试获取锁
    /// - timeout: 最大等待时间（秒）
    pub fn acquire(vault_path: &Path, timeout_secs: u64) -> Result<Self> {
        let lock_path = vault_path.join(".kb").join("index.db.lock");
        let start = SystemTime::now();
        let pid = std::process::id();
        let hostname = hostname().unwrap_or_else(|| "unknown".to_string());

        loop {
            // 尝试创建锁文件（原子操作，文件不存在时才成功）
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    // 成功获取锁
                    let now = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let content = format!("{}:{}:{}", pid, now, hostname);
                    let _ = file.write_all(content.as_bytes());
                    let _ = file.flush();
                    return Ok(FileLock {
                        lock_path,
                        owned: true,
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // 锁文件已存在，检查是否 stale
                    if let Some(stale) = is_stale_lock(&lock_path) {
                        if stale {
                            // 删除 stale 锁
                            let _ = fs::remove_file(&lock_path);
                            continue;
                        }
                    }

                    // 超时检查
                    if SystemTime::now()
                        .duration_since(start)
                        .unwrap_or(Duration::ZERO)
                        >= Duration::from_secs(timeout_secs)
                    {
                        return Err(KbError::Busy);
                    }

                    // 等待 100ms 后重试
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(KbError::Io(e)),
            }
        }
    }

    /// 释放锁
    pub fn release(&mut self) {
        if self.owned {
            let _ = fs::remove_file(&self.lock_path);
            self.owned = false;
        }
    }

    /// 检查是否仍持有锁
    pub fn is_owned(&self) -> bool {
        self.owned
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        self.release();
    }
}

/// 检查锁文件是否 stale（进程已崩溃或锁文件超过 30 秒）
fn is_stale_lock(lock_path: &Path) -> Option<bool> {
    // 方法 1: 检查修改时间
    if let Ok(meta) = fs::metadata(lock_path) {
        if let Ok(modified) = meta.modified() {
            if let Ok(elapsed) = SystemTime::now().duration_since(modified) {
                if elapsed > Duration::from_secs(30) {
                    return Some(true);
                }
            }
        }
    }

    // 方法 2: 尝试读取锁文件中的进程 ID，检查进程是否存活
    if let Ok(mut file) = fs::File::open(lock_path) {
        let mut content = String::new();
        if file.read_to_string(&mut content).is_ok() {
            if let Some(pid_str) = content.split(':').next() {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    // 在 Unix 上，kill 0 可以检查进程是否存在
                    #[cfg(unix)]
                    {
                        let result = std::process::Command::new("kill")
                            .arg("-0")
                            .arg(pid.to_string())
                            .output();
                        if let Ok(output) = result {
                            // 进程不存在
                            return Some(!output.status.success());
                        }
                    }
                }
            }
        }
    }

    None
}

/// 获取主机名（用于锁文件标识）
fn hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .or_else(|_| std::env::var("HOST"))
        .ok()
}
