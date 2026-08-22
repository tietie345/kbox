use thiserror::Error;

#[derive(Error, Debug)]
pub enum KbError {
    #[error("数据库错误: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("对象不存在: {0}")]
    NotFound(String),

    #[error("文件已存在: {0}")]
    AlreadyExists(String),

    #[error("解析失败: {0}")]
    ParseFailed(String),

    #[error("操作繁忙，请稍后重试")]
    Busy,

    #[error("操作已回滚: {0}（原因: {1}）")]
    OperationRolledBack(String, String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, KbError>;
