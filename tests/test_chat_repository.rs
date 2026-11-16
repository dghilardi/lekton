mod common;

use chrono::{Duration, Utc};

use lekton::db::chat_models::{ChatMessage, ChatMessageStatus, ChatSession, SourceReference};
use lekton::db::chat_repository::{ChatRepository, MongoChatRepository};

fn message(
    id: &str,
    role: &str,
    content: &str,
    status: ChatMessageStatus,
    age_secs: i64,
) -> ChatMessage {
    ChatMessage {
        id: id.into(),
        session_id: "s1".into(),
        role: role.into(),
        content: content.into(),
        sources: None,
        status,
        cited_sources: None,
        created_at: Utc::now() - Duration::seconds(age_secs),
    }
}

#[tokio::test]
async fn message_status_is_persisted_and_unfinished_answers_can_be_deleted() {
    let env = common::TestEnv::start().await;
    let repo = MongoChatRepository::new(&env.db);

    repo.create_session(ChatSession {
        id: "s1".into(),
        user_id: "u1".into(),
        title: "Device service".into(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
    .await
    .unwrap();
    repo.add_message(message(
        "u1",
        "user",
        "Come funziona?",
        ChatMessageStatus::Complete,
        3,
    ))
    .await
    .unwrap();
    repo.add_message(message(
        "a1",
        "assistant",
        "Il",
        ChatMessageStatus::Interrupted,
        2,
    ))
    .await
    .unwrap();

    let messages = repo.get_messages("s1", 10).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].status, ChatMessageStatus::Interrupted);

    repo.delete_message("a1").await.unwrap();
    let messages = repo.get_messages("s1", 10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "u1");
}

#[tokio::test]
async fn messages_saved_before_the_status_field_read_as_complete() {
    let env = common::TestEnv::start().await;
    let repo = MongoChatRepository::new(&env.db);

    env.db
        .collection::<mongodb::bson::Document>("chat_messages")
        .insert_one(mongodb::bson::doc! {
            "id": "legacy",
            "session_id": "s1",
            "role": "assistant",
            "content": "Answer",
            "created_at": mongodb::bson::DateTime::now(),
        })
        .await
        .unwrap();

    let message = repo.get_message_by_id("legacy").await.unwrap().unwrap();
    assert_eq!(message.status, ChatMessageStatus::Complete);
    assert_eq!(
        message.cited_sources, None,
        "legacy answers show every source"
    );
}

#[tokio::test]
async fn cited_sources_and_source_numbers_are_persisted() {
    let env = common::TestEnv::start().await;
    let repo = MongoChatRepository::new(&env.db);

    let mut answer = message(
        "a1",
        "assistant",
        "Yes [2].",
        ChatMessageStatus::Complete,
        1,
    );
    answer.sources = Some(
        [1, 2]
            .map(|n| SourceReference {
                document_slug: format!("docs/{n}"),
                document_title: format!("Doc {n}"),
                section_title: None,
                section_anchor: None,
                score: 0.5,
                snippet: None,
                attachment_key: None,
                source_page: None,
                number: Some(n),
            })
            .to_vec(),
    );
    answer.cited_sources = Some(vec![2]);
    repo.add_message(answer).await.unwrap();

    let saved = repo.get_message_by_id("a1").await.unwrap().unwrap();
    assert_eq!(saved.cited_sources, Some(vec![2]));
    let numbers: Vec<_> = saved.sources.unwrap().iter().map(|s| s.number).collect();
    assert_eq!(numbers, [Some(1), Some(2)]);
}
