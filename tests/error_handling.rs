use github_stars_server::error::{GitHubStarsError, Result};
use std::error::Error;

#[test]
fn test_error_display() {
    let error = GitHubStarsError::RateLimitExceeded("Rate limit hit".to_string());
    assert_eq!(format!("{}", error), "Rate limit exceeded: Rate limit hit");
    
    let error = GitHubStarsError::NotFound("User not found".to_string());
    assert_eq!(format!("{}", error), "Resource not found: User not found");
    
    let error = GitHubStarsError::ApiError("API failed".to_string());
    assert_eq!(format!("{}", error), "GitHub API error: API failed");
    
    let error = GitHubStarsError::InvalidRepoUrl("Bad URL".to_string());
    assert_eq!(format!("{}", error), "Invalid repository URL: Bad URL");
}

#[test]
fn test_error_source() {
    let error = GitHubStarsError::RateLimitExceeded("Rate limit hit".to_string());
    assert!(error.source().is_none());
}

#[test]
fn test_error_conversion() {
    // Test that we can convert from other error types
    let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let error: GitHubStarsError = io_error.into();
    assert!(matches!(error, GitHubStarsError::IoError(_)));
}

#[test]
fn test_result_type() {
    fn returns_result() -> Result<String> {
        Ok("success".to_string())
    }
    
    let result = returns_result();
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "success");
    
    fn returns_error() -> Result<String> {
        Err(GitHubStarsError::NotFound("Not found".to_string()))
    }
    
    let result = returns_error();
    assert!(result.is_err());
}