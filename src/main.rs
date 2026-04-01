use std::{sync::Arc, time::Duration};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, put},
    Json, Router,
};
use axum_prometheus::PrometheusMetricLayer;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace as sdktrace, Resource};
use serde::Deserialize;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use graph_db_cassie::{
    CassieClient, CassieConfig, CassieError, DocumentIndex, SearchResult, TreeNode,
};

// ─── App state ───────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    cassie: Option<Arc<CassieClient>>,
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

/// Liveness probe — always 200 while the process is alive.
async fn health() -> StatusCode {
    StatusCode::OK
}

/// Readiness probe — 200 always (Cassandra optional).
async fn ready(State(state): State<AppState>) -> StatusCode {
    match &state.cassie {
        Some(cassie) => match cassie.ping().await {
            Ok(_) => StatusCode::OK,
            Err(_) => StatusCode::SERVICE_UNAVAILABLE,
        },
        None => StatusCode::OK,
    }
}

fn count_vertices(node: &TreeNode) -> usize {
    1 + node.nodes.iter().map(count_vertices).sum::<usize>()
}

async fn save_document(
    State(state): State<AppState>,
    Json(index): Json<DocumentIndex>,
) -> Response {
    const MAX_VERTICES: usize = 2000;
    const MAX_RAW_CONTENT_BYTES: usize = 10 * 1024 * 1024; // 10 MB

    let cassie = match &state.cassie {
        Some(c) => c,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "Cassandra not available"})),
            )
                .into_response();
        }
    };

    let vertex_count = count_vertices(&index.tree);
    if vertex_count > MAX_VERTICES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": format!("Document has {vertex_count} nodes, max is {MAX_VERTICES}")
            })),
        )
            .into_response();
    }

    if let Some(ref rc) = index.raw_content {
        if rc.len() > MAX_RAW_CONTENT_BYTES {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({"error": "raw_content exceeds 10MB limit"})),
            )
                .into_response();
        }
    }

    match cassie.save(&index).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

async fn load_document(
    State(state): State<AppState>,
    Path((user_id, doc_id)): Path<(String, String)>,
) -> ApiResult<Json<DocumentIndex>> {
    let cassie = state
        .cassie
        .as_ref()
        .ok_or_else(|| ApiError(CassieError::NotFound("Cassandra not available".to_string())))?;
    let index = cassie.load(&user_id, &doc_id).await?;
    Ok(Json(index))
}

async fn list_documents(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> ApiResult<Json<Vec<DocumentIndex>>> {
    let cassie = state
        .cassie
        .as_ref()
        .ok_or_else(|| ApiError(CassieError::NotFound("Cassandra not available".to_string())))?;
    let docs = cassie.list(&user_id).await?;
    Ok(Json(docs))
}

async fn delete_document(
    State(state): State<AppState>,
    Path((user_id, doc_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let cassie = state
        .cassie
        .as_ref()
        .ok_or_else(|| ApiError(CassieError::NotFound("Cassandra not available".to_string())))?;
    cassie.delete(&user_id, &doc_id).await?;
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
    let cassie = state
        .cassie
        .as_ref()
        .ok_or_else(|| ApiError(CassieError::NotFound("Cassandra not available".to_string())))?;
    let results = cassie.search(&user_id, &params.q, params.top_k).await?;
    Ok(Json(results))
}

// ─── Router ──────────────────────────────────────────────────────────────────

fn router(state: AppState, prometheus_layer: PrometheusMetricLayer<'static>) -> Router {
    // /metrics and probes are excluded from Prometheus instrumentation so
    // they don't pollute histograms with high-frequency scrape noise.
    let api_routes = Router::new()
        .route("/documents", put(save_document))
        .route("/documents/:user_id", get(list_documents))
        .route("/documents/:user_id/:doc_id", get(load_document))
        .route("/documents/:user_id/:doc_id", delete(delete_document))
        .route("/search/:user_id", get(search_documents))
        .layer(prometheus_layer)
        .layer(TraceLayer::new_for_http());

    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .merge(api_routes)
        .with_state(state)
}

// ─── Observability init ───────────────────────────────────────────────────────

fn init_otel(service_name: &str, endpoint: &str) {
    use opentelemetry::trace::TracerProvider as _;

    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .with_trace_config(
            sdktrace::Config::default().with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                service_name.to_owned(),
            )])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .expect("Failed to initialize OTEL tracer");

    let tracer = provider.tracer(service_name.to_owned());
    opentelemetry::global::set_tracer_provider(provider);

    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "cassie_api=info".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(otel_layer)
        .init();
}

// ─── Startup helpers ─────────────────────────────────────────────────────────

async fn try_connect(config: CassieConfig) -> Option<CassieClient> {
    match CassieClient::new(config.clone()).await {
        Ok(client) => {
            info!("Connected to Cassandra");
            Some(client)
        }
        Err(e) => {
            warn!("Cassandra not available (optional): {e}");
            None
        }
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let otel_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());

    init_otel("cassie-api", &otel_endpoint);

    let host = std::env::var("CASSANDRA_HOST").unwrap_or_else(|_| "127.0.0.1:9042".to_string());
    let port = std::env::var("SERVER_PORT").unwrap_or_else(|_| "8080".to_string());

    info!("Attempting to connect to Cassandra at {host}");

    let config = CassieConfig {
        contact_points: vec![host],
        keyspace: "cassie".to_string(),
    };

    let cassie = match try_connect(config.clone()).await {
        Some(client) => {
            if let Err(e) = client.setup_schema().await {
                warn!("Failed to set up schema: {e}");
            } else {
                info!("Schema ready");
            }
            Some(Arc::new(client))
        }
        None => {
            info!("Running without Cassandra (in-memory mode)");
            None
        }
    };

    let state = AppState { cassie };

    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind");

    // Mount /metrics outside the main router so it is not self-instrumented.
    let app = router(state, prometheus_layer).route(
        "/metrics",
        get(move || async move { metric_handle.render() }),
    );

    info!("Listening on {addr}");
    axum::serve(listener, app).await.expect("Server error");

    opentelemetry::global::shutdown_tracer_provider();
}
