use github_stars_server::github::GitHubClient;
use github_stars_server::error::GitHubStarsError;

fn get_test_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN").ok()
}

#[tokio::test]
async fn test_github_client_creation() {
    let client = GitHubClient::new("test_token".to_string());
    assert!(client.is_ok());
}

#[tokio::test]
#[ignore = "Requires valid GitHub token"]
async fn test_get_repository_info() {
    let token = get_test_token().expect("GITHUB_TOKEN not set");
    let client = GitHubClient::new(token).expect("Failed to create client");
    
    // Test with a well-known repository
    let repo = client.get_repository_info("rust-lang", "rust").await
        .expect("Failed to get repository info");
    
    assert_eq!(repo.name, "rust");
    assert_eq!(repo.full_name, "rust-lang/rust");
    assert!(repo.stargazers_count > 0);
    assert!(!repo.html_url.is_empty());
}

#[tokio::test]
#[ignore = "Requires valid GitHub token"]
async fn test_fetch_stargazers_page() {
    let token = get_test_token().expect("GITHUB_TOKEN not set");
    let client = GitHubClient::new(token).expect("Failed to create client");
    
    // Test with a small repository to avoid rate limits
    let (users, has_more, rate_limit) = client.fetch_stargazers_page("octocat/Hello-World", 1).await
        .expect("Failed to fetch stargazers");
    
    // Verify we got some users
    assert!(!users.is_empty(), "No stargazers found");
    
    // Verify user structure
    for user in &users {
        assert!(!user.login.is_empty());
        assert!(user.id > 0);
        assert!(!user.html_url.is_empty());
    }
    
    // Verify rate limit info
    assert!(rate_limit.limit > 0);
    assert!(rate_limit.remaining <= rate_limit.limit);
    assert!(!rate_limit.is_limited);
}

#[tokio::test]
async fn test_repository_not_found() {
    let client = GitHubClient::new("test_token".to_string()).expect("Failed to create client");
    
    let result = client.get_repository_info("nonexistent", "repository").await;
    
    assert!(result.is_err());
    match result.unwrap_err() {
        GitHubStarsError::NotFound(_) => {}, // Expected
        other => panic!("Expected NotFound error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_invalid_repo_format() {
    let client = GitHubClient::new("test_token".to_string()).expect("Failed to create client");
    
    let result = client.fetch_stargazers_page("invalid-format", 1).await;
    
    assert!(result.is_err());
    match result.unwrap_err() {
        GitHubStarsError::InvalidRepoUrl(_) => {}, // Expected
        other => panic!("Expected InvalidRepoUrl error, got: {:?}", other),
    }
}

#[tokio::test]
#[ignore = "Requires valid GitHub token"]
async fn test_rate_limit_handling() {
    let token = get_test_token().expect("GITHUB_TOKEN not set");
    let client = GitHubClient::new(token).expect("Failed to create client");
    
    // Make a request to check rate limit
    let (_, _, rate_limit) = client.fetch_stargazers_page("octocat/Hello-World", 1).await
        .expect("Failed to fetch stargazers");
    
    println!("Rate limit state:");
    println!("  Remaining: {}/{}", rate_limit.remaining, rate_limit.limit);
    println!("  Reset time: {}", rate_limit.reset_time);
    println!("  Is limited: {}", rate_limit.is_limited);
    
    assert!(rate_limit.remaining > 0, "Rate limit exhausted");
}

#[tokio::test]
#[ignore = "Requires valid GitHub token"]
async fn test_pagination() {
    let token = get_test_token().expect("GITHUB_TOKEN not set");
    let client = GitHubClient::new(token).expect("Failed to create client");
    
    // Test with a repository that has multiple pages of stargazers
    let mut all_users = Vec::new();
    let mut page = 1;
    let mut total_pages = 0;
    
    loop {
        let (users, has_more, _) = client.fetch_stargazers_page("facebook/react", page).await
            .expect("Failed to fetch stargazers");
        
        all_users.extend(users);
        total_pages += 1;
        
        if !has_more || total_pages >= 3 { // Limit to 3 pages for testing
            break;
        }
        
        page += 1;
    }
    
    println!("Fetched {} stargazers across {} pages", all_users.len(), total_pages);
    assert!(all_users.len() > 100, "Expected more than 100 stargazers");
    
    // Check for duplicates
    let mut seen_ids = std::collections::HashSet::new();
    for user in &all_users {
        assert!(seen_ids.insert(user.id), "Found duplicate user ID: {}", user.id);
    }
}