use crate::error::{GitHubStarsError, Result};
use crate::types::{GitHubRepo, GitHubUser};
use crate::models::RateLimitState;
use chrono::{DateTime, Utc};
use reqwest::{Client, Response};
use std::time::{Duration, SystemTime};
use tokio::time::sleep;

const API_BASE_URL: &str = "https://api.github.com";
const PER_PAGE: u32 = 100;
const MAX_RETRIES: u32 = 3;

pub struct GitHubClient {
    client: Client,
    token: String,
}

impl GitHubClient {
    pub fn new(token: String) -> Result<Self> {
        let client = Client::builder()
            .user_agent("GitHub Stars Server/0.1.0")
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(GitHubClient { client, token })
    }

    async fn make_request(&self, url: &str) -> Result<Response> {
        let mut retries = 0;

        loop {
            let response = self
                .client
                .get(url)
                .header("Accept", "application/vnd.github.v3+json")
                .header("Authorization", format!("token {}", self.token))
                .send()
                .await?;

            let rate_limit_remaining = response
                .headers()
                .get("X-RateLimit-Remaining")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);

            let rate_limit_reset = response
                .headers()
                .get("X-RateLimit-Reset")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            match response.status() {
                reqwest::StatusCode::OK => {
                    // Add delay if rate limit is low
                    if rate_limit_remaining < 10 {
                        eprintln!("⚠️ Rate limit low ({} remaining). Adding delay...", rate_limit_remaining);
                        sleep(Duration::from_secs(1)).await;
                    }
                    return Ok(response);
                }
                reqwest::StatusCode::NOT_FOUND => {
                    return Err(GitHubStarsError::NotFound(format!("Resource not found: {}", url)));
                }
                reqwest::StatusCode::FORBIDDEN => {
                    if rate_limit_remaining == 0 {
                        let reset_time = SystemTime::UNIX_EPOCH + Duration::from_secs(rate_limit_reset);
                        let wait_time = reset_time.duration_since(SystemTime::now()).unwrap_or(Duration::from_secs(0));
                        
                        if wait_time > Duration::from_secs(60) {
                            return Err(GitHubStarsError::RateLimitExceeded(
                                format!("API rate limit exceeded. Reset at: {:?}", reset_time)
                            ));
                        } else {
                            eprintln!("⏱️ Rate limit reached. Waiting {} seconds...", wait_time.as_secs() + 1);
                            sleep(wait_time + Duration::from_secs(1)).await;
                            continue;
                        }
                    } else {
                        let error_text = response.text().await.unwrap_or_default();
                        return Err(GitHubStarsError::ApiError(format!("Forbidden: {}", error_text)));
                    }
                }
                status if status.is_server_error() && retries < MAX_RETRIES => {
                    eprintln!("⚠️ Server error ({}). Retrying in 2 seconds...", status);
                    sleep(Duration::from_secs(2)).await;
                    retries += 1;
                    continue;
                }
                status => {
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(GitHubStarsError::ApiError(
                        format!("API request failed with status {}: {}", status, error_text)
                    ));
                }
            }
        }
    }

    pub async fn get_repository_info(&self, owner: &str, repo: &str) -> Result<GitHubRepo> {
        let url = format!("{}/repos/{}/{}", API_BASE_URL, owner, repo);
        let response = self.make_request(&url).await?;
        let repo_data: GitHubRepo = response.json().await?;
        Ok(repo_data)
    }

    /// Get current rate limit state from last response
    pub fn get_rate_limit_state(&self, response: &Response) -> RateLimitState {
        let headers = response.headers();
        
        let remaining = headers
            .get("X-RateLimit-Remaining")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
            
        let limit = headers
            .get("X-RateLimit-Limit")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(5000);
            
        let reset = headers
            .get("X-RateLimit-Reset")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .map(|timestamp| DateTime::from_timestamp(timestamp, 0).unwrap_or_else(|| Utc::now()))
            .unwrap_or_else(|| Utc::now() + chrono::Duration::hours(1));
            
        RateLimitState {
            remaining,
            limit,
            reset_time: reset,
            is_limited: remaining == 0,
        }
    }

    /// Fetch a page of stargazers for a repository
    pub async fn fetch_stargazers_page(
        &self,
        repo_full_name: &str,
        page: u32,
    ) -> Result<(Vec<GitHubUser>, bool, RateLimitState)> {
        let parts: Vec<&str> = repo_full_name.split('/').collect();
        if parts.len() != 2 {
            return Err(GitHubStarsError::InvalidRepoUrl(
                format!("Invalid repository name format: {}", repo_full_name)
            ));
        }
        
        let owner = parts[0];
        let repo = parts[1];
        
        let url = format!(
            "{}/repos/{}/{}/stargazers?per_page={}&page={}",
            API_BASE_URL, owner, repo, PER_PAGE, page
        );

        let response = self.make_request(&url).await?;
        let rate_limit = self.get_rate_limit_state(&response);
        
        let users: Vec<GitHubUser> = response.json().await?;
        let has_more = users.len() == PER_PAGE as usize;
        
        Ok((users, has_more, rate_limit))
    }
}