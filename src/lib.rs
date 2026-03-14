//! **Cassie** — ScyllaDB-backed property graph store for document indexes.
//!
//! Drop-in replacement for the Sled-backed `PageIndexStore`, with additional
//! graph traversal and dirty-token search capabilities.
//!
//! # Quick start
//!
//! ```no_run
//! use graph_db_cassie::{CassieClient, CassieConfig};
//!
//! #[tokio::main]
//! async fn main() -> graph_db_cassie::error::Result<()> {
//!     let client = CassieClient::new(CassieConfig::default()).await?;
//!     client.setup_schema().await?;
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod error;
pub mod graph;
pub mod prepared;
pub mod schema;
pub mod search;
pub mod store;
pub mod types;

pub use client::CassieClient;
pub use error::{CassieError, Result};
pub use types::{
    CassieConfig, DocType, DocumentIndex, Edge, IndexConfig, SearchResult, TreeNode, Vertex,
    VertexType,
};
