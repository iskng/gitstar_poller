use serde::{Deserialize, Serialize};
use surrealdb::sql::Datetime;
use surrealdb::RecordId;

/// GitHub account information from SurrealDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: RecordId,
    #[serde(rename = "access_token")]
    pub access_token: String,
    #[serde(rename = "userId")]
    pub user_id: RecordId,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(rename = "providerAccountId")]
    pub provider_account_id: String,
    pub scope: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: Datetime,
    #[serde(rename = "updatedAt")]
    pub updated_at: Datetime,
}

/// User information from SurrealDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: RecordId,
    pub name: Option<String>,
    pub email: String,
    #[serde(rename = "emailVerified")]
    pub email_verified: bool,
    pub image: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: Datetime,
    #[serde(rename = "updatedAt")]
    pub updated_at: Datetime,
}

/// GitHub user stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubUser {
    pub id: RecordId,
    pub github_id: i64,
    pub username: String,
    pub avatar_url: String,
    pub profile_url: String,
    pub synced_at: Datetime,
}

/// Starred relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarredRelation {
    pub id: RecordId,
    pub r#in: RecordId,
    pub out: RecordId,
    pub starred_at: Datetime,
    pub source: String,
}

/// Repository information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub id: RecordId, // repo:owner/name
    pub github_id: i64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub url: String,
    pub stars: u32,
    pub language: Option<String>,
    pub owner: RepoOwner,
    pub is_private: bool,
    pub created_at: Datetime,
    pub updated_at: Datetime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoOwner {
    pub login: String,
    pub avatar_url: String,
}

/// Processing status for a repository
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProcessingStatus {
    pub id: RecordId, // processing_status:owner/name
    pub repo: RecordId, // repo:owner/name
    pub status: ProcessingStatus,
    #[serde(rename = "last_page_processed")]
    pub last_page_processed: u32,
    #[serde(rename = "total_pages")]
    pub total_pages: Option<u32>,
    #[serde(rename = "total_stargazers")]
    pub total_stargazers: Option<u32>,
    #[serde(rename = "processed_stargazers")]
    pub processed_stargazers: u32,
    #[serde(rename = "claimed_by")]
    pub claimed_by: Option<RecordId>,
    #[serde(rename = "claimed_at")]
    pub claimed_at: Option<Datetime>,
    #[serde(rename = "started_at")]
    pub started_at: Option<Datetime>,
    #[serde(rename = "completed_at")]
    pub completed_at: Option<Datetime>,
    #[serde(rename = "last_error")]
    pub last_error: Option<String>,
    #[serde(rename = "error_count")]
    pub error_count: u32,
    #[serde(rename = "next_retry_after")]
    pub next_retry_after: Option<Datetime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    RateLimited,
    PaginationLimited,
}

/// GitHub user (stargazer)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    pub id: RecordId, // github_user:username
    #[serde(rename = "githubId")]
    pub github_id: i64,
    pub username: String,
    #[serde(rename = "avatarUrl")]
    pub avatar_url: String,
    #[serde(rename = "profileUrl")]
    pub profile_url: String,
    #[serde(rename = "syncedAt")]
    pub synced_at: Datetime,
}

/// Event for new account creation
#[derive(Debug, Clone)]
pub struct NewAccountEvent {
    pub account: Account,
    pub user: User,
}

/// Token processing state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenProcessingState {
    pub id: RecordId, // token_state:userId
    pub user: RecordId, // user:id
    #[serde(rename = "rateLimitRemaining")]
    pub rate_limit_remaining: u32,
    #[serde(rename = "rateLimitReset")]
    pub rate_limit_reset: Datetime,
    #[serde(rename = "activeRepos")]
    pub active_repos: Vec<RecordId>,
    #[serde(rename = "lastActiveAt")]
    pub last_active_at: Datetime,
    pub status: TokenStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TokenStatus {
    Active,
    RateLimited,
    Idle,
    Error,
}

/// Rate limit state for an actor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitState {
    pub remaining: u32,
    pub limit: u32,
    pub reset_time: chrono::DateTime<chrono::Utc>,
    pub is_limited: bool,
}

impl Default for RateLimitState {
    fn default() -> Self {
        Self {
            remaining: 5000,
            limit: 5000,
            reset_time: chrono::Utc::now() + chrono::Duration::hours(1),
            is_limited: false,
        }
    }
}

/// Status of a token actor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenActorStatus {
    pub user_id: String,
    pub rate_limit: RateLimitState,
    pub current_repo: Option<String>,
    pub repos_in_queue: usize,
    pub repos_processed: usize,
    pub last_activity: chrono::DateTime<chrono::Utc>,
    pub status: TokenStatus,
}