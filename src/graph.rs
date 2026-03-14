use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use crate::{
    client::CassieClient,
    error::{CassieError, Result},
    types::{DocumentIndex, Edge, TreeNode, Vertex, VertexType},
};

const CONTAINS: &str = "CONTAINS";

// ─── Decompose ────────────────────────────────────────────────────────────────

/// Decompose a `DocumentIndex` tree into a flat list of vertices and edges.
/// Returns `(vertices, edges, root_vertex_id)`.
pub fn decompose(index: &DocumentIndex) -> (Vec<Vertex>, Vec<Edge>, Uuid) {
    let mut vertices = Vec::new();
    let mut edges = Vec::new();
    let root_id = Uuid::new_v4();

    decompose_node(
        &index.tree,
        root_id,
        None,
        &index.user_id,
        &index.doc_id,
        true,
        &mut vertices,
        &mut edges,
    );

    (vertices, edges, root_id)
}

#[allow(clippy::too_many_arguments)]
fn decompose_node(
    node: &TreeNode,
    vertex_id: Uuid,
    parent_id: Option<Uuid>,
    user_id: &str,
    doc_id: &str,
    is_root: bool,
    vertices: &mut Vec<Vertex>,
    edges: &mut Vec<Edge>,
) {
    let vtype = if is_root {
        VertexType::Document
    } else if node.nodes.is_empty() {
        VertexType::Leaf
    } else {
        VertexType::Section
    };

    let vertex = Vertex {
        vertex_id,
        user_id: user_id.to_string(),
        doc_id: doc_id.to_string(),
        vtype,
        title: node.title.clone(),
        summary: node.summary.clone(),
        content: None,
        start_idx: node.start_index,
        end_idx: node.end_index,
        node_id: node.node_id.clone(),
        properties: HashMap::new(),
        created_at: Utc::now(),
    };
    vertices.push(vertex);

    if let Some(pid) = parent_id {
        edges.push(Edge {
            from_id: pid,
            label: CONTAINS.to_string(),
            to_id: vertex_id,
        });
    }

    for child in &node.nodes {
        let child_id = Uuid::new_v4();
        decompose_node(
            child,
            child_id,
            Some(vertex_id),
            user_id,
            doc_id,
            false,
            vertices,
            edges,
        );
    }
}

// ─── Recompose ────────────────────────────────────────────────────────────────

/// Reconstruct a `TreeNode` tree from flat vertices + adjacency map.
pub fn recompose(
    root_id: Uuid,
    by_id: &HashMap<Uuid, &Vertex>,
    children_map: &HashMap<Uuid, Vec<Uuid>>,
) -> Result<TreeNode> {
    let root = by_id
        .get(&root_id)
        .ok_or_else(|| CassieError::NotFound(format!("Root vertex {root_id} not in map")))?;
    build_node(root, by_id, children_map)
}

fn build_node(
    v: &Vertex,
    by_id: &HashMap<Uuid, &Vertex>,
    children_map: &HashMap<Uuid, Vec<Uuid>>,
) -> Result<TreeNode> {
    let child_ids = children_map.get(&v.vertex_id).cloned().unwrap_or_default();
    let mut child_nodes = Vec::with_capacity(child_ids.len());
    for cid in &child_ids {
        let child_v = by_id
            .get(cid)
            .ok_or_else(|| CassieError::NotFound(format!("Vertex {cid} not found")))?;
        child_nodes.push(build_node(child_v, by_id, children_map)?);
    }
    Ok(TreeNode {
        title: v.title.clone(),
        node_id: v.node_id.clone(),
        start_index: v.start_idx,
        end_index: v.end_idx,
        summary: v.summary.clone(),
        nodes: child_nodes,
    })
}

// ─── Graph traversal helpers ─────────────────────────────────────────────────

pub async fn get_children(client: &CassieClient, vertex_id: Uuid) -> Result<Vec<Vertex>> {
    let result = client
        .session
        .query_unpaged(
            "SELECT to_id FROM cassie.edges_out WHERE from_id = ? AND label = ?",
            (vertex_id, CONTAINS),
        )
        .await?;

    let rows_result = result.into_rows_result()?;
    let rows = rows_result
        .rows::<(Uuid,)>()
        .map_err(|e| CassieError::RowDe(e.to_string()))?;

    let mut children = Vec::new();
    for row in rows {
        let (child_id,) = row.map_err(|e| CassieError::RowDe(e.to_string()))?;
        if let Some(v) = fetch_vertex(client, child_id).await? {
            children.push(v);
        }
    }
    Ok(children)
}

pub async fn get_ancestors(client: &CassieClient, vertex_id: Uuid) -> Result<Vec<Vertex>> {
    let mut ancestors = Vec::new();
    let mut current = vertex_id;

    loop {
        let result = client
            .session
            .query_unpaged(
                "SELECT from_id FROM cassie.edges_in WHERE to_id = ? AND label = ?",
                (current, CONTAINS),
            )
            .await?;

        let rows_result = result.into_rows_result()?;
        let mut rows = rows_result
            .rows::<(Uuid,)>()
            .map_err(|e| CassieError::RowDe(e.to_string()))?;

        let parent_id = if let Some(row) = rows.next() {
            let (pid,) = row.map_err(|e| CassieError::RowDe(e.to_string()))?;
            Some(pid)
        } else {
            None
        };

        match parent_id {
            None => break,
            Some(pid) => {
                if let Some(v) = fetch_vertex(client, pid).await? {
                    ancestors.push(v);
                }
                current = pid;
            }
        }
    }

    Ok(ancestors)
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

type VertexRow = (
    Uuid,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i32,
    i32,
    String,
    Option<HashMap<String, String>>,
    Option<scylla::value::CqlTimestamp>,
);

fn row_to_vertex(row: VertexRow) -> Result<Vertex> {
    use std::str::FromStr;
    let (
        vid,
        user_id,
        doc_id,
        vtype_str,
        title,
        summary,
        content,
        start_idx,
        end_idx,
        node_id,
        properties,
        created_at_raw,
    ) = row;

    let vtype = VertexType::from_str(&vtype_str)?;
    let created_at = created_at_raw
        .and_then(|ts| chrono::DateTime::from_timestamp_millis(ts.0))
        .unwrap_or_else(Utc::now);

    Ok(Vertex {
        vertex_id: vid,
        user_id,
        doc_id,
        vtype,
        title,
        summary,
        content,
        start_idx: start_idx as u32,
        end_idx: end_idx as u32,
        node_id,
        properties: properties.unwrap_or_default(),
        created_at,
    })
}

const VERTEX_SELECT: &str = "SELECT vertex_id, user_id, doc_id, vtype, title, summary, content, \
     start_idx, end_idx, node_id, properties, created_at \
     FROM cassie.vertices";

pub(crate) async fn fetch_vertex(client: &CassieClient, vertex_id: Uuid) -> Result<Option<Vertex>> {
    let result = client
        .session
        .query_unpaged(
            format!("{VERTEX_SELECT} WHERE vertex_id = ?").as_str(),
            (vertex_id,),
        )
        .await?;

    let rows_result = result.into_rows_result()?;
    let mut rows = rows_result
        .rows::<VertexRow>()
        .map_err(|e| CassieError::RowDe(e.to_string()))?;

    match rows.next() {
        None => Ok(None),
        Some(row) => {
            let r = row.map_err(|e| CassieError::RowDe(e.to_string()))?;
            Ok(Some(row_to_vertex(r)?))
        }
    }
}

pub(crate) async fn fetch_all_vertices_for_doc(
    client: &CassieClient,
    user_id: &str,
    doc_id: &str,
) -> Result<Vec<Vertex>> {
    use futures::future::try_join_all;

    // Step 1: get all vertex IDs from the lookup table (O(1) partition scan)
    let result = client
        .session
        .query_unpaged(
            "SELECT vertex_id FROM cassie.doc_vertices WHERE user_id = ? AND doc_id = ?",
            (user_id, doc_id),
        )
        .await?;

    let rows_result = result.into_rows_result()?;
    let vertex_ids: Vec<Uuid> = rows_result
        .rows::<(Uuid,)>()
        .map_err(|e| CassieError::RowDe(e.to_string()))?
        .map(|r| {
            r.map(|(vid,)| vid)
                .map_err(|e| CassieError::RowDe(e.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;

    // Step 2: fetch all vertices concurrently by their partition key (no filtering)
    let futs: Vec<_> = vertex_ids
        .iter()
        .map(|&vid| fetch_vertex(client, vid))
        .collect();
    let results = try_join_all(futs).await?;
    Ok(results.into_iter().flatten().collect())
}

pub(crate) async fn fetch_all_edges_for_doc(
    client: &CassieClient,
    doc_vertex_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Uuid>>> {
    use futures::future::try_join_all;

    let futs: Vec<_> = doc_vertex_ids
        .iter()
        .map(|&from_id| async move {
            let result = client
                .session
                .query_unpaged(
                    "SELECT to_id FROM cassie.edges_out WHERE from_id = ? AND label = ?",
                    (from_id, CONTAINS),
                )
                .await?;

            let rows_result = result.into_rows_result()?;
            let rows = rows_result
                .rows::<(Uuid,)>()
                .map_err(|e| CassieError::RowDe(e.to_string()))?;

            let mut children = Vec::new();
            for row in rows {
                let (child_id,) = row.map_err(|e| CassieError::RowDe(e.to_string()))?;
                children.push(child_id);
            }
            Ok::<(Uuid, Vec<Uuid>), CassieError>((from_id, children))
        })
        .collect();

    let pairs = try_join_all(futs).await?;
    let mut children_map: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (from_id, children) in pairs {
        children_map.insert(from_id, children);
    }
    Ok(children_map)
}
