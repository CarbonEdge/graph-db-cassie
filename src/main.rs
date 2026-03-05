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

/// Liveness probe — always 200 while the process is alive.
async fn health() -> StatusCode {
    StatusCode::OK
}

/// Readiness probe — 200 only when Cassandra is reachable.
async fn ready(State(state): State<AppState>) -> StatusCode {
    match state.cassie.ping().await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
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

async fn connect_with_retry(config: CassieConfig) -> CassieClient {
    let mut delay = Duration::from_secs(2);
    for attempt in 1u32.. {
        match CassieClient::new(config.clone()).await {
            Ok(client) => {
                info!("Connected to Cassandra on attempt {attempt}");
                return client;
            }
            Err(e) => {
                if attempt >= 15 {
                    panic!("Failed to connect to Cassandra after {attempt} attempts: {e}");
                }
                warn!(
                    attempt,
                    delay_secs = delay.as_secs(),
                    error = %e,
                    "Cassandra not ready, retrying"
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    }
    unreachable!()
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let otel_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());

    init_otel("cassie-api", &otel_endpoint);

    let host = std::env::var("CASSANDRA_HOST").unwrap_or_else(|_| "127.0.0.1:9042".to_string());
    let port = std::env::var("SERVER_PORT").unwrap_or_else(|_| "8080".to_string());

    info!("Connecting to Cassandra at {host}");

    let config = CassieConfig {
        contact_points: vec![host],
        keyspace: "cassie".to_string(),
    };

    let client = connect_with_retry(config).await;

    client
        .setup_schema()
        .await
        .expect("Failed to set up schema");

    info!("Schema ready");

    let state = AppState {
        cassie: Arc::new(client),
    };

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
