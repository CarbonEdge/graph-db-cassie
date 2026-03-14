use std::sync::Arc;

use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;

use crate::{
    error::Result,
    prepared::Prepared,
    schema,
    types::{CassieConfig, DocumentIndex, SearchResult, Vertex},
};

/// The main entry point for interacting with the Cassie graph store.
///
/// Wraps a ScyllaDB session and provides a drop-in replacement API for
/// the Sled-backed `PageIndexStore`, plus additional graph/search methods.
#[derive(Clone)]
pub struct CassieClient {
    pub(crate) session: Arc<Session>,
    pub(crate) prepared: Arc<Prepared>,
    #[allow(dead_code)]
    pub(crate) keyspace: String,
}

impl CassieClient {
    /// Connect to ScyllaDB, run schema migrations, and prepare all hot-path
    /// statements in parallel.  Returns a fully ready client.
    pub async fn new(config: CassieConfig) -> Result<Self> {
        let session = SessionBuilder::new()
            .known_nodes(&config.contact_points)
            .build()
            .await?;

        schema::setup_schema(&session).await?;
        let prepared = Prepared::new(&session).await?;

        Ok(Self {
            session: Arc::new(session),
            keyspace: config.keyspace,
            prepared: Arc::new(prepared),
        })
    }

    /// No-op kept for backward compatibility — schema is now set up in `new()`.
    pub async fn setup_schema(&self) -> Result<()> {
        Ok(())
    }

    // ─── Drop-in PageIndexStore API ──────────────────────────────────────────

    /// Persist a `DocumentIndex`, decomposing its tree into graph vertices/edges.
    pub async fn save(&self, index: &DocumentIndex) -> Result<()> {
        crate::store::save(self, index).await
    }

    /// Load a `DocumentIndex` by reconstructing its tree from the graph.
    pub async fn load(&self, user_id: &str, doc_id: &str) -> Result<DocumentIndex> {
        crate::store::load(self, user_id, doc_id).await
    }

    /// List all `DocumentIndex` objects for a user (metadata only, no tree).
    pub async fn list(&self, user_id: &str) -> Result<Vec<DocumentIndex>> {
        crate::store::list(self, user_id).await
    }

    /// Delete all data for a (user_id, doc_id) pair.
    pub async fn delete(&self, user_id: &str, doc_id: &str) -> Result<()> {
        crate::store::delete(self, user_id, doc_id).await
    }

    // ─── Graph API ───────────────────────────────────────────────────────────

    /// Return all direct child vertices of a vertex (CONTAINS edges).
    pub async fn get_children(&self, vertex_id: uuid::Uuid) -> Result<Vec<Vertex>> {
        crate::graph::get_children(self, vertex_id).await
    }

    /// Return all ancestor vertices up to the document root.
    pub async fn get_ancestors(&self, vertex_id: uuid::Uuid) -> Result<Vec<Vertex>> {
        crate::graph::get_ancestors(self, vertex_id).await
    }

    // ─── Diagnostics ─────────────────────────────────────────────────────────

    /// Run a trivial query to verify the session is alive.
    pub async fn ping(&self) -> Result<()> {
        self.session
            .query_unpaged("SELECT now() FROM system.local", ())
            .await?;
        Ok(())
    }

    // ─── Search API ──────────────────────────────────────────────────────────

    /// Dirty token search: split query into tokens, union match vertices, score
    /// by token hit count, return top-K results.
    pub async fn search(
        &self,
        user_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        crate::search::search(self, user_id, query, top_k).await
    }
}
