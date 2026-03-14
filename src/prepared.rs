use scylla::client::session::Session;
use scylla::statement::prepared::PreparedStatement;

use crate::error::Result;

/// All hot-path CQL statements, prepared once at startup.
///
/// Build with [`Prepared::new`], which fires every `session.prepare()` call in
/// parallel via `tokio::try_join!` so startup latency is bounded by the single
/// slowest round-trip rather than the sum.
pub struct Prepared {
    // store.rs
    pub insert_vertex: PreparedStatement,
    pub insert_doc_vertex: PreparedStatement,
    pub insert_doc_lookup: PreparedStatement,
    pub insert_document: PreparedStatement,
    pub select_doc_lookup: PreparedStatement,
    pub select_document_by_pk: PreparedStatement,
    pub select_documents_by_user: PreparedStatement,
    pub delete_doc_vertices: PreparedStatement,
    pub delete_doc_lookup: PreparedStatement,
    pub delete_document: PreparedStatement,
    // graph.rs
    pub select_doc_vertex_ids: PreparedStatement,
    pub select_vertex: PreparedStatement,
    pub insert_edge_out: PreparedStatement,
    pub insert_edge_in: PreparedStatement,
    pub select_edges_out: PreparedStatement,
    pub select_edges_in: PreparedStatement,
    pub delete_edges_out: PreparedStatement,
    pub delete_edges_in: PreparedStatement,
    pub delete_vertex: PreparedStatement,
    // search.rs
    pub insert_search_token: PreparedStatement,
    pub select_search_tokens: PreparedStatement,
    pub delete_search_token: PreparedStatement,
}

impl Prepared {
    pub async fn new(session: &Session) -> Result<Self> {
        let (
            insert_vertex,
            insert_doc_vertex,
            insert_doc_lookup,
            insert_document,
            select_doc_lookup,
            select_document_by_pk,
            select_documents_by_user,
            delete_doc_vertices,
            delete_doc_lookup,
            delete_document,
            select_doc_vertex_ids,
            select_vertex,
            insert_edge_out,
            insert_edge_in,
            select_edges_out,
            select_edges_in,
            delete_edges_out,
            delete_edges_in,
            delete_vertex,
            insert_search_token,
            select_search_tokens,
            delete_search_token,
        ) = tokio::try_join!(
            session.prepare(
                "INSERT INTO cassie.vertices \
                 (vertex_id, user_id, doc_id, vtype, title, summary, content, \
                  start_idx, end_idx, node_id, properties, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            session.prepare(
                "INSERT INTO cassie.doc_vertices \
                 (user_id, doc_id, vertex_id) VALUES (?, ?, ?)"
            ),
            session.prepare(
                "INSERT INTO cassie.doc_lookup \
                 (user_id, doc_id, created_at) VALUES (?, ?, ?)"
            ),
            session.prepare(
                "INSERT INTO cassie.documents \
                 (user_id, created_at, doc_id, root_id, filename, doc_type, \
                  description, total_pages, raw_content, config_json) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            session.prepare(
                "SELECT created_at FROM cassie.doc_lookup \
                 WHERE user_id = ? AND doc_id = ?"
            ),
            session.prepare(
                "SELECT user_id, created_at, doc_id, root_id, filename, doc_type, \
                 description, total_pages, raw_content, config_json \
                 FROM cassie.documents WHERE user_id = ? AND created_at = ? AND doc_id = ?"
            ),
            session.prepare(
                "SELECT user_id, created_at, doc_id, root_id, filename, doc_type, \
                 description, total_pages, raw_content, config_json \
                 FROM cassie.documents WHERE user_id = ?"
            ),
            session.prepare(
                "DELETE FROM cassie.doc_vertices \
                 WHERE user_id = ? AND doc_id = ?"
            ),
            session.prepare(
                "DELETE FROM cassie.doc_lookup \
                 WHERE user_id = ? AND doc_id = ?"
            ),
            session.prepare(
                "DELETE FROM cassie.documents \
                 WHERE user_id = ? AND created_at = ? AND doc_id = ?"
            ),
            session.prepare(
                "SELECT vertex_id FROM cassie.doc_vertices \
                 WHERE user_id = ? AND doc_id = ?"
            ),
            session.prepare(
                "SELECT vertex_id, user_id, doc_id, vtype, title, summary, content, \
                 start_idx, end_idx, node_id, properties, created_at \
                 FROM cassie.vertices WHERE vertex_id = ?"
            ),
            session.prepare(
                "INSERT INTO cassie.edges_out \
                 (from_id, label, to_id) VALUES (?, ?, ?)"
            ),
            session.prepare(
                "INSERT INTO cassie.edges_in \
                 (to_id, label, from_id) VALUES (?, ?, ?)"
            ),
            session.prepare(
                "SELECT to_id FROM cassie.edges_out \
                 WHERE from_id = ? AND label = ?"
            ),
            session.prepare(
                "SELECT from_id FROM cassie.edges_in \
                 WHERE to_id = ? AND label = ?"
            ),
            session.prepare(
                "DELETE FROM cassie.edges_out \
                 WHERE from_id = ? AND label = ?"
            ),
            session.prepare(
                "DELETE FROM cassie.edges_in \
                 WHERE to_id = ? AND label = ?"
            ),
            session.prepare("DELETE FROM cassie.vertices WHERE vertex_id = ?"),
            session.prepare(
                "INSERT INTO cassie.search_tokens \
                 (user_id, word, vertex_id, doc_id, title, summary, start_idx, end_idx, node_id) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            session.prepare(
                "SELECT vertex_id, doc_id, title, summary, start_idx, end_idx, node_id \
                 FROM cassie.search_tokens WHERE user_id = ? AND word = ?"
            ),
            session.prepare(
                "DELETE FROM cassie.search_tokens \
                 WHERE user_id = ? AND word = ? AND vertex_id = ?"
            ),
        )?;

        Ok(Self {
            insert_vertex,
            insert_doc_vertex,
            insert_doc_lookup,
            insert_document,
            select_doc_lookup,
            select_document_by_pk,
            select_documents_by_user,
            delete_doc_vertices,
            delete_doc_lookup,
            delete_document,
            select_doc_vertex_ids,
            select_vertex,
            insert_edge_out,
            insert_edge_in,
            select_edges_out,
            select_edges_in,
            delete_edges_out,
            delete_edges_in,
            delete_vertex,
            insert_search_token,
            select_search_tokens,
            delete_search_token,
        })
    }
}
