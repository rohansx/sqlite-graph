# sqlite-graph

An embeddable graph database built entirely on SQLite. Recursive CTEs for traversal, bi-temporal edges, FTS5 full-text search, and vector fusion — in a single file.

No Docker. No JVM. No network hop. No `docker-compose.yml` you'll fight with for an hour.

```rust
use sqlite_graph::{Graph, Entity, Edge};

let graph = Graph::open_or_create("my.db".as_ref())?;

let alice = Entity::new("Alice", "person");
let bob = Entity::new("Bob", "person");
graph.add_entity(alice.clone())?;
graph.add_entity(bob.clone())?;

graph.add_edge(Edge::new(&alice.id, &bob.id, "knows"))?;

// Multi-hop traversal via recursive CTE
let (entities, edges) = graph.traverse(&alice.id, 3)?;
```

## Why this exists

If you want a knowledge graph today, the typical stack looks like this:

| Layer | Traditional | sqlite-graph |
|---|---|---|
| Graph storage | Neo4j (Docker) | SQLite file |
| Entity extraction | OpenAI API | Your choice (or [ctxgraph](https://github.com/rohansx/ctxgraph) for local ONNX) |
| Embedding storage | Pinecone / Qdrant | BLOB columns |
| Semantic search | External vector DB | Cosine similarity in-process |
| Full-text search | Elasticsearch | FTS5 (built into SQLite) |

Four services, two network dependencies — or one library and one file.

The core insight: a graph database is really two things — a storage format for nodes and edges, and a query engine that can walk those edges. SQLite handles the first trivially. For the second, recursive CTEs give you everything you need for multi-hop traversal.

## Install

```toml
[dependencies]
sqlite-graph = "0.1"
```

## Features

| Feature | How it works |
|---|---|
| **Graph traversal** | Recursive CTEs — multi-hop walks in a single SQL statement |
| **Bi-temporal edges** | `valid_from`/`valid_until` (real-world) + `recorded_at` (system time) |
| **Full-text search** | FTS5 across episodes, entities, and edges with trigger-based sync |
| **Hybrid search** | FTS5 + vector cosine similarity fused with Reciprocal Rank Fusion |
| **Entity deduplication** | Jaro-Winkler fuzzy matching with alias resolution |
| **Embeddings** | Store and query f32 vectors as BLOB columns |
| **Single file** | Standard SQLite — `cp graph.db backup.db` and you're done |

## Usage

### Episodes (events)

Episodes are the fundamental unit of information — a decision, a message, a commit, an incident.

```rust
use sqlite_graph::{Graph, Episode};

let graph = Graph::in_memory()?;

let ep = Episode::builder("Team chose Postgres for the billing service")
    .source("standup")
    .tag("architecture")
    .meta("author", "alice")
    .build();

graph.add_episode(ep)?;
```

### Entities and edges

```rust
use sqlite_graph::{Graph, Entity, Edge};

let graph = Graph::in_memory()?;

let pg = Entity::new("PostgreSQL", "technology");
let billing = Entity::new("Billing Service", "component");
graph.add_entity(pg.clone())?;
graph.add_entity(billing.clone())?;

let mut edge = Edge::new(&pg.id, &billing.id, "used_by");
edge.fact = Some("Billing service uses PostgreSQL for persistence".into());
graph.add_edge(edge)?;
```

### Graph traversal

Traverse the graph from any entity. Uses a recursive CTE under the hood — bidirectional, depth-bounded, with cycle detection via `UNION`.

```rust
// Find everything within 2 hops of PostgreSQL
let (entities, edges) = graph.traverse(&pg.id, 2)?;

// Get an entity's immediate neighborhood
let ctx = graph.get_entity_context(&pg.id)?;
println!("{} has {} neighbors", ctx.entity.name, ctx.neighbors.len());
```

### Full-text search

Three FTS5 indexes (episodes, entities, edges) are automatically maintained via 9 triggers — no manual reindexing.

```rust
// Search episodes
let results = graph.search("postgres billing", 10)?;
for (episode, score) in &results {
    println!("[{:.2}] {}", score, episode.content);
}

// Search entities
let entities = graph.search_entities("PostgreSQL", 5)?;
```

### Hybrid search (FTS5 + vectors + RRF)

Keyword search misses semantic similarity. Vector search misses exact matches. We run both and fuse the results using [Reciprocal Rank Fusion](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf).

```rust
// You provide the embedding (from any model — OpenAI, sentence-transformers, etc.)
let query_embedding: Vec<f32> = your_embed_fn("postgres billing");

let results = graph.search_fused("postgres billing", &query_embedding, 10)?;
for result in &results {
    println!("[{:.4}] {}", result.score, result.episode.content);
}
```

RRF formula: `score(d) = sum(1 / (k + rank_i(d)))` for each mode where `d` appears (`k=60`). Rank-based fusion avoids the need to normalize incompatible score distributions (BM25 scores vs cosine similarities).

### Bi-temporal edges

Every edge tracks two time dimensions. Facts expire but never disappear.

- **`valid_from` / `valid_until`** — when the fact was true *in the real world*
- **`recorded_at`** — when the system *learned* about the fact

This distinction matters for debugging. "What did we *know* about the system on March 10th?" is a different question from "What was *actually true* on March 10th?" Bi-temporal modeling answers both.

```rust
use sqlite_graph::Edge;

// Alice worked at Google, then moved to Meta
let mut edge = Edge::new(&alice_id, &google_id, "works_at");
edge.fact = Some("Alice works at Google".into());
graph.add_edge(edge.clone())?;

// Later: invalidate the old edge (sets valid_until = now)
graph.invalidate_edge(&edge.id)?;

// Add the new one
let new_edge = Edge::new(&alice_id, &meta_id, "works_at");
graph.add_edge(new_edge)?;

// Traverse with history to see both
let (entities, edges) = graph.traverse_with_history(&alice_id, 2)?;
```

Invalidation never deletes — it sets `valid_until`, preserving the full audit trail:

```sql
UPDATE edges SET valid_until = ?1 WHERE id = ?2 AND valid_until IS NULL
```

### Entity deduplication

"PostgreSQL", "Postgres", and "PG" resolve to the same node via Jaro-Winkler similarity + an alias table.

```rust
let pg = Entity::new("PostgreSQL", "technology");
let (id, merged) = graph.add_entity_deduped(pg, 0.85)?;
// merged = false (new entity)

let pg2 = Entity::new("Postgres", "technology");
let (id2, merged) = graph.add_entity_deduped(pg2, 0.85)?;
// merged = true, id2 == id (matched "PostgreSQL" at 0.92 similarity)
```

Dedup is two-level: exact alias lookup first (SQL, O(1)), then Jaro-Winkler against all entities of the same type (Rust, computed in-process). The 0.85 threshold is tuned for software terminology — "PostgreSQL" vs "Postgres" scores 0.92, "React" vs "Redux" scores 0.73 (correctly rejected).

## Architecture

### Schema

7 tables, 3 FTS5 virtual tables, 8 indexes, 9 triggers. The full schema is applied via migration on first open.

```sql
-- Core tables
episodes          -- raw events (decisions, messages, incidents)
entities          -- graph nodes (people, services, technologies)
edges             -- relationships (bi-temporal, with confidence scores)
episode_entities  -- junction: which entities appear in which episodes
aliases           -- deduplication: alias_name → canonical_id
communities       -- clustering (reserved for community detection)
_migrations       -- schema versioning

-- FTS5 indexes (external content, trigger-synced)
episodes_fts      -- content, source, metadata
entities_fts      -- name, entity_type, summary
edges_fts         -- fact, relation
```

### How traversal works

The recursive CTE that replaces Cypher's `MATCH (a)-[*1..3]-(b)`:

```sql
WITH RECURSIVE traversal(entity_id, depth) AS (
    -- Base case: start at the given entity
    SELECT ?1, 0

    UNION

    -- Recursive step: walk edges in both directions
    SELECT
        CASE WHEN e.source_id = t.entity_id THEN e.target_id
             ELSE e.source_id END,
        t.depth + 1
    FROM traversal t
    JOIN edges e ON (e.source_id = t.entity_id OR e.target_id = t.entity_id)
    WHERE t.depth < ?2
      AND e.valid_until IS NULL  -- only current edges
)
SELECT ent.id, ent.name, ent.entity_type, ent.summary,
       ent.created_at, ent.metadata
FROM entities ent
WHERE ent.id IN (SELECT DISTINCT entity_id FROM traversal)
```

How it works:
1. **Base case** — seed with the starting entity at depth 0
2. **Recursive step** — for each discovered entity, find all edges where it's source or target (bidirectional traversal)
3. **`CASE` expression** — picks the *other* end of the edge
4. **`UNION` (not `UNION ALL`)** — deduplicates, preventing cycles
5. **Depth limit** — `WHERE t.depth < ?2` caps traversal hops
6. **Temporal filter** — `valid_until IS NULL` restricts to current edges

After collecting entities, a second query grabs all edges between them to return the complete subgraph.

The equivalent Cypher would be:

```cypher
MATCH path = (start:Entity {id: $id})-[*1..3]-(neighbor)
WHERE ALL(r IN relationships(path) WHERE r.valid_until IS NULL)
RETURN DISTINCT neighbor, length(path) AS depth
ORDER BY depth
```

Cypher is more concise. But the SQL version is self-contained — no external database process, no Bolt protocol, no connection pooling.

### Index strategy

8 indexes optimized for the workload:

| Index | Optimizes |
|---|---|
| `idx_edges_source` / `idx_edges_target` | Graph traversal JOIN — without these, every CTE step is a full table scan |
| `idx_edges_relation` | Filtering edges by relation type |
| `idx_edges_valid` | Temporal queries — composite `(valid_from, valid_until)` for current vs historical |
| `idx_entities_type` | Entity listing/filtering, used during fuzzy dedup |
| `idx_episode_entities` | Reverse lookup: which episodes mention this entity |
| `idx_episodes_source` / `idx_episodes_recorded` | Source filtering and time-range queries |

Plus 3 FTS5 virtual tables maintaining their own inverted indexes, synced via 9 triggers (INSERT/UPDATE/DELETE for each of episodes, entities, edges).

### FTS5 sync triggers

FTS5 external content tables don't auto-sync. Each base table has three triggers:

```sql
-- On INSERT: add to FTS index
CREATE TRIGGER episodes_ai AFTER INSERT ON episodes BEGIN
    INSERT INTO episodes_fts(rowid, content, source, metadata)
    VALUES (new.rowid, new.content, new.source, new.metadata);
END;

-- On DELETE: remove from FTS index
CREATE TRIGGER episodes_ad AFTER DELETE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, content, source, metadata)
    VALUES ('delete', old.rowid, old.content, old.source, old.metadata);
END;

-- On UPDATE: delete-then-insert
CREATE TRIGGER episodes_au AFTER UPDATE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, content, source, metadata)
    VALUES ('delete', old.rowid, old.content, old.source, old.metadata);
    INSERT INTO episodes_fts(rowid, content, source, metadata)
    VALUES (new.rowid, new.content, new.source, new.metadata);
END;
```

The `VALUES ('delete', ...)` syntax is FTS5's way of removing an entry. 9 triggers total (3 tables x 3 operations).

### Performance pragmas

```sql
PRAGMA journal_mode = WAL;      -- concurrent readers during writes
PRAGMA synchronous = NORMAL;    -- balance durability vs speed
PRAGMA foreign_keys = ON;       -- enforce referential integrity
```

**WAL** allows concurrent reads while writing — the typical workload is one writer ingesting data, multiple readers searching and traversing. **NORMAL** sync is fine for non-financial data. **Foreign keys** are off by default in SQLite — we turn them on because edges must reference valid entities.

## Performance characteristics

Designed for graphs up to ~100K nodes. Single-digit milliseconds for traversal at depth 2-3.

| Concern | Status | Mitigation |
|---|---|---|
| Traversal | O(branching^depth), bounded by `max_depth` | Fine for <100K with proper indexes |
| Vector search | Brute-force cosine O(n) — ~50ms at 100K embeddings | Drop in [sqlite-vec](https://github.com/asg017/sqlite-vec) for HNSW |
| FTS5 search | BM25 ranking, millisecond latency | Trigger-synced, zero manual maintenance |
| Concurrent readers | Unlimited with WAL mode | Built-in |
| Concurrent writers | Single writer at a time | Fine for embedded/single-process use |
| Database size | ~15MB per 100K episodes with 384-dim embeddings | Standard SQLite, no special storage |

### When to graduate

- **>100K entities with deep traversals** — consider Neo4j or Memgraph
- **Multi-user concurrent writes** — consider PostgreSQL with a graph schema
- **Complex pattern matching** — if you need Cypher regularly, use a graph database
- **Distributed systems** — SQLite doesn't replicate or shard

The schema maps directly to labeled property graphs — export to JSON and import into Neo4j, Memgraph, or DGraph when you outgrow this.

## Extracted from

This library powers the storage layer of [ctxgraph](https://github.com/rohansx/ctxgraph), a knowledge graph engine that achieves 0.800 combined F1 on extraction benchmarks with zero API calls. ctxgraph adds local ONNX-based NER (GLiNER) and relation extraction on top of sqlite-graph.

Read the full technical deep-dive: [We Replaced Neo4j with 45 SQL Statements](https://github.com/rohansx/ctxgraph/blob/main/docs/blog/we-replaced-neo4j-with-45-sql-statements.md)

## License

MIT
