//! Integration tests for the credit budget's atomic refill-and-reserve.
//!
//! These exercise the MongoDB update pipeline, which is the only part of the
//! budget that cannot be checked without a real server: the whole design rests
//! on refill and reservation happening in one indivisible step.

mod common;

use std::sync::Arc;

use lekton::db::budget_repository::{BudgetRepository, MongoBudgetRepository};

const CAPACITY: f64 = 100.0;
/// Fast enough that a test can observe a refill without sleeping for long.
const REFILL_PER_HOUR: f64 = 3_600.0;

#[tokio::test]
async fn a_first_time_caller_starts_with_a_full_bucket() {
    let env = common::TestEnv::start().await;
    let repo = MongoBudgetRepository::new(&env.db);

    let reservation = repo
        .reserve("user:new", 10.0, CAPACITY, 0.0)
        .await
        .expect("reserve");

    assert!(reservation.granted);
    assert!(
        (reservation.balance - 90.0).abs() < 0.5,
        "a new caller should start full, got {}",
        reservation.balance
    );
}

#[tokio::test]
async fn reservations_accumulate_until_the_bucket_is_empty() {
    let env = common::TestEnv::start().await;
    let repo = MongoBudgetRepository::new(&env.db);

    // No refill, so the bucket only goes down.
    for _ in 0..10 {
        let reservation = repo
            .reserve("user:steady", 10.0, CAPACITY, 0.0)
            .await
            .expect("reserve");
        assert!(reservation.granted, "the first ten should all fit");
    }

    let overdraft = repo
        .reserve("user:steady", 10.0, CAPACITY, 0.0)
        .await
        .expect("reserve");

    assert!(
        !overdraft.granted,
        "the eleventh must not fit, balance was {}",
        overdraft.balance
    );
}

#[tokio::test]
async fn concurrent_reservations_never_overspend() {
    // The reason the refill lives in an update pipeline: with a read-then-write
    // these would all observe a full bucket and collectively overdraw.
    let env = common::TestEnv::start().await;
    let repo = Arc::new(MongoBudgetRepository::new(&env.db));

    let attempts = (0..40).map(|_| {
        let repo = repo.clone();
        tokio::spawn(async move { repo.reserve("user:racer", 10.0, CAPACITY, 0.0).await })
    });

    let mut granted = 0;
    for attempt in attempts {
        if attempt.await.expect("task").expect("reserve").granted {
            granted += 1;
        }
    }

    assert_eq!(
        granted, 10,
        "a 100-credit bucket must grant exactly ten 10-credit reservations"
    );

    let balance = repo
        .balance("user:racer")
        .await
        .expect("balance")
        .expect("caller exists");
    assert!(
        balance >= 0.0,
        "a refusal must leave the balance untouched, never negative — got {balance}"
    );
}

#[tokio::test]
async fn a_release_returns_credits_but_never_past_capacity() {
    let env = common::TestEnv::start().await;
    let repo = MongoBudgetRepository::new(&env.db);

    repo.reserve("user:settler", 40.0, CAPACITY, 0.0)
        .await
        .expect("reserve");

    // Settling a call that turned out to cost 10 rather than the 40 reserved.
    repo.release("user:settler", 30.0, CAPACITY)
        .await
        .expect("release");
    let balance = repo.balance("user:settler").await.expect("balance");
    assert!((balance.unwrap() - 90.0).abs() < 0.5, "got {balance:?}");

    // A duplicated or spurious release must not mint credits.
    repo.release("user:settler", 1_000.0, CAPACITY)
        .await
        .expect("release");
    let balance = repo.balance("user:settler").await.expect("balance");
    assert!(
        (balance.unwrap() - CAPACITY).abs() < 0.5,
        "the balance must clamp to capacity, got {balance:?}"
    );
}

#[tokio::test]
async fn the_bucket_refills_over_elapsed_time() {
    let env = common::TestEnv::start().await;
    let repo = MongoBudgetRepository::new(&env.db);

    // Drain it.
    repo.reserve("user:waiter", CAPACITY, CAPACITY, REFILL_PER_HOUR)
        .await
        .expect("reserve");
    let drained = repo
        .balance("user:waiter")
        .await
        .expect("balance")
        .expect("exists");
    assert!(drained < 1.0, "should be empty, got {drained}");

    // At 3600 credits/hour, one second is worth one credit.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let reservation = repo
        .reserve("user:waiter", 1.0, CAPACITY, REFILL_PER_HOUR)
        .await
        .expect("reserve");

    assert!(
        reservation.granted,
        "elapsed time should have refilled enough for one credit, balance {}",
        reservation.balance
    );
}
