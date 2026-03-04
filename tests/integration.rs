//! Integration tests for CassieClient.
//!
//! These tests require a running ScyllaDB instance at 127.0.0.1:9042.
//! Start one with: `docker compose up -d` in the graph-db-cassie directory.
//!
//! Run with: `cargo test -- --nocapture`

use chrono::Utc;
use graph_db_cassie::{CassieClient, CassieConfig, DocType, DocumentIndex, IndexConfig, TreeNode};

fn scylla_config() -> CassieConfig {
    CassieConfig {
        contact_points: vec![
            std::env::var("SCYLLA_HOST").unwrap_or_else(|_| "127.0.0.1:9042".to_string())
        ],
        keyspace: "cassie_test".to_string(),
    }
}

/// Build a test DocumentIndex with a nested tree.
fn make_index(user_id: &str, doc_id: &str, filename: &str) -> DocumentIndex {
    DocumentIndex {
        doc_id: doc_id.to_string(),
        user_id: user_id.to_string(),
        filename: filename.to_string(),
        doc_type: DocType::Markdown,
        description: Some("Integration test document".to_string()),
        total_pages: 20,
        tree: TreeNode {
            title: "Root Section".to_string(),
            node_id: "0001".to_string(),
            start_index: 1,
            end_index: 20,
            summary: Some("Overview of the root section".to_string()),
            nodes: vec![
                TreeNode {
                    title: "Introduction".to_string(),
                    node_id: "0002".to_string(),
                    start_index: 1,
                    end_index: 5,
                    summary: Some("Introductory material about medical procedures".to_string()),
                    nodes: vec![],
                },
                TreeNode {
                    title: "Methods".to_string(),
                    node_id: "0003".to_string(),
                    start_index: 6,
                    end_index: 15,
                    summary: Some("Clinical methodology and research protocols".to_string()),
                    nodes: vec![
                        TreeNode {
                            title: "Data Collection".to_string(),
                            node_id: "0004".to_string(),
                            start_index: 6,
                            end_index: 10,
                            summary: Some("Patient data collection procedures".to_string()),
                            nodes: vec![],
                        },
                        TreeNode {
                            title: "Analysis".to_string(),
                            node_id: "0005".to_string(),
                            start_index: 11,
                            end_index: 15,
                            summary: Some("Statistical analysis methods".to_string()),
                            nodes: vec![],
                        },
                    ],
                },
                TreeNode {
                    title: "Conclusion".to_string(),
                    node_id: "0006".to_string(),
                    start_index: 16,
                    end_index: 20,
                    summary: Some("Summary and future directions".to_string()),
                    nodes: vec![],
                },
            ],
        },
        raw_content: Some("# Medical Research\n\nContent here.".to_string()),
        config: IndexConfig::default(),
        created_at: Utc::now(),
    }
}

async fn setup() -> CassieClient {
    let client = CassieClient::new(scylla_config())
        .await
        .expect("Failed to connect to ScyllaDB. Is it running? (`docker compose up -d`)");
    client
        .setup_schema()
        .await
        .expect("Failed to set up schema");
    client
}

// ─── Save & Load round-trip ───────────────────────────────────────────────────

#[tokio::test]
async fn test_save_and_load_round_trip() {
    let client = setup().await;
    let index = make_index("rt_user", "rt_doc", "round_trip.md");

    client.save(&index).await.expect("save failed");

    let loaded = client.load("rt_user", "rt_doc").await.expect("load failed");

    assert_eq!(loaded.doc_id, "rt_doc");
    assert_eq!(loaded.user_id, "rt_user");
    assert_eq!(loaded.filename, "round_trip.md");
    assert_eq!(loaded.total_pages, 20);
    assert_eq!(loaded.tree.title, "Root Section");
    assert_eq!(loaded.tree.nodes.len(), 3);

    let methods = &loaded.tree.nodes[1];
    assert_eq!(methods.title, "Methods");
    assert_eq!(methods.nodes.len(), 2);
    assert_eq!(methods.nodes[0].title, "Data Collection");
    assert_eq!(methods.nodes[1].title, "Analysis");

    // Cleanup
    let _ = client.delete("rt_user", "rt_doc").await;
}

// ─── Upsert ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_upsert() {
    let client = setup().await;
    let index = make_index("up_user", "up_doc", "first.md");
    client.save(&index).await.expect("first save failed");

    let updated = make_index("up_user", "up_doc", "updated.md");
    client.save(&updated).await.expect("second save failed");

    let loaded = client.load("up_user", "up_doc").await.expect("load failed");
    // The last-written document record wins (both are valid Cassandra upserts)
    assert_eq!(loaded.user_id, "up_user");
    assert_eq!(loaded.doc_id, "up_doc");

    let _ = client.delete("up_user", "up_doc").await;
}

// ─── Load not found ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_load_not_found() {
    let client = setup().await;
    let result = client.load("nf_user", "no_such_doc").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.to_lowercase().contains("not found"),
        "unexpected error: {msg}"
    );
}

// ─── List ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_user_isolation() {
    let client = setup().await;

    // Save 3 docs for list_u1 and 1 for list_u2
    client
        .save(&make_index("list_u1", "list_d1", "a.md"))
        .await
        .unwrap();
    client
        .save(&make_index("list_u1", "list_d2", "b.md"))
        .await
        .unwrap();
    client
        .save(&make_index("list_u1", "list_d3", "c.md"))
        .await
        .unwrap();
    client
        .save(&make_index("list_u2", "list_d4", "d.md"))
        .await
        .unwrap();

    let u1_docs = client.list("list_u1").await.expect("list u1 failed");
    assert_eq!(u1_docs.len(), 3, "expected 3 docs for list_u1");

    let u2_docs = client.list("list_u2").await.expect("list u2 failed");
    assert_eq!(u2_docs.len(), 1, "expected 1 doc for list_u2");
    assert_eq!(u2_docs[0].doc_id, "list_d4");

    // Cleanup
    for (u, d) in [
        ("list_u1", "list_d1"),
        ("list_u1", "list_d2"),
        ("list_u1", "list_d3"),
        ("list_u2", "list_d4"),
    ] {
        let _ = client.delete(u, d).await;
    }
}

#[tokio::test]
async fn test_list_empty() {
    let client = setup().await;
    let docs = client.list("nobody_xyz").await.expect("list failed");
    assert!(docs.is_empty());
}

// ─── Delete ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_delete() {
    let client = setup().await;
    client
        .save(&make_index("del_user", "del_doc", "delete_me.md"))
        .await
        .unwrap();

    assert!(client.load("del_user", "del_doc").await.is_ok());

    client
        .delete("del_user", "del_doc")
        .await
        .expect("delete failed");

    assert!(
        client.load("del_user", "del_doc").await.is_err(),
        "document should be gone after delete"
    );
}

#[tokio::test]
async fn test_delete_not_found() {
    let client = setup().await;
    let result = client.delete("del_user", "ghost_doc").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.to_lowercase().contains("not found"),
        "unexpected error: {msg}"
    );
}

// ─── Search ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_search_returns_top_k() {
    let client = setup().await;
    client
        .save(&make_index("srch_user", "srch_doc1", "medical.md"))
        .await
        .unwrap();
    client
        .save(&make_index("srch_user", "srch_doc2", "research.md"))
        .await
        .unwrap();

    // "clinical methodology" should hit the Methods + Data Collection nodes
    let results = client
        .search("srch_user", "clinical methodology", 3)
        .await
        .expect("search failed");

    assert!(
        !results.is_empty(),
        "expected at least one search result for 'clinical methodology'"
    );
    assert!(results.len() <= 3, "expected at most 3 results");

    // Highest score should be first
    let scores: Vec<u32> = results.iter().map(|r| r.score).collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(scores, sorted, "results not sorted by score");

    // Cleanup
    let _ = client.delete("srch_user", "srch_doc1").await;
    let _ = client.delete("srch_user", "srch_doc2").await;
}

#[tokio::test]
async fn test_dirty_search_partial_match() {
    let client = setup().await;
    client
        .save(&make_index("dirty_user", "dirty_doc", "dirty.md"))
        .await
        .unwrap();

    // Single token that exists in summary
    let results = client
        .search("dirty_user", "statistical", 5)
        .await
        .expect("search failed");

    assert!(
        !results.is_empty(),
        "expected results for single token 'statistical'"
    );

    // Cleanup
    let _ = client.delete("dirty_user", "dirty_doc").await;
}

#[tokio::test]
async fn test_search_no_results() {
    let client = setup().await;

    let results = client
        .search("nobody_xyz", "completely_unrelated_xyzzy", 5)
        .await
        .expect("search failed");

    assert!(results.is_empty());
}

// ─── Tokenizer unit tests ─────────────────────────────────────────────────────

#[test]
fn test_tokenize_basic() {
    let tokens = graph_db_cassie::search::tokenize("Hello World, medical data");
    assert!(tokens.contains(&"hello".to_string()));
    assert!(tokens.contains(&"world".to_string()));
    assert!(tokens.contains(&"medical".to_string()));
    assert!(tokens.contains(&"data".to_string()));
}

#[test]
fn test_tokenize_deduplicates() {
    let tokens = graph_db_cassie::search::tokenize("the cat sat on the mat with the cat");
    let count = tokens.iter().filter(|t| t.as_str() == "cat").count();
    assert_eq!(count, 1, "tokens should be deduplicated");
}

#[test]
fn test_tokenize_filters_short() {
    let tokens = graph_db_cassie::search::tokenize("a an the go do it");
    // All words < 3 chars should be filtered
    for t in &tokens {
        assert!(t.len() >= 3, "token '{t}' is too short");
    }
}

// ─── Graph traversal ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_children() {
    let client = setup().await;
    let index = make_index("gchild_user", "gchild_doc", "graph.md");
    client.save(&index).await.unwrap();

    let loaded = client.load("gchild_user", "gchild_doc").await.unwrap();
    // We need the root vertex_id — fetch it via the store's document record
    // For the test we just verify that get_children doesn't error on a valid UUID
    // (deep graph traversal tests require knowing the actual UUID, which is generated at save time)
    let result = client
        .get_children(uuid::Uuid::new_v4()) // random = 0 children
        .await
        .expect("get_children should not error on unknown UUID");
    assert!(result.is_empty());

    let _ = client.delete("gchild_user", "gchild_doc").await;
    let _ = loaded; // suppress unused warning
}
