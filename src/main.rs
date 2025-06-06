mod actors;
mod cli;
mod error;
mod github;
mod models;
mod pool;
mod surreal_client;
mod types;

use actors::{ ProcessingSupervisor, ProcessingSupervisorMessage };
use actors::github_factory::GitHubFactoryConfig;
use clap::Parser;
use cli::Cli;
use colored::*;
use error::{ GitHubStarsError, Result };
use pool::{ create_pool, PoolConfig, SurrealConnectionConfig };
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if it exists
    dotenv::dotenv().ok();

    // Initialize tracing with DEBUG level by default
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("debug"))
        )
        .init();

    let mut cli = Cli::parse();

    // Override db_url if --local flag is set
    if cli.local {
        cli.db_url = "ws://localhost:8000".to_string();
        println!("{}", "Running in local mode (DB URL: ws://localhost:8000)".yellow());
    }

    println!("{}", "GitHub Stars Processing Server".bold().green());
    println!("{}\n", "=".repeat(50).dimmed());

    // Configure connection pool
    let connection_config = SurrealConnectionConfig {
        url: cli.db_url.clone(),
        username: cli.db_user.clone(),
        password: cli.db_pass.clone(),
        namespace: cli.db_namespace.clone(),
        database: cli.db_database.clone(),
    };

    // Configure pool based on CLI args or dynamic sizing
    let pool_config = PoolConfig {
        max_size: cli.db_pool_max_size,
        min_idle: Some(cli.db_pool_min_idle),
        connection_timeout: std::time::Duration::from_secs(cli.db_connection_timeout),
        ..Default::default()
    };

    // Create connection pool
    let db_pool = Arc::new(
        create_pool(connection_config, pool_config).map_err(|e|
            GitHubStarsError::ApiError(format!("Failed to create connection pool: {}", e))
        )?
    );

    println!("✅ Created SurrealDB connection pool with {} connections", cli.db_pool_max_size);

    // Query for accounts to determine actual worker count
    let db_conn = db_pool
        .get().await
        .map_err(|e|
            GitHubStarsError::ApiError(format!("Failed to get connection from pool: {}", e))
        )?;

    let accounts = db_conn
        .get_github_accounts(100, 0).await
        .map_err(|e| GitHubStarsError::ApiError(format!("Failed to query accounts: {}", e)))?;

    // Set actual workers based on accounts found (minimum 1, maximum from accounts)
    let num_workers = std::cmp::max(1, accounts.len());

    println!("📊 Found {} GitHub accounts", accounts.len());

    // Configure the factory
    let factory_config = GitHubFactoryConfig {
        num_initial_workers: num_workers,
        max_workers: 1000,
        queue_capacity: 1000,
        dead_mans_switch_timeout_seconds: 21600, // 6 hours - allows processing large repos
    };

    // Start the processing supervisor with live query
    let supervisor = ProcessingSupervisor::spawn_with_live_query(
        db_pool.clone(),
        factory_config
    ).await.map_err(|e| GitHubStarsError::ApiError(format!("Failed to start supervisor: {}", e)))?;

    println!("✅ Processing supervisor started with {} workers", num_workers);
    println!("📡 Listening for new GitHub accounts...");
    println!("\nPress Ctrl+C to stop the server\n");

    // Set up graceful shutdown
    let shutdown = tokio::signal::ctrl_c();

    tokio::select! {
        _ = shutdown => {
            println!("\n🛑 Shutting down server...");
            
            // Get final statistics
            match supervisor.call(
                |reply| ProcessingSupervisorMessage::GetStats(reply),
                Some(std::time::Duration::from_secs(5))
            ).await {
                Ok(call_result) => {
                    match call_result {
                        ractor::rpc::CallResult::Success(stats) => {
                            println!("\n📊 Final Statistics:");
                            println!("Total accounts processed: {}", stats.total_accounts_processed);
                            println!("Factory queue depth: {}", stats.factory_queue_depth);
                            println!("Active workers: {} / {} total", stats.factory_active_workers, stats.current_worker_count);
                            println!("System resources - CPU: {:.1}%, Memory: {:.1}%", stats.cpu_usage, stats.memory_usage_percent);
                        }
                        ractor::rpc::CallResult::Timeout => {
                            eprintln!("Timeout getting final statistics");
                        }
                        ractor::rpc::CallResult::SenderError => {
                            eprintln!("Sender error getting final statistics");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to get final statistics: {}", e);
                }
            }
            
            // Shutdown supervisor
            supervisor.send_message(ProcessingSupervisorMessage::Shutdown)
                .map_err(|e| GitHubStarsError::ApiError(format!("Failed to shutdown supervisor: {:?}", e)))?;
            
            // Give actors time to clean up
            println!("Waiting for workers to finish current tasks...");
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            
            println!("✅ Server stopped");
        }
    }

    Ok(())
}
