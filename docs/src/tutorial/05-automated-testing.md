# Tutorial Part 5: Automated Testing

In Part 5, we write integration tests in Rust to test our endpoints, authentication, CSRF middleware, and database state transitions.

> [!NOTE]
> All test code in this part is adapted directly from [`examples/polls/tests/voting.rs`](file:///root/dev/Rango/examples/polls/tests/voting.rs).

---

## 1. Setting Up Integration Tests

Djangors applications leverage standard Rust integration test files in `tests/`. Create `tests/voting.rs`:

```rust,compile
use djangors_auth::{hash_password, User};
use djangors_test::TestDatabase;
use djangors_test::TestClient;
use djangors_orm::Model as _;
use polls::models::{Choice, Question};
use polls::urls;
```

---

## 2. Testing Database-Backed Routes

The test suite initializes an isolated database, seeds model data, and sends a request through the real router with `TestClient`. See the [real-socket testing guide](../guides/testing.md#2-real-socket-integration-testing) when a test needs to exercise TCP or Tower middleware:

```rust,compile
# use djangors_auth::{hash_password, User};
# use djangors_test::TestClient;
# use djangors_test::TestDatabase;
# use polls::models::{Choice, Question};
# use polls::urls;
# #[ignore]
#[tokio::test]
async fn test_polls_voting_integration() {
    let test_db = TestDatabase::isolated().await.unwrap();
    let db = test_db.database();

    // Clean up and recreate schema for isolated test execution
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS polls_choice").execute(db.pool()).await;
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS polls_question").execute(db.pool()).await;
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS auth_user").execute(db.pool()).await;

    djangors_orm::sqlx::query(
        "CREATE TABLE auth_user (
            id BIGSERIAL PRIMARY KEY,
            username VARCHAR(150) NOT NULL,
            email VARCHAR(254) NOT NULL,
            password TEXT NOT NULL,
            is_active BOOLEAN NOT NULL,
            is_staff BOOLEAN NOT NULL,
            is_superuser BOOLEAN NOT NULL,
            date_joined TIMESTAMPTZ NOT NULL,
            last_login TIMESTAMPTZ
        )",
    ).execute(db.pool()).await.unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE polls_question (
            id BIGSERIAL PRIMARY KEY,
            question_text VARCHAR(200) NOT NULL,
            pub_date TIMESTAMPTZ NOT NULL
        )",
    ).execute(db.pool()).await.unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE polls_choice (
            id BIGSERIAL PRIMARY KEY,
            question BIGINT NOT NULL REFERENCES polls_question(id) ON DELETE CASCADE,
            choice_text VARCHAR(200) NOT NULL,
            votes INTEGER NOT NULL DEFAULT 0
        )",
    ).execute(db.pool()).await.unwrap();

    // Seed test fixtures
    let now = chrono::Utc::now();
    let user = User {
        id: 0,
        username: "testuser".to_string(),
        email: "test@example.com".to_string(),
        password: hash_password("correct_password").unwrap(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    user.save(db).await.unwrap();

    let question = Question {
        id: 0,
        question_text: "What is your favorite color?".to_string(),
        pub_date: now,
    };
    let q_saved = question.save(db).await.unwrap();

    let choice = Choice {
        id: 0,
        question: djangors_orm::ForeignKey::new(q_saved.id),
        choice_text: "Blue".to_string(),
        votes: 0,
    };
    let c_saved = choice.save(db).await.unwrap();

    let response = TestClient::new(urls::urls())
        .get("/")
        .with_state(db.clone())
        .send()
        .await;
    response
        .assert_status(hyper::StatusCode::OK)
        .assert_contains("What is your favorite color?");
    test_db.cleanup().await.unwrap();
}
```

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **Async Test Runner**: Tests use `
