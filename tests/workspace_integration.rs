#![cfg(feature = "postgres")]
//! Integration tests for the workspace module.
//!
//! Requires a running PostgreSQL with pgvector extension.
//! Set DATABASE_URL=postgres://localhost/thinclaw_test

use std::sync::Arc;

use thinclaw::db::{Database as _, postgres::PgBackend};
use thinclaw::workspace::{MockEmbeddings, SearchConfig, Workspace, paths};
use thinclaw_workspace::postgres_vector::{
    ANN_INDEXES, postgres_exact_vector_search_sql, postgres_vector_search_sql,
};

fn postgres_required() -> bool {
    std::env::var("THINCLAW_REQUIRE_POSTGRES_INTEGRATION")
        .ok()
        .as_deref()
        == Some("1")
}

fn postgres_unavailable(message: impl std::fmt::Display) -> Option<deadpool_postgres::Pool> {
    if postgres_required() {
        panic!("required PostgreSQL integration dependency is unavailable: {message}");
    }
    eprintln!("skipping workspace integration: {message}");
    None
}

async fn get_pool() -> Option<deadpool_postgres::Pool> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/thinclaw_test".to_string());

    let config: tokio_postgres::Config = database_url.parse().expect("Invalid DATABASE_URL");

    let mgr = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(mgr)
        .max_size(4)
        .build()
        .expect("Failed to create pool");
    if let Err(error) = pool.get().await {
        return postgres_unavailable(format!("database connection failed: {error}"));
    }
    let backend = PgBackend::from_pool(pool.clone());
    if let Err(error) = backend.run_migrations().await {
        return postgres_unavailable(format!("database migrations failed: {error}"));
    }
    Some(pool)
}

async fn cleanup_user(pool: &deadpool_postgres::Pool, user_id: &str) {
    let conn = pool.get().await.expect("Failed to get connection");
    conn.execute(
        "DELETE FROM memory_documents WHERE user_id = $1",
        &[&user_id],
    )
    .await
    .ok();
}

fn benchmark_sizes(name: &str, defaults: &[usize]) -> Vec<usize> {
    let mut sizes = std::env::var(name)
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    value
                        .parse::<usize>()
                        .unwrap_or_else(|error| panic!("invalid {name} value {value:?}: {error}"))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| defaults.to_vec());
    sizes.retain(|value| *value > 0);
    sizes.sort_unstable();
    sizes.dedup();
    assert!(!sizes.is_empty(), "{name} must contain a positive value");
    sizes
}

fn deterministic_unit_embedding(dimension: usize, prototype: usize) -> Vec<f32> {
    let mut state = (prototype as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut values = Vec::with_capacity(dimension);
    let mut norm_squared = 0.0_f32;
    for _ in 0..dimension {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let value = ((state >> 40) as f32 / ((1_u32 << 24) - 1) as f32) * 2.0 - 1.0;
        norm_squared += value * value;
        values.push(value);
    }
    let norm = norm_squared.sqrt();
    values.iter_mut().for_each(|value| *value /= norm);
    values
}

async fn seed_vector_benchmark_rows(
    conn: &tokio_postgres::Client,
    document_id: uuid::Uuid,
    identity_prefix: &str,
    model: &str,
    dimension: usize,
    start: usize,
    end: usize,
) {
    const PROTOTYPES: usize = 32;
    let end_inclusive = i32::try_from(end - 1).expect("benchmark row count exceeds i32");
    let step = PROTOTYPES as i32;
    for prototype in 0..PROTOTYPES.min(end) {
        let first = i32::try_from(start + prototype).expect("benchmark row count exceeds i32");
        if first > end_inclusive {
            continue;
        }
        let vector = pgvector::Vector::from(deterministic_unit_embedding(dimension, prototype));
        conn.execute(
            r#"
            INSERT INTO memory_chunks (
                id, document_id, chunk_index, content, embedding, embedding_model
            )
            SELECT md5($1 || ':' || chunk_index::text)::uuid,
                   $2, chunk_index, 'benchmark row ' || chunk_index::text, $6, $7
            FROM generate_series($3::integer, $4::integer, $5::integer) AS chunk_index
            "#,
            &[
                &identity_prefix,
                &document_id,
                &first,
                &end_inclusive,
                &step,
                &vector,
                &model,
            ],
        )
        .await
        .expect("seed deterministic vector benchmark rows");
    }
}

async fn explain_vector_query(
    conn: &tokio_postgres::Client,
    sql: &str,
    user_id: &str,
    query: &pgvector::Vector,
    model: &str,
) -> (f64, String) {
    let explain_sql = format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}");
    let agent_id = Option::<uuid::Uuid>::None;
    let path_prefixes = Vec::<String>::new();
    let model = Some(model);
    let row = conn
        .query_one(
            &explain_sql,
            &[&user_id, &agent_id, &query, &50_i64, &path_prefixes, &model],
        )
        .await
        .expect("execute vector benchmark query plan");
    let plan: serde_json::Value = row.get(0);
    let execution_ms = plan[0]["Execution Time"]
        .as_f64()
        .expect("EXPLAIN JSON execution time");
    let rendered = serde_json::to_string_pretty(&plan).expect("render EXPLAIN JSON");
    (execution_ms, rendered)
}

#[tokio::test]
async fn test_workspace_write_and_read() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_write_read";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write a file
    let doc = workspace
        .write("README.md", "# Hello World\n\nThis is a test.")
        .await
        .expect("Failed to write");

    assert_eq!(doc.path, "README.md");
    assert!(doc.content.contains("Hello World"));

    // Read it back
    let doc2 = workspace.read("README.md").await.expect("Failed to read");
    assert_eq!(doc2.content, "# Hello World\n\nThis is a test.");

    // Cleanup
    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_append() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_append";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write initial content
    workspace
        .write("notes.md", "Line 1")
        .await
        .expect("Failed to write");

    // Append more
    workspace
        .append("notes.md", "Line 2")
        .await
        .expect("Failed to append");

    // Read and verify
    let doc = workspace.read("notes.md").await.expect("Failed to read");
    assert_eq!(doc.content, "Line 1\nLine 2");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_nested_paths() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_nested";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write nested files
    workspace
        .write("projects/alpha/README.md", "# Alpha")
        .await
        .expect("Failed to write alpha");
    workspace
        .write("projects/alpha/notes.md", "Notes here")
        .await
        .expect("Failed to write notes");
    workspace
        .write("projects/beta/README.md", "# Beta")
        .await
        .expect("Failed to write beta");

    // List root
    let root = workspace.list("").await.expect("Failed to list root");
    assert_eq!(root.len(), 1); // just "projects/"
    assert!(root[0].is_directory);
    assert_eq!(root[0].name(), "projects");

    // List projects
    let projects = workspace
        .list("projects")
        .await
        .expect("Failed to list projects");
    assert_eq!(projects.len(), 2); // alpha/, beta/

    // List alpha
    let alpha = workspace
        .list("projects/alpha")
        .await
        .expect("Failed to list alpha");
    assert_eq!(alpha.len(), 2); // README.md, notes.md

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_delete() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_delete";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write and verify exists
    workspace
        .write("temp.md", "temporary")
        .await
        .expect("Failed to write");
    assert!(workspace.exists("temp.md").await.expect("exists failed"));

    // Delete
    workspace.delete("temp.md").await.expect("Failed to delete");

    // Verify gone
    assert!(!workspace.exists("temp.md").await.expect("exists failed"));

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_memory_operations() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_memory_ops";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Append to memory
    workspace
        .append_memory("User prefers dark mode")
        .await
        .expect("Failed to append memory");
    workspace
        .append_memory("User's timezone is PST")
        .await
        .expect("Failed to append memory");

    // Read memory
    let memory = workspace.memory().await.expect("Failed to get memory");
    assert!(memory.content.contains("dark mode"));
    assert!(memory.content.contains("PST"));
    // Entries should be separated by double newline
    assert!(memory.content.contains("\n\n"));

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_daily_log() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_daily_log";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Append to daily log (timestamped)
    workspace
        .append_daily_log("Started working on feature X")
        .await
        .expect("Failed to append daily log");

    // Read today's log
    let log = workspace
        .today_log()
        .await
        .expect("Failed to get today log");
    assert!(log.content.contains("feature X"));
    // Should have timestamp prefix like [HH:MM:SS]
    assert!(log.content.contains("["));

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_fts_search() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_fts_search";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write some documents
    workspace
        .write(
            "docs/authentication.md",
            "# Authentication\n\nThe system uses JWT tokens for authentication.",
        )
        .await
        .expect("write failed");
    workspace
        .write(
            "docs/database.md",
            "# Database\n\nWe use PostgreSQL with pgvector for vector search.",
        )
        .await
        .expect("write failed");
    workspace
        .write(
            "docs/api.md",
            "# API\n\nThe REST API uses JSON for request and response bodies.",
        )
        .await
        .expect("write failed");

    // Search for JWT (FTS only since no embeddings)
    let results = workspace
        .search_with_config("JWT authentication", SearchConfig::default().fts_only())
        .await
        .expect("search failed");

    assert!(!results.is_empty(), "Should find results for JWT");
    assert!(
        results[0].content.contains("JWT"),
        "Top result should contain JWT"
    );

    // Search for PostgreSQL
    let results = workspace
        .search_with_config("PostgreSQL database", SearchConfig::default().fts_only())
        .await
        .expect("search failed");

    assert!(!results.is_empty(), "Should find results for PostgreSQL");
    assert!(
        results[0].content.contains("PostgreSQL"),
        "Top result should contain PostgreSQL"
    );

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_hybrid_search_with_mock_embeddings() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_hybrid_search";
    cleanup_user(&pool, user_id).await;

    // Create workspace with mock embeddings (1536 dimensions to match OpenAI)
    let embeddings = Arc::new(MockEmbeddings::new(1536));
    let workspace = Workspace::new(user_id, pool.clone()).with_embeddings(embeddings);

    // Write documents
    workspace
        .write(
            "memory.md",
            "The user prefers dark mode and vim keybindings.",
        )
        .await
        .expect("write failed");
    workspace
        .write(
            "prefs.md",
            "Settings: theme=dark, editor=vim, font=monospace",
        )
        .await
        .expect("write failed");

    // Hybrid search
    let results = workspace
        .search("dark theme preference", 5)
        .await
        .expect("search failed");

    assert!(!results.is_empty(), "Should find results");
    // At least one result should be a hybrid match (found by both FTS and vector)
    // or we should have results from either method

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_supported_vector_queries_have_valid_hnsw_plans() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let conn = pool.get().await.expect("get PostgreSQL plan connection");
    conn.batch_execute("SET enable_seqscan = off")
        .await
        .expect("disable sequential plans for index contract");

    for index in ANN_INDEXES {
        let mut query = vec![0.0f32; index.dimension];
        query[0] = 1.0;
        let vector = pgvector::Vector::from(query);
        let sql = format!(
            "EXPLAIN (COSTS OFF, FORMAT TEXT) {}",
            postgres_vector_search_sql(index.dimension, false)
        );
        let rows = conn
            .query(
                &sql,
                &[
                    &"plan-contract-user",
                    &Option::<uuid::Uuid>::None,
                    &vector,
                    &5_i64,
                    &Vec::<String>::new(),
                    &Some("plan-contract-model"),
                ],
            )
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "EXPLAIN failed for {} dimensions ({}): {error}",
                    index.dimension, index.name
                )
            });
        let plan = rows
            .iter()
            .map(|row| row.get::<_, String>(0))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            plan.contains(index.name),
            "{}-dimension plan did not use {}:\n{}",
            index.dimension,
            index.name,
            plan
        );
    }

    conn.batch_execute("RESET enable_seqscan")
        .await
        .expect("restore planner setting");
}

/// Reproducible performance proof for issue #370.
///
/// The regular PostgreSQL CI leg compiles this benchmark and exercises the
/// production plans above. Nightly CI runs the ignored benchmark at 1k and
/// 10k rows for a small, medium, and large built-in embedding profile.
#[tokio::test]
#[ignore = "performance benchmark; run explicitly against an isolated PostgreSQL database"]
async fn benchmark_postgres_vector_ann_against_exact_scan() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let dimensions = benchmark_sizes("THINCLAW_VECTOR_BENCH_DIMENSIONS", &[384, 1536, 3072]);
    let row_counts = benchmark_sizes("THINCLAW_VECTOR_BENCH_ROW_COUNTS", &[1_000, 10_000]);
    let minimum_speedup = std::env::var("THINCLAW_VECTOR_BENCH_MIN_SPEEDUP")
        .ok()
        .map(|value| {
            value
                .parse::<f64>()
                .unwrap_or_else(|error| panic!("invalid benchmark speedup {value:?}: {error}"))
        })
        .unwrap_or(1.25);
    let largest_row_count = *row_counts.last().expect("non-empty benchmark row counts");

    for dimension in dimensions {
        let index = ANN_INDEXES
            .iter()
            .find(|index| index.dimension == dimension)
            .unwrap_or_else(|| {
                panic!("benchmark dimension {dimension} is not an indexed production profile")
            });
        let user_id = format!("vector_benchmark_{dimension}");
        let model = format!("benchmark-model-{dimension}");
        cleanup_user(&pool, &user_id).await;
        let conn = pool
            .get()
            .await
            .expect("get PostgreSQL benchmark connection");
        let document_id = uuid::Uuid::new_v4();
        conn.execute(
            r#"
            INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
            VALUES ($1, $2, NULL, $3, '', '{}'::jsonb)
            "#,
            &[&document_id, &user_id, &format!("bench/{dimension}.md")],
        )
        .await
        .expect("create vector benchmark document");

        let identity_prefix = format!("{user_id}:{document_id}");
        let query = pgvector::Vector::from(deterministic_unit_embedding(dimension, 0));
        let indexed_sql = postgres_vector_search_sql(dimension, false);
        let exact_sql = postgres_exact_vector_search_sql(dimension, false);
        let mut seeded = 0;

        for row_count in &row_counts {
            seed_vector_benchmark_rows(
                &conn,
                document_id,
                &identity_prefix,
                &model,
                dimension,
                seeded,
                *row_count,
            )
            .await;
            seeded = *row_count;
            conn.batch_execute("ANALYZE memory_chunks")
                .await
                .expect("analyze vector benchmark data");

            // Warm both code paths before recording three repetitions. Forcing
            // seqscan off proves and measures the intended ANN path; the exact
            // query uses a raw vector expression that cannot match the HNSW
            // expression index while retaining ordinary join indexes.
            conn.batch_execute("SET enable_seqscan = off")
                .await
                .expect("force ANN benchmark plan");
            let _ = explain_vector_query(&conn, &indexed_sql, &user_id, &query, &model).await;
            conn.batch_execute("RESET enable_seqscan")
                .await
                .expect("reset benchmark planner");
            let _ = explain_vector_query(&conn, &exact_sql, &user_id, &query, &model).await;

            let mut indexed_ms = f64::INFINITY;
            let mut exact_ms = f64::INFINITY;
            let mut indexed_plan = String::new();
            let mut exact_plan = String::new();
            for _ in 0..3 {
                conn.batch_execute("SET enable_seqscan = off")
                    .await
                    .expect("force ANN benchmark plan");
                let (elapsed, plan) =
                    explain_vector_query(&conn, &indexed_sql, &user_id, &query, &model).await;
                conn.batch_execute("RESET enable_seqscan")
                    .await
                    .expect("reset benchmark planner");
                if elapsed < indexed_ms {
                    indexed_ms = elapsed;
                    indexed_plan = plan;
                }
                let (elapsed, plan) =
                    explain_vector_query(&conn, &exact_sql, &user_id, &query, &model).await;
                if elapsed < exact_ms {
                    exact_ms = elapsed;
                    exact_plan = plan;
                }
            }

            assert!(
                indexed_plan.contains(index.name),
                "benchmark ANN plan did not use {}:\n{}",
                index.name,
                indexed_plan
            );
            assert!(
                !exact_plan.contains(index.name),
                "exact baseline unexpectedly used {}:\n{}",
                index.name,
                exact_plan
            );
            let speedup = exact_ms / indexed_ms;
            println!(
                "VECTOR_BENCHMARK dimension={dimension} rows={row_count} indexed_ms={indexed_ms:.3} exact_ms={exact_ms:.3} speedup={speedup:.2}x"
            );
            println!("VECTOR_BENCHMARK indexed_plan={indexed_plan}");
            println!("VECTOR_BENCHMARK exact_plan={exact_plan}");

            if *row_count == largest_row_count {
                assert!(
                    speedup >= minimum_speedup,
                    "{dimension}-dimension HNSW speedup {speedup:.2}x at {row_count} rows is below the required {minimum_speedup:.2}x"
                );
            }
        }
        cleanup_user(&pool, &user_id).await;
    }
}

#[tokio::test]
async fn test_postgres_vector_index_reconciliation_recovers_missing_index() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let index = ANN_INDEXES
        .iter()
        .find(|index| index.dimension == 384)
        .expect("384-dimension index policy");
    let conn = pool
        .get()
        .await
        .expect("get PostgreSQL recovery connection");
    conn.batch_execute(&format!("DROP INDEX CONCURRENTLY IF EXISTS {}", index.name))
        .await
        .expect("simulate interrupted/missing online index backfill");
    drop(conn);

    PgBackend::from_pool(pool.clone())
        .run_migrations()
        .await
        .expect("idempotent startup reconciliation should recreate the index");
    let conn = pool
        .get()
        .await
        .expect("get PostgreSQL verification connection");
    let valid: bool = conn
        .query_one(
            r#"
            SELECT pg_index.indisvalid AND pg_index.indisready
            FROM pg_class
            JOIN pg_namespace ON pg_namespace.oid = pg_class.relnamespace
            JOIN pg_index ON pg_index.indexrelid = pg_class.oid
            WHERE pg_namespace.nspname = current_schema()
              AND pg_class.relname = $1
            "#,
            &[&index.name],
        )
        .await
        .expect("reconciled index catalog row")
        .get(0);
    assert!(valid, "reconciled vector index must be ready and valid");
}

#[tokio::test]
async fn test_workspace_list_all() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let user_id = "test_list_all";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write files at various depths
    workspace.write("README.md", "root").await.unwrap();
    workspace.write("docs/intro.md", "intro").await.unwrap();
    workspace.write("docs/api/rest.md", "rest").await.unwrap();
    workspace.write("src/main.md", "main").await.unwrap();

    // List all
    let all = workspace.list_all().await.expect("list_all failed");
    assert_eq!(all.len(), 4);
    assert!(all.contains(&"README.md".to_string()));
    assert!(all.contains(&"docs/intro.md".to_string()));
    assert!(all.contains(&"docs/api/rest.md".to_string()));
    assert!(all.contains(&"src/main.md".to_string()));

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_system_prompt_separates_user_profile_from_trusted_instructions() {
    let Some(pool) = get_pool().await else {
        return;
    };
    let temp_home = tempfile::tempdir().expect("temp home");
    let previous_home = std::env::var_os("THINCLAW_HOME");
    unsafe {
        std::env::set_var("THINCLAW_HOME", temp_home.path());
    }
    let user_id = "test_system_prompt";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write identity files and the canonical home soul.
    workspace
        .write(
            paths::AGENTS,
            "## Session Startup\n\nYou are a helpful assistant.",
        )
        .await
        .unwrap();
    thinclaw::identity::soul_store::write_home_soul(
        &thinclaw::identity::soul::compose_seeded_soul("balanced").unwrap(),
    )
    .unwrap();
    workspace
        .write(paths::USER, "# USER.md\n\n- **Name:** Alice")
        .await
        .unwrap();

    // Get system prompt
    let prompt = workspace
        .system_prompt()
        .await
        .expect("system_prompt failed");

    assert!(
        prompt.contains("helpful assistant"),
        "Should include AGENTS.md content, got: {prompt}"
    );
    assert!(
        prompt.contains("## Soul"),
        "Should include SOUL.md content, got: {prompt}"
    );
    assert!(
        !prompt.contains("Alice"),
        "Trusted system prompt must not elevate actor-authored USER.md content: {prompt}"
    );
    let actor_overlay = workspace
        .actor_overlay_section(user_id)
        .await
        .expect("actor overlay lookup failed")
        .expect("USER.md should produce an actor overlay");
    assert!(
        actor_overlay.contains("Alice"),
        "Actor USER.md should remain available as untrusted overlay evidence: {actor_overlay}"
    );

    cleanup_user(&pool, user_id).await;
    if let Some(previous_home) = previous_home {
        unsafe {
            std::env::set_var("THINCLAW_HOME", previous_home);
        }
    } else {
        unsafe {
            std::env::remove_var("THINCLAW_HOME");
        }
    }
}
