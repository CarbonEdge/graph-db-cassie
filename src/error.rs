use thiserror::Error;

#[derive(Debug, Error)]
pub enum CassieError {
    #[error("ScyllaDB execution error: {0}")]
    Execution(#[from] scylla::errors::ExecutionError),

    #[error("ScyllaDB new session error: {0}")]
    NewSession(#[from] scylla::errors::NewSessionError),

    #[error("ScyllaDB rows result error: {0}")]
    RowsResult(#[from] scylla::errors::IntoRowsResultError),

    #[error("ScyllaDB row deserialization error: {0}")]
    RowDe(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type Result<T> = std::result::Result<T, CassieError>;
