use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitHubStarsError {
    #[error("GitHub API error: {0}")]
    ApiError(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),

    #[error("Invalid repository URL: {0}")]
    InvalidRepoUrl(String),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Environment error: {0}")]
    EnvError(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Resource not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, GitHubStarsError>;