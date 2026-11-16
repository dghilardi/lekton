mod common;

use lekton::db::embedding_cache_repository::{
    EmbeddingCacheEntry, EmbeddingCacheRepository, MongoEmbeddingCacheRepository,
};
use mongodb::bson::doc;

async fn run_plan(db: &mongodb::Database) {
    lekton::db::migrations::build_plan()
        .run(db.clone())
        .await
        .expect("migration plan failed");
}

fn cache_entry(hash: &str, namespace: &str, dims: usize) -> EmbeddingCacheEntry {
    EmbeddingCacheEntry {
        hash: hash.to_string(),
        model: namespace.to_string(),
        embedding: vec![0.25; dims],
        generated_at: chrono::Utc::now(),
        text: None,
    }
}

/// Counting namespaces must not read the stored vectors: the aggregation is a
/// covered scan of the `(hash, model)` index, so the profiler records no
/// document examined for it.
#[tokio::test]
async fn embedding_cache_namespaces_are_counted_without_reading_vectors_and_purged() {
    let env = common::TestEnv::start().await;
    run_plan(&env.db).await;
    let repo = MongoEmbeddingCacheRepository::new(&env.db);

    let mut entries = Vec::new();
    for i in 0..3 {
        entries.push(cache_entry(&format!("h{i}"), "old-model|768|aaaa", 768));
    }
    for i in 0..2 {
        entries.push(cache_entry(&format!("h{i}"), "new-model|1024|bbbb", 1024));
    }
    repo.upsert_many(entries).await.unwrap();

    env.db
        .run_command(doc! { "profile": 2 })
        .await
        .expect("enable profiler");
    let stats = repo.namespace_stats().await.unwrap();
    env.db
        .run_command(doc! { "profile": 0 })
        .await
        .expect("disable profiler");

    let counts: Vec<(&str, u64)> = stats
        .iter()
        .map(|s| (s.namespace.as_str(), s.entries))
        .collect();
    assert_eq!(
        counts,
        vec![("new-model|1024|bbbb", 2), ("old-model|768|aaaa", 3)]
    );
    // A 1024-float entry is larger than a 768-float one, and both are at
    // least their vector's worth of BSON doubles.
    let bytes = |ns: &str| {
        stats
            .iter()
            .find(|s| s.namespace == ns)
            .unwrap()
            .approx_bytes
    };
    assert!(bytes("old-model|768|aaaa") >= 3 * 768 * 8);
    assert!(bytes("new-model|1024|bbbb") >= 2 * 1024 * 8);

    let group = env
        .db
        .collection::<mongodb::bson::Document>("system.profile")
        .find_one(doc! {
            "command.aggregate": "embedding_cache",
            "command.pipeline.0.$group": { "$exists": true },
        })
        .await
        .unwrap()
        .expect("the namespace count must have been profiled");
    let examined = group
        .get("docsExamined")
        .and_then(|v| v.as_i64().or_else(|| v.as_i32().map(i64::from)))
        .unwrap_or(-1);
    assert_eq!(
        examined, 0,
        "counting namespaces must not fetch documents (and their vectors): {group:?}"
    );

    let deleted = repo
        .delete_namespaces(&["old-model|768|aaaa".to_string()])
        .await
        .unwrap();
    assert_eq!(deleted, 3);
    let remaining: Vec<String> = repo
        .namespace_stats()
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.namespace)
        .collect();
    assert_eq!(remaining, vec!["new-model|1024|bbbb".to_string()]);
}
