use scylla::client::session::Session;

use crate::error::Result;

const CREATE_KEYSPACE: &str = r#"
    CREATE KEYSPACE IF NOT EXISTS cassie
    WITH replication = {
        'class': 'SimpleStrategy',
        'replication_factor': 1
    }
"#;

const CREATE_VERTICES: &str = r#"
    CREATE TABLE IF NOT EXISTS cassie.vertices (
        vertex_id  UUID,
        user_id    TEXT,
        doc_id     TEXT,
        vtype      TEXT,
        title      TEXT,
        summary    TEXT,
        content    TEXT,
        start_idx  INT,
        end_idx    INT,
        node_id    TEXT,
        properties MAP<TEXT, TEXT>,
        created_at TIMESTAMP,
        PRIMARY KEY (vertex_id)
    )
"#;

const CREATE_EDGES_OUT: &str = r#"
    CREATE TABLE IF NOT EXISTS cassie.edges_out (
        from_id    UUID,
        label      TEXT,
        to_id      UUID,
        PRIMARY KEY ((from_id, label), to_id)
    )
"#;

const CREATE_EDGES_IN: &str = r#"
    CREATE TABLE IF NOT EXISTS cassie.edges_in (
        to_id      UUID,
        label      TEXT,
        from_id    UUID,
        PRIMARY KEY ((to_id, label), from_id)
    )
"#;

const CREATE_DOCUMENTS: &str = r#"
    CREATE TABLE IF NOT EXISTS cassie.documents (
        user_id      TEXT,
        created_at   TIMESTAMP,
        doc_id       TEXT,
        root_id      UUID,
        filename     TEXT,
        doc_type     TEXT,
        description  TEXT,
        total_pages  INT,
        raw_content  TEXT,
        config_json  TEXT,
        PRIMARY KEY ((user_id), created_at, doc_id)
    ) WITH CLUSTERING ORDER BY (created_at DESC, doc_id ASC)
"#;

const CREATE_SEARCH_TOKENS: &str = r#"
    CREATE TABLE IF NOT EXISTS cassie.search_tokens (
        user_id    TEXT,
        word       TEXT,
        vertex_id  UUID,
        doc_id     TEXT,
        title      TEXT,
        summary    TEXT,
        start_idx  INT,
        end_idx    INT,
        node_id    TEXT,
        PRIMARY KEY ((user_id, word), vertex_id)
    )
"#;

/// Lookup table: (user_id, doc_id) → all vertex_ids for that document.
/// Eliminates ALLOW FILTERING on the vertices table.
const CREATE_DOC_VERTICES: &str = r#"
    CREATE TABLE IF NOT EXISTS cassie.doc_vertices (
        user_id   TEXT,
        doc_id    TEXT,
        vertex_id UUID,
        PRIMARY KEY ((user_id, doc_id), vertex_id)
    )
"#;

/// Lookup table: (user_id, doc_id) → created_at timestamp.
/// Enables O(1) lookup of the document primary key without ALLOW FILTERING.
const CREATE_DOC_LOOKUP: &str = r#"
    CREATE TABLE IF NOT EXISTS cassie.doc_lookup (
        user_id    TEXT,
        doc_id     TEXT,
        created_at TIMESTAMP,
        PRIMARY KEY ((user_id, doc_id))
    )
"#;

/// Create keyspace and all tables. Safe to call multiple times (IF NOT EXISTS).
pub async fn setup_schema(session: &Session) -> Result<()> {
    session.query_unpaged(CREATE_KEYSPACE, &[]).await?;
    session.query_unpaged(CREATE_VERTICES, &[]).await?;
    session.query_unpaged(CREATE_EDGES_OUT, &[]).await?;
    session.query_unpaged(CREATE_EDGES_IN, &[]).await?;
    session.query_unpaged(CREATE_DOCUMENTS, &[]).await?;
    session.query_unpaged(CREATE_SEARCH_TOKENS, &[]).await?;
    session.query_unpaged(CREATE_DOC_VERTICES, &[]).await?;
    session.query_unpaged(CREATE_DOC_LOOKUP, &[]).await?;
    // Best-effort: adds node_id to existing search_tokens tables (no-op if already present)
    let _ = session
        .query_unpaged("ALTER TABLE cassie.search_tokens ADD node_id TEXT", &[])
        .await;
    Ok(())
}
