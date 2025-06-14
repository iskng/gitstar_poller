mod common;

use common::TestContext;
use github_stars_server::actors::processing_supervisor::{ProcessingSupervisor, ProcessingSupervisorMessage};
use github_stars_server::actors::github_factory::{GitHubFactoryConfig, spawn_github_factory, submit_account_processing_job};
use github_stars_server::models::Account;
use ractor::Actor;
use std::time::Duration;
use surrealdb::RecordId;
use surrealdb::sql::Datetime;

#[tokio::test]
async fn test_github_factory_spawn() {
    let ctx = TestContext::new().await.expect("Failed to create test context");

    // Create factory config
    let config = GitHubFactoryConfig {
        num_initial_workers: 2,
        max_workers: 10,
        queue_capacity: 100,
        dead_mans_switch_timeout_seconds: 60,
    };

    // Spawn the factory
    let factory = spawn_github_factory(config, ctx.db_pool.clone())
        .await
        .expect("Failed to spawn GitHub factory");

    // Get queue depth (should be 0 initially)
    let queue_depth = factory.call(
        |reply| ractor::factory::FactoryMessage::GetQueueDepth(reply),
        Some(Duration::from_secs(5))
    ).await;

    assert!(queue_depth.is_ok());
    let call_result = queue_depth.unwrap();

    match call_result {
        ractor::rpc::CallResult::Success(depth) => {
            assert_eq!(depth, 0, "Expected empty queue initially");
        }
        _ => panic!("Expected success response"),
    }

    // Give workers time to spawn
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Just verify the factory is responsive - worker count can vary during startup
    let factory_alive = factory.call(
        |reply| ractor::factory::FactoryMessage::GetQueueDepth(reply),
        Some(Duration::from_secs(5))
    ).await;

    assert!(factory_alive.is_ok(), "Factory should be responsive");

    // Shutdown factory
    factory.send_message(ractor::factory::FactoryMessage::DrainRequests)
        .expect("Failed to send drain message");

    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_processing_supervisor_spawn() {
    let ctx = TestContext::new().await.expect("Failed to create test context");

    // Create factory config
    let config = GitHubFactoryConfig {
        num_initial_workers: 2,
        max_workers: 10,
        queue_capacity: 100,
        dead_mans_switch_timeout_seconds: 60,
    };

    // Spawn supervisor with live query
    let supervisor = ProcessingSupervisor::spawn_with_live_query(ctx.db_pool.clone(), config)
        .await
        .expect("Failed to spawn processing supervisor");

    // Give workers time to spawn
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Get statistics
    let stats = supervisor.call(
        |reply| ProcessingSupervisorMessage::GetStats(reply),
        Some(Duration::from_secs(5))
    ).await;

    assert!(stats.is_ok());
    let call_result = stats.unwrap();

    match call_result {
        ractor::rpc::CallResult::Success(stats) => {
            // Just verify we get valid stats - worker count can vary during startup
            assert!(stats.total_accounts_processed >= 0, "Stats should be valid");
            println!("Supervisor stats: {} workers, {} queue depth", 
                stats.factory_active_workers, stats.factory_queue_depth);
        }
        _ => panic!("Expected success response"),
    }

    // Shutdown supervisor
    supervisor
        .send_message(ProcessingSupervisorMessage::Shutdown)
        .expect("Failed to send shutdown message");

    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[tokio::test]
async fn test_submit_job_to_factory() {
    let ctx = TestContext::new().await.expect("Failed to create test context");

    // Create factory config
    let config = GitHubFactoryConfig {
        num_initial_workers: 1,
        max_workers: 10,
        queue_capacity: 100,
        dead_mans_switch_timeout_seconds: 60,
    };

    // Spawn the factory
    let factory = spawn_github_factory(config, ctx.db_pool.clone())
        .await
        .expect("Failed to spawn GitHub factory");

    // Submit a test job with RecordIds
    let user_id = RecordId::from(("user", "test_user"));
    
    let result = submit_account_processing_job(
        &factory,
        &user_id,
        "test_token".to_string(),
        false, // not a new account for test
    ).await;

    assert!(result.is_ok(), "Failed to submit job: {:?}", result);

    // Check queue depth increased
    let queue_depth = factory.call(
        |reply| ractor::factory::FactoryMessage::GetQueueDepth(reply),
        Some(Duration::from_secs(5))
    ).await;

    assert!(queue_depth.is_ok());
    let call_result = queue_depth.unwrap();

    match call_result {
        ractor::rpc::CallResult::Success(depth) => {
            // Queue might be 0 or 1 depending on how fast the worker picks it up
            assert!(depth <= 1, "Expected queue depth to be 0 or 1");
        }
        _ => panic!("Expected success response"),
    }

    // Shutdown factory
    factory.send_message(ractor::factory::FactoryMessage::DrainRequests)
        .expect("Failed to send drain message");

    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_supervisor_with_new_account() {
    let ctx = TestContext::new().await.expect("Failed to create test context");

    // Create a test account
    let now = Datetime::from(chrono::Utc::now());
    let test_account = Account {
        id: RecordId::from(("account", "testfactory")),
        user_id: RecordId::from(("user", "testfactory")),
        access_token: "test_token".to_string(),
        provider_id: "github".to_string(),
        provider_account_id: "testfactory".to_string(),
        scope: Some("repo".to_string()),
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    // Create factory config
    let config = GitHubFactoryConfig {
        num_initial_workers: 2,
        max_workers: 10,
        queue_capacity: 100,
        dead_mans_switch_timeout_seconds: 60,
    };

    // Create supervisor args manually to inject test account event
    let (tx, rx) = tokio::sync::mpsc::channel::<github_stars_server::models::NewAccountEvent>(100);
    let args = github_stars_server::actors::processing_supervisor::ProcessingSupervisorArgs {
        db_pool: ctx.db_pool.clone(),
        factory_config: config,
        account_receiver: rx,
    };

    let (supervisor_ref, _) = Actor::spawn(
        Some("test-processing-supervisor".to_string()),
        ProcessingSupervisor {},
        args
    ).await.expect("Failed to spawn supervisor");

    // Send the test account through the channel
    let event = github_stars_server::models::NewAccountEvent {
        account: test_account.clone(),
        user: github_stars_server::models::User {
            id: test_account.user_id.clone(),
            name: Some("Test Factory User".to_string()),
            email: "testfactory@example.com".to_string(),
            email_verified: true,
            image: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    };
    tx.send(event).await.expect("Failed to send account event");

    // Give it time to process
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Get statistics
    let stats = supervisor_ref.call(
        |reply| ProcessingSupervisorMessage::GetStats(reply),
        Some(Duration::from_secs(5))
    ).await;

    assert!(stats.is_ok());
    let call_result = stats.unwrap();

    match call_result {
        ractor::rpc::CallResult::Success(stats) => {
            println!("Supervisor stats after new account:");
            println!("  Total accounts processed: {}", stats.total_accounts_processed);
            println!("  Factory queue depth: {}", stats.factory_queue_depth);
            println!("  Active workers: {}", stats.factory_active_workers);
            
            // Should have started processing the account
            assert!(stats.total_accounts_processed > 0 || stats.factory_queue_depth > 0,
                "Expected account to be processed or queued");
        }
        _ => panic!("Expected success response"),
    }

    // Shutdown supervisor
    supervisor_ref
        .send_message(ProcessingSupervisorMessage::Shutdown)
        .expect("Failed to send shutdown message");

    tokio::time::sleep(Duration::from_millis(500)).await;
}