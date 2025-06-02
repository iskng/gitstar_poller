mod actors;
mod cli;
mod error;
mod github;
mod models;
mod surreal_client;
mod types;

use actors::{ ProcessingSupervisor, ProcessingSupervisorMessage };
use actors::github_factory::GitHubFactoryConfig;
use clap::Parser;
use cli::Cli;
use colored::*;
use error::{ GitHubStarsError, Result };
use surreal_client::SurrealClient;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if it exists
    dotenv::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    println!("{}", "GitHub Stars Processing Server".bold().green());
    println!("{}\n", "=".repeat(50).dimmed());

    // Connect to SurrealDB
    let db_client = SurrealClient::new(
        &cli.db_url,
        &cli.db_user,
        &cli.db_pass,
        &cli.db_namespace,
        &cli.db_database
    ).await.map_err(|e|
        GitHubStarsError::ApiError(format!("Failed to connect to SurrealDB: {}", e))
    )?;

    println!("✅ Connected to SurrealDB");

    // Query for accounts to determine initial worker count
    let accounts = db_client
        .get_github_accounts(100).await
        .map_err(|e| GitHubStarsError::ApiError(format!("Failed to query accounts: {}", e)))?;

    // Set initial workers based on accounts found (minimum 5, maximum from accounts)
    let num_workers = std::cmp::max(1, accounts.len());

    println!("📊 Found {} GitHub accounts", accounts.len());

    // Configure the factory
    let factory_config = GitHubFactoryConfig {
        num_initial_workers: num_workers,
        queue_capacity: 1000,
        dead_mans_switch_timeout_seconds: 10000, // 10000 seconds = 2 hours 46 minutes 40 seconds
    };

    // Start the processing supervisor with live query
    let supervisor = ProcessingSupervisor::spawn_with_live_query(
        db_client,
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
                            println!("Active workers: {}", stats.factory_active_workers);
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
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            
            println!("✅ Server stopped");
        }
    }

    Ok(())
}
