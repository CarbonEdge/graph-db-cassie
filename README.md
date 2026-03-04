# Cassie — Graph DB on Cassandra

Cassandra-backed property graph store for hierarchical document indexes.
Replaces the embedded Sled key-value store with a distributed graph that
supports node-level search, tree traversal, and top-K relevance queries.

---

## Database Structure

### Keyspace: `cassie`

Five tables. Each document tree is decomposed into vertices (nodes) and edges
(relationships), with two supporting tables for fast document listing and
full-text search.

```
cassie keyspace
│
├── documents          — document registry, one row per document
├── vertices           — every tree node as a graph vertex
├── edges_out          — parent → child directed edges
├── edges_in           — child → parent reverse index
└── search_tokens      — inverted word index for dirty search
```

---

### Table: `documents`

The document registry. One row per `(user_id, doc_id)` pair. Sorted by
creation time descending so listing a user's documents is a single
partition read.

```
PRIMARY KEY ((user_id), created_at DESC, doc_id)

user_id      TEXT        — partition key (all docs for a user in one partition)
created_at   TIMESTAMP   — clustering key, newest first
doc_id       TEXT        — clustering key
root_id      UUID        — vertex_id of the root TreeNode
filename     TEXT
doc_type     TEXT        — 'pdf' | 'markdown'
description  TEXT
total_pages  INT
raw_content  TEXT        — original source text / page JSON
config_json  TEXT        — IndexConfig serialised as JSON
```

---

### Table: `vertices`

Every `TreeNode` in every document tree is stored as one vertex row.
A document with 6 sections produces 7 vertex rows (root + 6 children).

```
PRIMARY KEY (vertex_id)

vertex_id    UUID        — partition key, randomly generated per node
user_id      TEXT
doc_id       TEXT
vtype        TEXT        — 'document' | 'section' | 'leaf'
title        TEXT
summary      TEXT
content      TEXT        — raw text for this section (optional)
start_idx    INT         — start page / line
end_idx      INT         — end page / line (inclusive)
node_id      TEXT        — original TreeNode.node_id ("0001", "1.2.3")
properties   MAP<TEXT,TEXT>
created_at   TIMESTAMP
```

---

### Tables: `edges_out` and `edges_in`

Edges encode the parent→child relationships of the original tree.
Two tables give O(1) lookups in both directions without a secondary index.

```
edges_out — forward traversal (get children of a node)
PRIMARY KEY ((from_id, label), to_id)

from_id   UUID    — parent vertex
label     TEXT    — always 'CONTAINS' in the current schema
to_id     UUID    — child vertex


edges_in — reverse traversal (get parent / ancestors)
PRIMARY KEY ((to_id, label), from_id)

to_id     UUID    — child vertex
label     TEXT    — always 'CONTAINS'
from_id   UUID    — parent vertex
```

---

### Table: `search_tokens`

Inverted index built at write time. Each word in a vertex's `title` and
`summary` gets one row. Query time: split the search string into words,
fetch each word's rows in parallel, union the results, rank by hit count.

```
PRIMARY KEY ((user_id, word), vertex_id)

user_id    TEXT    — partition key (search is always scoped to one user)
word       TEXT    — partition key, lowercase alphabetic, min 3 chars
vertex_id  UUID    — clustering key
doc_id     TEXT    — denormalised for fast display
title      TEXT    — denormalised
summary    TEXT    — denormalised
start_idx  INT
end_idx    INT
```

---

## Data Flow

### Write path (`save`)

```
DocumentIndex (nested TreeNode tree)
        │
        ▼
   graph::decompose()          — DFS walk produces flat lists
        │
        ├─▶  Vec<Vertex>       — INSERT INTO cassie.vertices  (one row per node)
        │
        ├─▶  Vec<Edge>         — INSERT INTO cassie.edges_out  (parent → child)
        │                        INSERT INTO cassie.edges_in   (child  → parent)
        │
        ├─▶  documents row     — INSERT INTO cassie.documents  (root_id stored here)
        │
        └─▶  search_tokens     — tokenise title+summary → INSERT one row per word
```

### Read path (`load`)

```
(user_id, doc_id)
        │
        ▼
  cassie.documents              — fetch root_id + metadata
        │
        ▼
  cassie.vertices               — fetch all vertices for doc  (ALLOW FILTERING)
        │
        ▼
  cassie.edges_out              — fetch children for every vertex_id
        │
        ▼
   graph::recompose()           — BFS rebuild of nested TreeNode tree
        │
        ▼
   DocumentIndex
```

### Search path (`search`)

```
query string
        │
        ▼
   tokenize()                   — lowercase, split on non-alpha, deduplicate, min 3 chars
        │
        ▼  (one query per word, can be parallelised)
   cassie.search_tokens         — SELECT WHERE (user_id, word) = ?
        │
        ▼
   union results, count hits per vertex_id
        │
        ▼
   sort by score DESC, truncate to top_k
        │
        ▼
   Vec<SearchResult>
```

---

## Schema Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                        cassie keyspace                              │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  documents                                                   │  │
│  │  PK: (user_id) | (created_at DESC, doc_id)                  │  │
│  │                                                              │  │
│  │  user_id · created_at · doc_id · root_id ──────────────┐   │  │
│  │  filename · doc_type · description · total_pages        │   │  │
│  │  raw_content · config_json                              │   │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                          │          │
│                             points to root vertex        │          │
│                                                          ▼          │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  vertices                                                    │  │
│  │  PK: (vertex_id)                                             │  │
│  │                                                              │  │
│  │  vertex_id · user_id · doc_id · vtype                       │  │
│  │  title · summary · content                                   │  │
│  │  start_idx · end_idx · node_id                               │  │
│  │  properties · created_at                                     │  │
│  └──────────────┬───────────────────────────────────────────────┘  │
│                 │                                                   │
│        vertex_id referenced by edges                                │
│                 │                                                   │
│       ┌─────────┴──────────┐                                        │
│       ▼                    ▼                                        │
│  ┌──────────────┐    ┌──────────────┐                               │
│  │  edges_out   │    │  edges_in    │                               │
│  │  PK:         │    │  PK:         │                               │
│  │  (from_id,   │    │  (to_id,     │                               │
│  │   label)     │    │   label)     │                               │
│  │  + to_id     │    │  + from_id   │                               │
│  │              │    │              │                               │
│  │  parent→child│    │  child→parent│                               │
│  │  traversal   │    │  traversal   │                               │
│  └──────────────┘    └──────────────┘                               │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  search_tokens                                               │  │
│  │  PK: (user_id, word) | (vertex_id)                          │  │
│  │                                                              │  │
│  │  user_id · word · vertex_id · doc_id                        │  │
│  │  title · summary · start_idx · end_idx                      │  │
│  │                                                              │  │
│  │  one row per word per vertex — built at write time           │  │
│  └──────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Example: A 6-node document

Given a PDF with this structure:

```
Root (pages 1–20)
├── Introduction  (pages 1–5)
├── Methods       (pages 6–15)
│   ├── Data Collection  (pages 6–10)
│   └── Analysis         (pages 11–15)
└── Conclusion    (pages 16–20)
```

Cassie stores:

| Table | Rows written |
|-------|-------------|
| `documents` | 1 (the document record) |
| `vertices` | 6 (one per TreeNode) |
| `edges_out` | 5 (Root→Intro, Root→Methods, Root→Conclusion, Methods→DataCollection, Methods→Analysis) |
| `edges_in` | 5 (same edges, reverse direction) |
| `search_tokens` | N (one per unique word across all titles + summaries) |

---

## HTTP API

Accessed internally within the cluster at `cassie:8080`. No ingress, no auth.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check |
| `PUT` | `/documents` | Save a DocumentIndex (JSON body) |
| `GET` | `/documents/:user_id` | List all documents for a user |
| `GET` | `/documents/:user_id/:doc_id` | Load full document with tree |
| `DELETE` | `/documents/:user_id/:doc_id` | Delete document and all data |
| `GET` | `/search/:user_id?q=...&top_k=5` | Token search, returns top-K nodes |

---

## Local Development

```bash
# Start Cassandra
docker compose up -d

# Run unit tests (no DB needed)
cargo test --lib

# Run integration tests (DB required, wait ~45s for Cassandra to be healthy)
cargo test --test integration -- --nocapture

# Run the API server
CASSANDRA_HOST=127.0.0.1:9042 cargo run --bin cassie-api
```

## Deploy

```bash
# Dev
gh workflow run helm-deploy.yml -f service=cassie -f environment=dev

# Production
gh workflow run helm-deploy.yml -f service=cassie -f environment=production
```

Cassandra and the API server are deployed together as a single Helm release.
Other services reach the API via Kubernetes DNS: `cassie:8080`.

version 0.0.1