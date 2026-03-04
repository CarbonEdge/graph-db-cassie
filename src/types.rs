use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Re-export the DocumentIndex types so users of this crate don't need to import
// the mediman-rust-chat types separately. For standalone use we define them here.

/// Type of document
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DocType {
    Pdf,
    Markdown,
}

impl std::fmt::Display for DocType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocType::Pdf => write!(f, "pdf"),
            DocType::Markdown => write!(f, "markdown"),
        }
    }
}

impl std::str::FromStr for DocType {
    type Err = crate::error::CassieError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pdf" => Ok(DocType::Pdf),
            "markdown" => Ok(DocType::Markdown),
            other => Err(crate::error::CassieError::InvalidData(format!(
                "Unknown doc_type: {other}"
            ))),
        }
    }
}

/// Configuration for index building
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    pub max_pages_per_node: u32,
    pub max_tokens_per_window: usize,
    pub window_overlap: usize,
    pub min_tokens_per_node: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            max_pages_per_node: 10,
            max_tokens_per_window: 3000,
            window_overlap: 1,
            min_tokens_per_node: 50,
        }
    }
}

/// A node in the hierarchical document tree
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TreeNode {
    pub title: String,
    pub node_id: String,
    pub start_index: u32,
    pub end_index: u32,
    pub summary: Option<String>,
    pub nodes: Vec<TreeNode>,
}

/// The persisted document index (mirrors mediman-rust-chat DocumentIndex)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentIndex {
    pub doc_id: String,
    pub user_id: String,
    pub filename: String,
    pub doc_type: DocType,
    pub description: Option<String>,
    pub total_pages: u32,
    pub tree: TreeNode,
    pub raw_content: Option<String>,
    pub config: IndexConfig,
    pub created_at: DateTime<Utc>,
}

// ─── Graph types ─────────────────────────────────────────────────────────────

/// Vertex type in the property graph
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VertexType {
    Document,
    Section,
    Leaf,
}

impl std::fmt::Display for VertexType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VertexType::Document => write!(f, "document"),
            VertexType::Section => write!(f, "section"),
            VertexType::Leaf => write!(f, "leaf"),
        }
    }
}

impl std::str::FromStr for VertexType {
    type Err = crate::error::CassieError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "document" => Ok(VertexType::Document),
            "section" => Ok(VertexType::Section),
            "leaf" => Ok(VertexType::Leaf),
            other => Err(crate::error::CassieError::InvalidData(format!(
                "Unknown vertex type: {other}"
            ))),
        }
    }
}

/// A vertex in the property graph (maps to one TreeNode)
#[derive(Debug, Clone)]
pub struct Vertex {
    pub vertex_id: Uuid,
    pub user_id: String,
    pub doc_id: String,
    pub vtype: VertexType,
    pub title: String,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub start_idx: u32,
    pub end_idx: u32,
    /// Original TreeNode.node_id ("0001", "1.2.3", …)
    pub node_id: String,
    pub properties: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
}

/// A directed edge between two vertices
#[derive(Debug, Clone)]
pub struct Edge {
    pub from_id: Uuid,
    pub label: String,
    pub to_id: Uuid,
}

/// Search result for dirty token search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub vertex_id: Uuid,
    pub doc_id: String,
    pub title: String,
    pub summary: Option<String>,
    /// Number of query tokens matched
    pub score: u32,
    pub start_idx: u32,
    pub end_idx: u32,
}

/// Configuration for CassieClient
#[derive(Debug, Clone)]
pub struct CassieConfig {
    /// ScyllaDB/Cassandra contact points, e.g. ["127.0.0.1:9042"]
    pub contact_points: Vec<String>,
    /// Keyspace name (will be created if it doesn't exist)
    pub keyspace: String,
}

impl Default for CassieConfig {
    fn default() -> Self {
        Self {
            contact_points: vec!["127.0.0.1:9042".to_string()],
            keyspace: "cassie".to_string(),
        }
    }
}
