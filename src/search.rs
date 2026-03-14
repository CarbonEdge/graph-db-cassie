use std::collections::HashMap;

use uuid::Uuid;

use crate::{
    client::CassieClient,
    error::{CassieError, Result},
    types::{SearchResult, Vertex},
};

// ─── Tokenizer ────────────────────────────────────────────────────────────────

/// Split text into lowercase alphabetic words (min 3 chars), deduplicated.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    text.split(|c: char| !c.is_alphabetic())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 3)
        .filter(|w| seen.insert(w.clone()))
        .collect()
}

// ─── Index ────────────────────────────────────────────────────────────────────

/// Write search words for a vertex into `cassie.search_tokens`.
pub async fn index_vertex(client: &CassieClient, vertex: &Vertex) -> Result<()> {
    let mut words = tokenize(&vertex.title);
    if let Some(ref summary) = vertex.summary {
        words.extend(tokenize(summary));
    }
    if let Some(ref content) = vertex.content {
        words.extend(tokenize(content));
    }
    words.sort();
    words.dedup();

    for word in words {
        client
            .session
            .query_unpaged(
                "INSERT INTO cassie.search_tokens \
                 (user_id, word, vertex_id, doc_id, title, summary, start_idx, end_idx, node_id) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    &vertex.user_id,
                    &word,
                    vertex.vertex_id,
                    &vertex.doc_id,
                    &vertex.title,
                    &vertex.summary,
                    vertex.start_idx as i32,
                    vertex.end_idx as i32,
                    &vertex.node_id,
                ),
            )
            .await?;
    }
    Ok(())
}

/// Remove all search words for a vertex (called during delete).
pub async fn delete_vertex_words(client: &CassieClient, vertex: &Vertex) -> Result<()> {
    let mut words = tokenize(&vertex.title);
    if let Some(ref summary) = vertex.summary {
        words.extend(tokenize(summary));
    }
    if let Some(ref content) = vertex.content {
        words.extend(tokenize(content));
    }
    words.sort();
    words.dedup();

    for word in words {
        client
            .session
            .query_unpaged(
                "DELETE FROM cassie.search_tokens \
                 WHERE user_id = ? AND word = ? AND vertex_id = ?",
                (&vertex.user_id, &word, vertex.vertex_id),
            )
            .await?;
    }
    Ok(())
}

// ─── Search ───────────────────────────────────────────────────────────────────

/// TF-IDF-ranked search: tokenise query, union-match vertices, weight rare words
/// higher, return top-K by score.
pub async fn search(
    client: &CassieClient,
    user_id: &str,
    query: &str,
    top_k: usize,
) -> Result<Vec<SearchResult>> {
    let words = tokenize(query);
    if words.is_empty() {
        return Ok(vec![]);
    }

    // vertex_id → (score_numerator, SearchResult)
    let mut hits: HashMap<Uuid, (u32, SearchResult)> = HashMap::new();

    for word in &words {
        let result = client
            .session
            .query_unpaged(
                "SELECT vertex_id, doc_id, title, summary, start_idx, end_idx, node_id \
                 FROM cassie.search_tokens \
                 WHERE user_id = ? AND word = ?",
                (user_id, word),
            )
            .await?;

        let rows_result = result.into_rows_result()?;
        let rows: Vec<_> = rows_result
            .rows::<(Uuid, String, String, Option<String>, i32, i32, Option<String>)>()
            .map_err(|e| CassieError::RowDe(e.to_string()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| CassieError::RowDe(e.to_string()))?;

        // IDF weight: rare words score higher
        let doc_freq = rows.len();
        let idf_weight = 1000u32 / (1 + doc_freq as u32);

        for (vid, doc_id, title, summary, start_idx, end_idx, node_id) in rows {
            let entry = hits.entry(vid).or_insert_with(|| {
                (
                    0,
                    SearchResult {
                        vertex_id: vid,
                        doc_id,
                        title,
                        summary,
                        score: 0,
                        start_idx: start_idx as u32,
                        end_idx: end_idx as u32,
                        node_id: node_id.unwrap_or_default(),
                    },
                )
            });
            entry.0 += idf_weight;
        }
    }

    let mut results: Vec<SearchResult> = hits
        .into_values()
        .map(|(score, mut r)| {
            r.score = score;
            r
        })
        .collect();

    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.truncate(top_k);
    Ok(results)
}
