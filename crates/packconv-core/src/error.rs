use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unsupported archive format (expected ZIP or RAR)")]
    UnknownArchive,
    #[error("pack.mcmeta not found in archive")]
    MissingPackMeta,
    #[error("archive limit exceeded: {0}")]
    LimitExceeded(&'static str),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("rar error: {0}")]
    Rar(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid pack.mcmeta: {0}")]
    PackMeta(String),
    #[error("invalid JSON in {0}")]
    Json(String),
    #[error("invalid image {0}")]
    Image(String),
    #[error("unknown or unsupported version: {0}")]
    UnknownVersion(String),
    #[error("conversion cancelled (time limit exceeded)")]
    Cancelled,
    #[error("cannot detect source version, pass it explicitly")]
    UndetectedVersion,
}

pub type Result<T> = std::result::Result<T, Error>;
