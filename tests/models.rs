use github_stars_server::models::{
    Account,
    User,
    Repo,
    RateLimitState,
    TokenActorStatus,
    TokenStatus,
    RepoOwner,
};
use surrealdb::sql::Datetime;
use surrealdb::RecordId;
use chrono::Utc;

#[test]
fn test_account_creation() {
    let now = Datetime::from(Utc::now());
    let account = Account {
        id: RecordId::from(("account", "123")),
        user_id: RecordId::from(("user", "456")),
        access_token: "token123".to_string(),
        provider_id: "github".to_string(),
        provider_account_id: "gh789".to_string(),
        scope: Some("repo,user".to_string()),
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    assert_eq!(account.access_token, "token123");
    assert_eq!(account.provider_id, "github");
    assert_eq!(account.provider_account_id, "gh789");
}

#[test]
fn test_user_creation() {
    let now = Datetime::from(Utc::now());
    let user = User {
        id: RecordId::from(("user", "123")),
        name: Some("Test User".to_string()),
        email: "test@example.com".to_string(),
        email_verified: true,
        image: Some("https://example.com/avatar.jpg".to_string()),
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    assert_eq!(user.email, "test@example.com");
    assert_eq!(user.name.unwrap(), "Test User");
    assert_eq!(user.image.unwrap(), "https://example.com/avatar.jpg");
    assert!(user.email_verified);
}

#[test]
fn test_repo_creation() {
    let now = Datetime::from(Utc::now());
    let repo = Repo {
        id: RecordId::from(("repo", "owner/test-repo")),
        github_id: 12345,
        name: "test-repo".to_string(),
        full_name: "owner/test-repo".to_string(),
        description: Some("Test repository".to_string()),
        url: "https://github.com/owner/test-repo".to_string(),
        stars: 100,
        language: Some("Rust".to_string()),
        owner: RepoOwner {
            login: "owner".to_string(),
            avatar_url: "https://github.com/owner.png".to_string(),
        },
        is_private: false,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    assert_eq!(repo.name, "test-repo");
    assert_eq!(repo.full_name, "owner/test-repo");
    assert_eq!(repo.stars, 100);
    assert_eq!(repo.owner.login, "owner");
    assert!(!repo.is_private);
}

#[test]
fn test_rate_limit_state() {
    let now = Utc::now();
    let rate_limit = RateLimitState {
        remaining: 100,
        limit: 5000,
        reset_time: now,
        is_limited: false,
    };

    assert_eq!(rate_limit.remaining, 100);
    assert_eq!(rate_limit.limit, 5000);
    assert_eq!(rate_limit.reset_time, now);
    assert!(!rate_limit.is_limited);

    // Test limited state
    let limited = RateLimitState {
        remaining: 0,
        limit: 5000,
        reset_time: now,
        is_limited: true,
    };

    assert_eq!(limited.remaining, 0);
    assert!(limited.is_limited);
}

#[test]
fn test_token_actor_status() {
    let now = Utc::now();
    let status = TokenActorStatus {
        user_id: "user:123".to_string(),
        rate_limit: RateLimitState {
            remaining: 4500,
            limit: 5000,
            reset_time: now,
            is_limited: false,
        },
        current_repo: Some("repo:owner/name".to_string()),
        repos_in_queue: 10,
        repos_processed: 5,
        last_activity: now,
        status: TokenStatus::Active,
    };

    assert_eq!(status.user_id, "user:123");
    assert_eq!(status.repos_processed, 5);
    assert_eq!(status.repos_in_queue, 10);
    assert_eq!(status.rate_limit.remaining, 4500);
    assert!(!status.rate_limit.is_limited);
    assert_eq!(status.status, TokenStatus::Active);
}

#[test]
fn test_serde_serialization() {
    use serde_json;

    let now = Datetime::from(Utc::now());
    // Test Account serialization
    let account = Account {
        id: RecordId::from(("account", "123")),
        user_id: RecordId::from(("user", "456")),
        access_token: "token123".to_string(),
        provider_id: "github".to_string(),
        provider_account_id: "gh789".to_string(),
        scope: Some("repo,user".to_string()),
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let json = serde_json::to_string(&account).unwrap();
    assert!(json.contains("\"tb\":\"account\""));
    assert!(json.contains("\"providerId\":\"github\""));
}

#[test]
fn test_rate_limit_default() {
    let default_rate_limit = RateLimitState::default();

    assert_eq!(default_rate_limit.remaining, 5000);
    assert_eq!(default_rate_limit.limit, 5000);
    assert!(!default_rate_limit.is_limited);
    // Reset time should be approximately 1 hour from now
    let time_diff = default_rate_limit.reset_time.signed_duration_since(Utc::now());
    assert!(time_diff.num_minutes() >= 59 && time_diff.num_minutes() <= 61);
}

#[test]
fn test_token_status_enum() {
    assert_eq!(TokenStatus::Active, TokenStatus::Active);
    assert_ne!(TokenStatus::Active, TokenStatus::RateLimited);

    // Test serialization
    let json = serde_json::to_string(&TokenStatus::Active).unwrap();
    assert_eq!(json, "\"active\"");

    let json = serde_json::to_string(&TokenStatus::RateLimited).unwrap();
    assert_eq!(json, "\"rate_limited\"");
}
