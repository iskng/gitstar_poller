mod common;

use common::TestContext;
use surrealdb::RecordId;

#[tokio::test]
async fn test_database_connection() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let _db_client = ctx.db_client.lock().await;
    
    // If we can create the context, the connection is working
    // The SurrealClient doesn't expose direct query access in tests
}

#[tokio::test]
async fn test_get_github_accounts() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    // Get GitHub accounts with a limit and offset
    let accounts = db_client.get_github_accounts(10, 0).await;
    
    // This might return empty if no accounts exist
    assert!(accounts.is_ok(), "Failed to get GitHub accounts");
}

#[tokio::test]
#[ignore = "Method not exposed in current implementation"]
async fn test_get_user_info() {
    // The get_user_info method is not exposed in the current SurrealClient implementation
    // This test would need the method to be added to test user retrieval
}

#[tokio::test]
async fn test_get_repos_for_processing() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    // Test with a sample user ID
    let test_user_id = RecordId::from(("user", "123"));
    let repos = db_client.get_repos_for_processing(&test_user_id).await;
    
    // This might return empty if no repos need processing
    assert!(repos.is_ok(), "Failed to get repos for processing");
}

#[tokio::test]
async fn test_claim_repo_for_processing() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    // Use test IDs
    let test_repo_id = RecordId::from(("repo", "owner/test-repo"));
    let test_user_id = RecordId::from(("user", "123"));
    
    // Try to claim the repo
    let claimed = db_client.claim_repo_for_processing(&test_repo_id, &test_user_id).await
        .expect("Failed to claim repo");
    
    // The claim might succeed or fail depending on the repo's current state
    // This is fine for integration testing
    println!("Repo claim result: {}", claimed);
}

#[tokio::test]
async fn test_update_repo_processing_progress() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    let test_repo_id = RecordId::from(("repo", "owner/test-repo"));
    
    // Update processing progress
    db_client.update_repo_processing_progress(&test_repo_id, 1, 100).await
        .expect("Failed to update repo processing progress");
}

#[tokio::test]
async fn test_insert_stargazers_batch() {
    use github_stars_server::types::GitHubUser;
    
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    let test_repo_id = RecordId::from(("repo", "owner/test-repo"));
    
    // Create test stargazer data
    let stargazers = vec![
        GitHubUser {
            login: "testuser1".to_string(),
            id: 12345,
            avatar_url: Some("https://example.com/avatar1.jpg".to_string()),
            html_url: "https://github.com/testuser1".to_string(),
        },
        GitHubUser {
            login: "testuser2".to_string(), 
            id: 67890,
            avatar_url: Some("https://example.com/avatar2.jpg".to_string()),
            html_url: "https://github.com/testuser2".to_string(),
        },
    ];
    
    // Save stargazers
    db_client.insert_stargazers_batch(&test_repo_id, &stargazers).await
        .expect("Failed to insert stargazers batch");
}

#[tokio::test]
async fn test_setup_account_live_query() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    // Set up live query
    let mut live_stream = db_client.setup_account_live_query().await
        .expect("Failed to setup live query");
    
    // The live query should be established successfully
    // We can't easily test new accounts appearing without modifying the database,
    // but we can verify the query was set up
    
    // Try to receive with a timeout to ensure channel is working
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        live_stream.recv()
    ).await;
    
    // It's OK if we timeout - that means no new accounts were created
    assert!(result.is_err() || result.unwrap().is_some());
}

#[tokio::test]
#[ignore = "Direct database access not exposed"]
async fn test_account_structure() {
    // Direct database queries are not exposed in the current implementation
    // This test would need raw query access to verify database structure
}

#[tokio::test]
async fn test_processing_stats() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    // Get processing statistics
    let stats = db_client.get_processing_stats().await
        .expect("Failed to get processing stats");
    
    // Stats should have valid fields
    println!("Processing stats - Completed: {}, Failed: {}, Processing: {}", 
        stats.completed, stats.failed, stats.processing);
}

#[tokio::test]
async fn test_mark_repo_complete() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    let test_repo_id = RecordId::from(("repo", "owner/test-repo"));
    
    // Mark repo as complete
    db_client.mark_repo_processing_complete(&test_repo_id, 1000).await
        .expect("Failed to mark repo complete");
}

#[tokio::test]
async fn test_mark_repo_failed() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    let test_repo_id = RecordId::from(("repo", "owner/test-repo"));
    
    // Mark repo as failed
    db_client.mark_repo_processing_failed(&test_repo_id, "Test error").await
        .expect("Failed to mark repo failed");
}

#[tokio::test]
async fn test_update_repo_rate_limited() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    let test_repo_id = RecordId::from(("repo", "owner/test-repo"));
    let retry_time = chrono::Utc::now() + chrono::Duration::hours(1);
    
    // Update rate limited status
    db_client.update_repo_rate_limited(&test_repo_id, retry_time).await
        .expect("Failed to update rate limited status");
}

#[tokio::test]
async fn test_reset_stale_claims() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    // Reset stale claims older than 60 minutes
    let reset_count = db_client.reset_stale_claims(60).await
        .expect("Failed to reset stale claims");
    
    // The count could be 0 if no stale claims exist
    println!("Reset {} stale claims", reset_count);
}

#[tokio::test]
async fn test_get_repos_processing_status() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let db_client = ctx.db_client.lock().await;
    
    // Test with some repo IDs
    let repo_ids = vec![
        "owner1/repo1".to_string(),
        "owner2/repo2".to_string(),
    ];
    
    let statuses = db_client.get_repos_processing_status(&repo_ids).await
        .expect("Failed to get processing statuses");
    
    // The result might be empty if these repos don't exist
    println!("Found {} processing statuses", statuses.len());
}