use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, put},
    Json, Router,
};
use serde::Deserialize;
use tracing::info;

use graph_db_cassie::{CassieClient, CassieConfig, CassieError, DocumentIndex, SearchResult};

// ─── App state ───────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    cassie: Arc<CassieClient>,
}

// ─── Error response ──────────────────────────────────────────────────────────

struct ApiError(CassieError);

impl From<CassieError> for ApiError {
    fn from(e: CassieError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            CassieError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ─── Handlers ────────────────────────────────────────────────────────────────

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn save_document(
    State(state): State<AppState>,
    Json(index): Json<DocumentIndex>,
) -> ApiResult<StatusCode> {
    state.cassie.save(&index).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn load_document(
    State(state): State<AppState>,
    Path((user_id, doc_id)): Path<(String, String)>,
) -> ApiResult<Json<DocumentIndex>> {
    let index = state.cassie.load(&user_id, &doc_id).await?;
    Ok(Json(index))
}

async fn list_documents(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> ApiResult<Json<Vec<DocumentIndex>>> {
    let docs = state.cassie.list(&user_id).await?;
    Ok(Json(docs))
}

async fn delete_document(
    State(state): State<AppState>,
    Path((user_id, doc_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    state.cassie.delete(&user_id, &doc_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SearchParams {
    q: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

fn default_top_k() -> usize {
    5
}

async fn search_documents(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<SearchResult>>> {
    let results = state
        .cassie
        .search(&user_id, &params.q, params.top_k)
        .await?;
    Ok(Json(results))
}

// ─── Router ──────────────────────────────────────────────────────────────────

fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/documents", put(save_document))
        .route("/documents/:user_id", get(list_documents))
        .route("/documents/:user_id/:doc_id", get(load_document))
        .route("/documents/:user_id/:doc_id", delete(delete_document))
        .route("/search/:user_id", get(search_documents))
        .with_state(state)
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "cassie_api=info".to_string()),
        )
        .init();

    let host = std::env::var("CASSANDRA_HOST").unwrap_or_else(|_| "127.0.0.1:9042".to_string());
    let port = std::env::var("SERVER_PORT").unwrap_or_else(|_| "8080".to_string());

    info!("Connecting to Cassandra at {host}");

    let config = CassieConfig {
        contact_points: vec![host],
        keyspace: "cassie".to_string(),
    };

    let client = CassieClient::new(config)
        .await
        .expect("Failed to connect to Cassandra");

    client
        .setup_schema()
        .await
        .expect("Failed to set up schema");

    info!("Schema ready");

    let state = AppState {
        cassie: Arc::new(client),
    };

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind");

    info!("Listening on {addr}");
    axum::serve(listener, router(state))
        .await
        .expect("Server error");
}
