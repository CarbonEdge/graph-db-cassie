use std::collections::HashMap;

use scylla::value::CqlTimestamp;
use uuid::Uuid;

use crate::{
    client::CassieClient,
    error::{CassieError, Result},
    graph,
    types::{DocType, DocumentIndex, IndexConfig, TreeNode, Vertex},
};

const CONTAINS: &str = "CONTAINS";

// ─── Save ─────────────────────────────────────────────────────────────────────

/// Insert a single vertex into `cassie.vertices` and `cassie.doc_vertices`.
async fn save_vertex(client: &CassieClient, v: &Vertex) -> Result<()> {
    let created_ms = v.created_at.timestamp_millis();
    client
        .session
        .execute_unpaged(
            &client.prepared.insert_vertex,
            (
                v.vertex_id,
                &v.user_id,
                &v.doc_id,
                v.vtype.to_string(),
                &v.title,
                &v.summary,
                &v.content,
                v.start_idx as i32,
                v.end_idx as i32,
                &v.node_id,
                &v.properties,
                CqlTimestamp(created_ms),
            ),
        )
        .await?;
    client
        .session
        .execute_unpaged(
            &client.prepared.insert_doc_vertex,
            (&v.user_id, &v.doc_id, v.vertex_id),
        )
        .await?;
    Ok(())
}

pub async fn save(client: &CassieClient, index: &DocumentIndex) -> Result<()> {
    use futures::future::try_join_all;

    let (vertices, edges, root_id) = graph::decompose(index);

    // 1. Insert all vertices concurrently (each also writes to doc_vertices)
    let vertex_futs: Vec<_> = vertices.iter().map(|v| save_vertex(client, v)).collect();
    try_join_all(vertex_futs).await?;

    // 2. Insert edges (both directions)
    for e in &edges {
        client
            .session
            .execute_unpaged(
                &client.prepared.insert_edge_out,
                (e.from_id, &e.label, e.to_id),
            )
            .await?;

        client
            .session
            .execute_unpaged(
                &client.prepared.insert_edge_in,
                (e.to_id, &e.label, e.from_id),
            )
            .await?;
    }

    // 3. Insert document record
    let created_ms = index.created_at.timestamp_millis();
    let config_json = serde_json::to_string(&index.config)?;
    client
        .session
        .execute_unpaged(
            &client.prepared.insert_document,
            (
                &index.user_id,
                CqlTimestamp(created_ms),
                &index.doc_id,
                root_id,
                &index.filename,
                index.doc_type.to_string(),
                &index.description,
                index.total_pages as i32,
                &index.raw_content,
                &config_json,
            ),
        )
        .await?;

    // 4. Write doc_lookup entry so fetch_document_row can find created_at by (user_id, doc_id)
    client
        .session
        .execute_unpaged(
            &client.prepared.insert_doc_lookup,
            (&index.user_id, &index.doc_id, CqlTimestamp(created_ms)),
        )
        .await?;

    // 5. Index tokens for search
    for v in &vertices {
        crate::search::index_vertex(client, v).await?;
    }

    Ok(())
}

// ─── Load ─────────────────────────────────────────────────────────────────────

pub async fn load(client: &CassieClient, user_id: &str, doc_id: &str) -> Result<DocumentIndex> {
    let doc_row = fetch_document_row(client, user_id, doc_id).await?;

    let vertices = graph::fetch_all_vertices_for_doc(client, user_id, doc_id).await?;
    if vertices.is_empty() {
        return Err(CassieError::NotFound(format!(
            "No vertices found for doc {doc_id}"
        )));
    }

    let all_ids: Vec<Uuid> = vertices.iter().map(|v| v.vertex_id).collect();
    let children_map = graph::fetch_all_edges_for_doc(client, &all_ids).await?;

    let by_id: HashMap<Uuid, &crate::types::Vertex> =
        vertices.iter().map(|v| (v.vertex_id, v)).collect();
    let tree = graph::recompose(doc_row.root_id, &by_id, &children_map)?;

    Ok(DocumentIndex {
        doc_id: doc_row.doc_id,
        user_id: doc_row.user_id,
        filename: doc_row.filename,
        doc_type: doc_row.doc_type,
        description: doc_row.description,
        total_pages: doc_row.total_pages,
        tree,
        raw_content: doc_row.raw_content,
        config: doc_row.config,
        created_at: doc_row.created_at,
    })
}

// ─── List ─────────────────────────────────────────────────────────────────────

pub async fn list(client: &CassieClient, user_id: &str) -> Result<Vec<DocumentIndex>> {
    use std::str::FromStr;

    let result = client
        .session
        .execute_unpaged(&client.prepared.select_documents_by_user, (user_id,))
        .await?;

    type DocRow = (
        String,
        Option<CqlTimestamp>,
        String,
        Uuid,
        String,
        String,
        Option<String>,
        i32,
        Option<String>,
        Option<String>,
    );

    let rows_result = result.into_rows_result()?;
    let rows = rows_result
        .rows::<DocRow>()
        .map_err(|e| CassieError::RowDe(e.to_string()))?;

    let mut docs = Vec::new();
    for row in rows {
        let (
            uid,
            created_at_raw,
            did,
            _root_id,
            filename,
            doc_type_str,
            description,
            total_pages,
            raw_content,
            config_json,
        ) = row.map_err(|e| CassieError::RowDe(e.to_string()))?;

        let doc_type = DocType::from_str(&doc_type_str)?;
        let config: IndexConfig = config_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        let created_at = created_at_raw
            .and_then(|ts| chrono::DateTime::from_timestamp_millis(ts.0))
            .unwrap_or_else(chrono::Utc::now);

        docs.push(DocumentIndex {
            doc_id: did,
            user_id: uid,
            filename,
            doc_type,
            description,
            total_pages: total_pages as u32,
            // list() returns documents without the full tree; callers use load() for that
            tree: TreeNode {
                title: String::new(),
                node_id: String::new(),
                start_index: 0,
                end_index: 0,
                summary: None,
                nodes: vec![],
            },
            raw_content,
            config,
            created_at,
        });
    }
    Ok(docs)
}

// ─── Delete ───────────────────────────────────────────────────────────────────

pub async fn delete(client: &CassieClient, user_id: &str, doc_id: &str) -> Result<()> {
    let doc_row = fetch_document_row(client, user_id, doc_id).await?;

    let vertices = graph::fetch_all_vertices_for_doc(client, user_id, doc_id).await?;
    let all_ids: Vec<Uuid> = vertices.iter().map(|v| v.vertex_id).collect();

    for v in &vertices {
        crate::search::delete_vertex_words(client, v).await?;
    }

    for &vid in &all_ids {
        client
            .session
            .execute_unpaged(&client.prepared.delete_edges_out, (vid, CONTAINS))
            .await?;
        client
            .session
            .execute_unpaged(&client.prepared.delete_edges_in, (vid, CONTAINS))
            .await?;
    }

    for &vid in &all_ids {
        client
            .session
            .execute_unpaged(&client.prepared.delete_vertex, (vid,))
            .await?;
    }

    let created_ms = doc_row.created_at.timestamp_millis();
    client
        .session
        .execute_unpaged(
            &client.prepared.delete_document,
            (user_id, CqlTimestamp(created_ms), doc_id),
        )
        .await?;

    // Remove the lookup table entries
    client
        .session
        .execute_unpaged(&client.prepared.delete_doc_vertices, (user_id, doc_id))
        .await?;
    client
        .session
        .execute_unpaged(&client.prepared.delete_doc_lookup, (user_id, doc_id))
        .await?;

    Ok(())
}

// ─── Internal helper ──────────────────────────────────────────────────────────

struct DocRow {
    doc_id: String,
    user_id: String,
    root_id: Uuid,
    filename: String,
    doc_type: DocType,
    description: Option<String>,
    total_pages: u32,
    raw_content: Option<String>,
    config: IndexConfig,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn fetch_document_row(client: &CassieClient, user_id: &str, doc_id: &str) -> Result<DocRow> {
    use std::str::FromStr;

    // Step 1: look up created_at from the O(1) lookup table
    let lookup = client
        .session
        .execute_unpaged(&client.prepared.select_doc_lookup, (user_id, doc_id))
        .await?;

    let lookup_rows = lookup.into_rows_result()?;
    let mut lookup_iter = lookup_rows
        .rows::<(Option<CqlTimestamp>,)>()
        .map_err(|e| CassieError::RowDe(e.to_string()))?;

    let created_at_raw = match lookup_iter.next() {
        None => {
            return Err(CassieError::NotFound(format!(
                "Document not found: {doc_id}"
            )))
        }
        Some(row) => {
            let (ts,) = row.map_err(|e| CassieError::RowDe(e.to_string()))?;
            ts
        }
    };

    // Step 2: fetch with full primary key — no ALLOW FILTERING needed
    type Row = (
        String,
        Option<CqlTimestamp>,
        String,
        Uuid,
        String,
        String,
        Option<String>,
        i32,
        Option<String>,
        Option<String>,
    );

    let result = client
        .session
        .execute_unpaged(
            &client.prepared.select_document_by_pk,
            (user_id, created_at_raw, doc_id),
        )
        .await?;

    let rows_result = result.into_rows_result()?;
    let mut rows = rows_result
        .rows::<Row>()
        .map_err(|e| CassieError::RowDe(e.to_string()))?;

    match rows.next() {
        None => Err(CassieError::NotFound(format!(
            "Document not found: {doc_id}"
        ))),
        Some(row) => {
            let (
                uid,
                created_at_raw2,
                did,
                root_id,
                filename,
                doc_type_str,
                description,
                total_pages,
                raw_content,
                config_json,
            ) = row.map_err(|e| CassieError::RowDe(e.to_string()))?;

            let doc_type = DocType::from_str(&doc_type_str)?;
            let config: IndexConfig = config_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_default();
            let created_at = created_at_raw2
                .and_then(|ts| chrono::DateTime::from_timestamp_millis(ts.0))
                .unwrap_or_else(chrono::Utc::now);

            Ok(DocRow {
                doc_id: did,
                user_id: uid,
                root_id,
                filename,
                doc_type,
                description,
                total_pages: total_pages as u32,
                raw_content,
                config,
                created_at,
            })
        }
    }
}
