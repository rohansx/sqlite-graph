//! # sqlite-graph
//!
//! An embeddable graph database built entirely on SQLite.
//!
//! - **Recursive CTE traversal** — multi-hop graph walks in a single SQL statement
//! - **Bi-temporal edges** — facts have valid time (real-world) and recorded time (system)
//! - **FTS5 full-text search** — across episodes, entities, and edges
//! - **Hybrid search** — FTS5 + vector cosine similarity fused with Reciprocal Rank Fusion
//! - **Entity deduplication** — Jaro-Winkler fuzzy matching with alias resolution
//! - **Single file** — no Docker, no JVM, no network hop
//!
//! ## Quick start
//!
//! ```rust
//! use sqlite_graph::{Graph, Episode, Entity, Edge};
//!
//! let graph = Graph::in_memory().unwrap();
//!
//! // Add an episode (an event, decision, or message)
//! let ep = Episode::builder("Team decided to use Postgres for billing")
//!     .source("standup")
//!     .build();
//! graph.add_episode(ep).unwrap();
//!
//! // Add entities and edges manually
//! let pg = Entity::new("PostgreSQL", "technology");
//! let billing = Entity::new("Billing Service", "component");
//! graph.add_entity(pg.clone()).unwrap();
//! graph.add_entity(billing.clone()).unwrap();
//!
//! let edge = Edge::new(&pg.id, &billing.id, "used_by");
//! graph.add_edge(edge).unwrap();
//!
//! // Traverse the graph
//! let (entities, edges) = graph.traverse(&pg.id, 2).unwrap();
//! assert_eq!(entities.len(), 2);
//! ```

mod error;
mod graph;
mod storage;
mod types;

pub use error::{Error, Result};
pub use graph::Graph;
pub use types::*;
