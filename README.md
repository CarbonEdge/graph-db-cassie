# Cassie — Graph DB + AI Query Stack

Cassandra-backed document graph store with a web crawler for ingestion and an AI query layer powered by Claude.

---

## How it works

```
URLs / PDFs
     │
     ▼
data-pipeline/ingest.py        crawls URLs, extracts text, builds document trees
     │
     ▼  PUT /documents
cassie-api  (port 8080)        stores documents as a property graph in Cassandra
     │
     ▼  GET /search + GET /documents
cassie-ai   (port 8081)        retrieves context, calls Claude, returns answers
```

---

## Prerequisites

- [Docker Desktop](https://www.docker.com/products/docker-desktop/) (with Compose)
- An [Anthropic API key](https://console.anthropic.com/)
- Python 3.11+ (for running the ingestion script)

---

## 1. Start the local stack

```bash
cd D:\dev\runpod\graph-db-cassie

# Copy the example env file and add your API key
cp .env.example .env
```

Edit `.env`:
```
LLM_API_KEY=sk-ant-your-key-here
```

Then start everything:
```bash
docker compose up --build
```

This starts three containers:

| Service | Port | Description |
|---------|------|-------------|
| `cassie-db` | 9042 | Cassandra (data store) |
| `cassie-api` | 8080 | Rust graph API |
| `cassie-ai` | 8081 | FastAPI AI query service |

> **First run takes 3–5 minutes** — the Rust API compiles inside Docker. Subsequent starts reuse the cached image.

Wait until you see Cassie API logs appear, then confirm everything is healthy:

```bash
curl http://localhost:8080/health
curl http://localhost:8081/health
```

Both should return `{"status":"ok"}`.

---

## 2. Index your first document

Install the ingestion dependencies:

```bash
cd D:\dev\runpod\data-pipeline
pip install -r requirements.txt
```

Edit `config/urls.yaml` to add the document you want to ingest:

```yaml
sitemaps: []

seeds:
  - https://www.health.gov.za/wp-content/uploads/2025/07/Primary-Healthcare-Standard-Treatment-Guidelines-and-Essential-Medicines-List-8th-Edition-2024.pdf
```

Run the ingester:

```bash
python ingest.py \
  --config config/urls.yaml \
  --cassie-url http://localhost:8080 \
  --user-id public
```

You should see output like:

```
2026-03-05 12:00:01 INFO ingest: Crawled 1 documents
2026-03-05 12:00:03 INFO ingest: Pushed Primary-Healthcare-Standard-Treatment-...pdf (https://...)
2026-03-05 12:00:03 INFO ingest: Ingest complete — crawled=1 pushed=1 skipped=0 errors=0
```

Confirm the document was stored:

```bash
curl http://localhost:8080/documents/public
```

---

## 3. Ask a question

```bash
curl -X POST http://localhost:8081/query \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "public",
    "question": "What is the first-line treatment for hypertension?",
    "top_k": 5,
    "depth": 1
  }'
```

Response:

```json
{
  "answer": "According to the PHC Standard Treatment Guidelines (8th Edition), the first-line treatment for hypertension is...",
  "sources": [
    {
      "doc_id": "wp-content-uploads-2025-07-Primary-Healthcare-...",
      "title": "Hypertension",
      "node_id": "0003.0002",
      "score": 4
    }
  ]
}
```

---

## 4. Adding more documents

Add more URLs to `config/urls.yaml` and re-run `ingest.py`. Documents that are already indexed are skipped automatically — use `--force` to re-ingest:

```yaml
sitemaps:
  - https://example.com/sitemap.xml   # all pages from a sitemap

seeds:
  - https://www.health.gov.za/wp-content/uploads/2025/07/Primary-Healthcare-Standard-Treatment-Guidelines-and-Essential-Medicines-List-8th-Edition-2024.pdf
  - https://example.com/some-page.html
```

```bash
# Skip already-indexed docs (default)
python ingest.py --config config/urls.yaml --cassie-url http://localhost:8080 --user-id public

# Force re-index everything
python ingest.py --config config/urls.yaml --cassie-url http://localhost:8080 --user-id public --force
```

---

## 5. Other useful commands

**List all indexed documents:**
```bash
curl http://localhost:8080/documents/public
```

**Run a raw search (without AI):**
```bash
curl "http://localhost:8080/search/public?q=hypertension&top_k=5"
```

**Delete a document:**
```bash
curl -X DELETE http://localhost:8080/documents/public/<doc_id>
```

**Stop the stack:**
```bash
docker compose down

# To also wipe Cassandra data:
docker compose down -v
```

---

## Repository layout

```
graph-db-cassie/          Rust API + docker-compose (this repo)
├── src/                  Cassie API source (Axum + Scylla)
├── docker-compose.yml    Local stack: Cassandra + cassie-api + cassie-ai
├── Dockerfile            Builds the Rust API
└── .env.example          Copy to .env and set LLM_API_KEY

data-pipeline/            Python ingestion scripts
├── ingest.py             Entry point — crawl → build → push
├── crawler.py            Sitemap parser + HTML/PDF fetcher
├── document_builder.py   Converts raw content → DocumentIndex tree
├── cassie_client.py      HTTP client for the Cassie API
└── config/urls.yaml      Edit this to configure what gets indexed

cassie-ai/                FastAPI AI query service
├── app/main.py           POST /query endpoint
├── app/retriever.py      Graph-aware context assembly
├── app/llm.py            Claude (or OpenAI) integration
└── Dockerfile
```

---

## Troubleshooting

**Cassandra takes too long to start**
Cassandra needs ~45 seconds to initialise on first run. The API container will wait for it. If it keeps failing, increase Docker Desktop's memory allocation (4 GB+ recommended).

**PDF extraction is slow**
Large PDFs are extracted in-process by `pymupdf4llm`. A 500-page document may take 30–60 seconds. This is normal.

**`connection refused` on port 8080/8081**
The API is still starting. Check `docker compose logs cassie` or `docker compose logs cassie-ai`.

**Search returns no results**
Make sure ingestion completed without errors. The search index is built at write time — if the push failed, there's nothing to search. Check `docker compose logs cassie-api` for write errors.

---

## Database internals

### Keyspace: `cassie`

Five tables. Each document tree is decomposed into vertices (nodes) and edges (relationships), with two supporting tables for fast document listing and full-text search.

```
cassie keyspace
│
├── documents          — document registry, one row per document
├── vertices           — every tree node as a graph vertex
├── edges_out          — parent → child directed edges
├── edges_in           — child → parent reverse index
└── search_tokens      — inverted word index for dirty search
```

### Table: `documents`

```
PRIMARY KEY ((user_id), created_at DESC, doc_id)

user_id      TEXT        — partition key
created_at   TIMESTAMP   — clustering key, newest first
doc_id       TEXT        — clustering key
root_id      UUID        — vertex_id of the root TreeNode
filename     TEXT
doc_type     TEXT        — 'pdf' | 'markdown'
description  TEXT
total_pages  INT
raw_content  TEXT
config_json  TEXT        — IndexConfig serialised as JSON
```

### Table: `vertices`

```
PRIMARY KEY (vertex_id)

vertex_id    UUID
user_id      TEXT
doc_id       TEXT
vtype        TEXT        — 'document' | 'section' | 'leaf'
title        TEXT
summary      TEXT
content      TEXT
start_idx    INT
end_idx      INT
node_id      TEXT        — hierarchical id e.g. "0001.0002"
properties   MAP<TEXT,TEXT>
created_at   TIMESTAMP
```

### Tables: `edges_out` / `edges_in`

```
edges_out  PRIMARY KEY ((from_id, label), to_id)   — parent → child
edges_in   PRIMARY KEY ((to_id,   label), from_id) — child  → parent
```

### Table: `search_tokens`

```
PRIMARY KEY ((user_id, word), vertex_id)

One row per word per vertex. Words are lowercase, alphabetic-only, minimum 3 characters.
Built at write time. Query time: union results across words, rank by hit count.
```

### HTTP API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Liveness check |
| `GET` | `/ready` | Readiness check (pings Cassandra) |
| `PUT` | `/documents` | Save a DocumentIndex (JSON body) |
| `GET` | `/documents/:user_id` | List documents for a user |
| `GET` | `/documents/:user_id/:doc_id` | Load full document tree |
| `DELETE` | `/documents/:user_id/:doc_id` | Delete document and all data |
| `GET` | `/search/:user_id?q=...&top_k=5` | Token search, returns top-K nodes |
