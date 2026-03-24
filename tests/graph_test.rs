use sqlite_graph::{Edge, Entity, Episode, Graph};

#[test]
fn test_basic_operations() {
    let graph = Graph::in_memory().unwrap();

    // Add episode
    let ep = Episode::builder("Team decided to use Postgres for billing")
        .source("standup")
        .build();
    let result = graph.add_episode(ep).unwrap();
    assert!(!result.episode_id.is_empty());

    // Add entities
    let pg = Entity::new("PostgreSQL", "technology");
    let billing = Entity::new("Billing Service", "component");
    let pg_id = pg.id.clone();
    let billing_id = billing.id.clone();
    graph.add_entity(pg).unwrap();
    graph.add_entity(billing).unwrap();

    // Add edge
    let edge = Edge::new(&pg_id, &billing_id, "used_by");
    graph.add_edge(edge).unwrap();

    // Traverse
    let (entities, edges) = graph.traverse(&pg_id, 2).unwrap();
    assert_eq!(entities.len(), 2);
    assert_eq!(edges.len(), 1);

    // Stats
    let stats = graph.stats().unwrap();
    assert_eq!(stats.episode_count, 1);
    assert_eq!(stats.entity_count, 2);
    assert_eq!(stats.edge_count, 1);
}

#[test]
fn test_fts5_search() {
    let graph = Graph::in_memory().unwrap();

    let ep1 = Episode::builder("PostgreSQL chosen for billing service")
        .source("decision")
        .build();
    let ep2 = Episode::builder("Redis cache layer added for sessions")
        .source("decision")
        .build();
    graph.add_episode(ep1).unwrap();
    graph.add_episode(ep2).unwrap();

    let results = graph.search("PostgreSQL", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].0.content.contains("PostgreSQL"));

    let results = graph.search("Redis", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].0.content.contains("Redis"));
}

#[test]
fn test_entity_dedup() {
    let graph = Graph::in_memory().unwrap();

    let pg = Entity::new("PostgreSQL", "technology");
    let (id1, merged) = graph.add_entity_deduped(pg, 0.85).unwrap();
    assert!(!merged);

    // "Postgres" should match "PostgreSQL" with Jaro-Winkler >= 0.85
    let pg2 = Entity::new("Postgres", "technology");
    let (id2, merged) = graph.add_entity_deduped(pg2, 0.85).unwrap();
    assert!(merged);
    assert_eq!(id1, id2);

    // "Redis" should NOT match "PostgreSQL"
    let redis = Entity::new("Redis", "technology");
    let (id3, merged) = graph.add_entity_deduped(redis, 0.85).unwrap();
    assert!(!merged);
    assert_ne!(id1, id3);
}

#[test]
fn test_bi_temporal_edges() {
    let graph = Graph::in_memory().unwrap();

    let alice = Entity::new("Alice", "person");
    let google = Entity::new("Google", "company");
    let alice_id = alice.id.clone();
    let google_id = google.id.clone();
    graph.add_entity(alice).unwrap();
    graph.add_entity(google).unwrap();

    let mut edge = Edge::new(&alice_id, &google_id, "works_at");
    let edge_id = edge.id.clone();
    edge.fact = Some("Alice works at Google".to_string());
    graph.add_edge(edge).unwrap();

    // Edge should be current
    let edges = graph.get_edges_for_entity(&alice_id).unwrap();
    assert_eq!(edges.len(), 1);
    assert!(edges[0].is_current());

    // Invalidate it
    graph.invalidate_edge(&edge_id).unwrap();

    // Traversal with current_only should not find it
    let (entities, _) = graph.traverse(&alice_id, 2).unwrap();
    assert_eq!(entities.len(), 1); // only Alice, no Google

    // Traversal with history should find it
    let (entities, _) = graph.traverse_with_history(&alice_id, 2).unwrap();
    assert_eq!(entities.len(), 2);
}

#[test]
fn test_embeddings_and_rrf() {
    let graph = Graph::in_memory().unwrap();

    let ep = Episode::builder("Machine learning model deployed")
        .source("deploy")
        .build();
    let ep_id = ep.id.clone();
    graph.add_episode(ep).unwrap();

    // Store a dummy embedding
    let embedding = vec![0.1, 0.2, 0.3, 0.4];
    graph.store_embedding(&ep_id, &embedding).unwrap();

    // Retrieve embeddings
    let embeddings = graph.get_embeddings().unwrap();
    assert_eq!(embeddings.len(), 1);
    assert_eq!(embeddings[0].1, vec![0.1, 0.2, 0.3, 0.4]);

    // Fused search (will use FTS5 + semantic)
    let query_emb = vec![0.1, 0.2, 0.3, 0.4];
    let results = graph.search_fused("machine learning", &query_emb, 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_entity_context() {
    let graph = Graph::in_memory().unwrap();

    let center = Entity::new("API Gateway", "component");
    let auth = Entity::new("Auth Service", "component");
    let billing = Entity::new("Billing Service", "component");
    let center_id = center.id.clone();
    let auth_id = auth.id.clone();
    let billing_id = billing.id.clone();

    graph.add_entity(center).unwrap();
    graph.add_entity(auth).unwrap();
    graph.add_entity(billing).unwrap();

    graph.add_edge(Edge::new(&center_id, &auth_id, "routes_to")).unwrap();
    graph.add_edge(Edge::new(&center_id, &billing_id, "routes_to")).unwrap();

    let ctx = graph.get_entity_context(&center_id).unwrap();
    assert_eq!(ctx.entity.name, "API Gateway");
    assert_eq!(ctx.neighbors.len(), 2);
    assert_eq!(ctx.edges.len(), 2);
}

#[test]
fn test_open_or_create() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // Creates the database
    let graph = Graph::open_or_create(&db_path).unwrap();
    let ep = Episode::builder("test").build();
    graph.add_episode(ep).unwrap();
    drop(graph);

    // Reopens existing database
    let graph = Graph::open(&db_path).unwrap();
    let stats = graph.stats().unwrap();
    assert_eq!(stats.episode_count, 1);
}
