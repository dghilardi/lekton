mod common;

use chrono::Utc;

use lekton::db::learn_models::{
    GlossaryTerm, LearningPath, LearningRecord, LearningRecordKind, LearningScope, Lesson,
    QuizQuestion,
};

fn path(id: &str, user: &str) -> LearningPath {
    LearningPath {
        id: id.into(),
        user_id: user.into(),
        scope: LearningScope::Tag {
            tag: "kafka".into(),
        },
        title: "Kafka basics".into(),
        mission: None,
        covered_anchors: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn lesson(id: &str, path_id: &str, user: &str, seq: u32) -> Lesson {
    Lesson {
        id: id.into(),
        path_id: path_id.into(),
        user_id: user.into(),
        seq,
        title: format!("Lesson {seq}"),
        body_html: "<p>content</p>".into(),
        citations: vec![],
        primary_source: None,
        quiz: vec![QuizQuestion {
            prompt: "Q?".into(),
            options: vec!["a".into(), "b".into()],
            correct_index: 0,
            explanation: "because".into(),
        }],
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn path_lesson_record_crud_and_ordering() {
    let env = common::TestEnv::start().await;
    let repo = env.learn_repo.clone();

    repo.create_path(path("p1", "u1")).await.unwrap();
    // A second user's path must not leak into u1's listing.
    repo.create_path(path("p2", "u2")).await.unwrap();

    let fetched = repo.get_path("p1").await.unwrap().expect("path exists");
    assert_eq!(fetched.user_id, "u1");
    assert_eq!(
        fetched.scope,
        LearningScope::Tag {
            tag: "kafka".into()
        }
    );

    let mine = repo.list_paths_for_user("u1").await.unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, "p1");

    // Insert lessons out of order; listing must sort by seq ascending.
    repo.add_lesson(lesson("l2", "p1", "u1", 2)).await.unwrap();
    repo.add_lesson(lesson("l1", "p1", "u1", 1)).await.unwrap();
    let lessons = repo.list_lessons_for_path("p1").await.unwrap();
    assert_eq!(
        lessons.iter().map(|l| l.seq).collect::<Vec<_>>(),
        vec![1, 2]
    );

    let one = repo.get_lesson("l1").await.unwrap().expect("lesson exists");
    assert_eq!(one.quiz.len(), 1);

    repo.add_record(LearningRecord {
        id: "r1".into(),
        path_id: "p1".into(),
        lesson_id: Some("l1".into()),
        user_id: "u1".into(),
        kind: LearningRecordKind::QuizResult {
            per_question: vec![true],
            score: 1.0,
        },
        created_at: Utc::now(),
    })
    .await
    .unwrap();
    let records = repo.list_records_for_path("p1").await.unwrap();
    assert_eq!(records.len(), 1);

    // Progress update replaces covered anchors.
    repo.update_path_progress("p1", &["docs/kafka#intro".into()])
        .await
        .unwrap();
    let updated = repo.get_path("p1").await.unwrap().unwrap();
    assert_eq!(
        updated.covered_anchors,
        vec!["docs/kafka#intro".to_string()]
    );
}

#[tokio::test]
async fn delete_path_cascades_and_delete_all_for_user() {
    let env = common::TestEnv::start().await;
    let repo = env.learn_repo.clone();

    repo.create_path(path("p1", "u1")).await.unwrap();
    repo.add_lesson(lesson("l1", "p1", "u1", 1)).await.unwrap();
    repo.add_record(LearningRecord {
        id: "r1".into(),
        path_id: "p1".into(),
        lesson_id: Some("l1".into()),
        user_id: "u1".into(),
        kind: LearningRecordKind::Insight {
            text: "knows partitions".into(),
        },
        created_at: Utc::now(),
    })
    .await
    .unwrap();

    // delete_path removes the path and its dependent lessons and records.
    repo.delete_path("p1").await.unwrap();
    assert!(repo.get_path("p1").await.unwrap().is_none());
    assert!(repo.list_lessons_for_path("p1").await.unwrap().is_empty());
    assert!(repo.list_records_for_path("p1").await.unwrap().is_empty());

    // A second path for the same user, wiped by delete_all_for_user.
    repo.create_path(path("p3", "u1")).await.unwrap();
    repo.add_lesson(lesson("l3", "p3", "u1", 1)).await.unwrap();
    repo.delete_all_for_user("u1").await.unwrap();
    assert!(repo.list_paths_for_user("u1").await.unwrap().is_empty());
    assert!(repo.get_lesson("l3").await.unwrap().is_none());
}

#[tokio::test]
async fn persist_preference_defaults_true_and_roundtrips() {
    let env = common::TestEnv::start().await;
    let repo = env.learn_repo.clone();

    // Unset preference defaults to persisting.
    assert!(repo.get_persist("u1").await.unwrap());

    repo.set_persist("u1", false).await.unwrap();
    assert!(!repo.get_persist("u1").await.unwrap());

    // Upsert (not insert) on the second set.
    repo.set_persist("u1", true).await.unwrap();
    assert!(repo.get_persist("u1").await.unwrap());

    // Preferences are per-user.
    assert!(repo.get_persist("u2").await.unwrap());
}

#[tokio::test]
async fn glossary_upsert_is_stable_and_per_user_and_deletable() {
    let env = common::TestEnv::start().await;
    let repo = env.learn_repo.clone();

    let term = |t: &str, d: &str| GlossaryTerm {
        term: t.into(),
        definition: d.into(),
    };

    repo.upsert_glossary_terms(
        "u1",
        &[
            term("partition", "an ordered, append-only log"),
            term("broker", "a Kafka server"),
        ],
    )
    .await
    .unwrap();

    // Re-defining an existing term is a no-op: the first definition stands.
    repo.upsert_glossary_terms("u1", &[term("partition", "SOMETHING ELSE")])
        .await
        .unwrap();
    // Blank terms/definitions are ignored.
    repo.upsert_glossary_terms("u1", &[term("  ", "x"), term("empty", "  ")])
        .await
        .unwrap();

    let mut mine = repo.list_glossary("u1").await.unwrap();
    mine.sort_by(|a, b| a.term.cmp(&b.term));
    assert_eq!(mine.len(), 2);
    assert_eq!(mine[0].term, "broker");
    assert_eq!(mine[1].term, "partition");
    assert_eq!(mine[1].definition, "an ordered, append-only log");

    // Glossary is per-user.
    assert!(repo.list_glossary("u2").await.unwrap().is_empty());

    // Wiped by delete_all_for_user.
    repo.delete_all_for_user("u1").await.unwrap();
    assert!(repo.list_glossary("u1").await.unwrap().is_empty());
}
