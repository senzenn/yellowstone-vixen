use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(String),

    #[error("token: {0}")]
    Token(String),

    #[error("grpc: {0}")]
    Grpc(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
