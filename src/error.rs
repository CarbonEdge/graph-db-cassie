use thiserror::Error;

#[derive(Debug, Error)]
pub enum CassieError {
    #[error("ScyllaDB execution error: {0}")]
    Execution(Box<scylla::errors::ExecutionError>),

    #[error("ScyllaDB new session error: {0}")]
    NewSession(Box<scylla::errors::NewSessionError>),

    #[error("ScyllaDB prepare error: {0}")]
    Prepare(Box<scylla::errors::PrepareError>),

    #[error("ScyllaDB rows result error: {0}")]
    RowsResult(Box<scylla::errors::IntoRowsResultError>),

    #[error("ScyllaDB row deserialization error: {0}")]
    RowDe(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

impl From<scylla::errors::ExecutionError> for CassieError {
    fn from(e: scylla::errors::ExecutionError) -> Self {
        CassieError::Execution(Box::new(e))
    }
}

impl From<scylla::errors::NewSessionError> for CassieError {
    fn from(e: scylla::errors::NewSessionError) -> Self {
        CassieError::NewSession(Box::new(e))
    }
}

impl From<scylla::errors::PrepareError> for CassieError {
    fn from(e: scylla::errors::PrepareError) -> Self {
        CassieError::Prepare(Box::new(e))
    }
}

impl From<scylla::errors::IntoRowsResultError> for CassieError {
    fn from(e: scylla::errors::IntoRowsResultError) -> Self {
        CassieError::RowsResult(Box::new(e))
    }
}

pub type Result<T> = std::result::Result<T, CassieError>;
