use crate::actors::github_factory::{
    spawn_github_factory,
    submit_repo_processing_job,
    GitHubFactoryConfig,
};
use crate::actors::github_worker::{ GitHubJobKey, GitHubJobPayload };
use crate::models::NewAccountEvent;
use crate::surreal_client::SurrealClient;
use anyhow::Result;
use ractor::{
    Actor,
    ActorProcessingErr,
    ActorRef,
    RpcReplyPort,
    SpawnErr,
    factory::FactoryMessage,
};
use std::sync::Arc;
use surrealdb::RecordId;
use tokio::sync::mpsc;
use tracing::{ debug, error, info, warn };

/// The main supervisor that coordinates all processing
pub struct ProcessingSupervisor;

/// State for the processing supervisor
pub struct ProcessingSupervisorState {
    db_client: Arc<SurrealClient>,
    factory: ActorRef<FactoryMessage<GitHubJobKey, GitHubJobPayload>>,
    total_accounts_processed: u64,
}

/// Messages the supervisor can handle
#[derive(Debug)]
pub enum ProcessingSupervisorMessage {
    /// New account detected via live query
    NewAccount(NewAccountEvent),
    /// Get statistics about processing
    GetStats(RpcReplyPort<ProcessingStats>),
    /// Shutdown the system
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct ProcessingStats {
    pub total_accounts_processed: u64,
    pub factory_queue_depth: usize,
    pub factory_active_workers: usize,
}

/// Arguments for starting the supervisor
pub struct ProcessingSupervisorArgs {
    pub db_client: Arc<SurrealClient>,
    pub factory_config: GitHubFactoryConfig,
    pub account_receiver: mpsc::Receiver<NewAccountEvent>,
}

impl ProcessingSupervisor {
    /// Spawn the supervisor with live query integration
    pub async fn spawn_with_live_query(
        db_client: SurrealClient,
        factory_config: GitHubFactoryConfig
    ) -> Result<ActorRef<ProcessingSupervisorMessage>, SpawnErr> {
        let db_client_arc = Arc::new(db_client);

        // Set up live query for new accounts
        let account_receiver = db_client_arc
            .setup_account_live_query().await
            .map_err(|e| SpawnErr::StartupFailed(e.to_string().into()))?;

        let args = ProcessingSupervisorArgs {
            db_client: db_client_arc,
            factory_config,
            account_receiver,
        };

        let (actor_ref, _handle) = Actor::spawn(None, ProcessingSupervisor, args).await?;

        info!("Processing supervisor started with live query integration");
        Ok(actor_ref)
    }
}

#[ractor::async_trait]
impl Actor for ProcessingSupervisor {
    type Msg = ProcessingSupervisorMessage;
    type State = ProcessingSupervisorState;
    type Arguments = ProcessingSupervisorArgs;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        args: Self::Arguments
    ) -> Result<Self::State, ActorProcessingErr> {
        info!("Starting processing supervisor");

        // Spawn the GitHub processing factory
        let factory = spawn_github_factory(
            args.factory_config,
            args.db_client.clone()
        ).await.map_err(|e| ActorProcessingErr::from(format!("Failed to spawn factory: {}", e)))?;

        // Spawn task to forward live query events to the actor
        let myself_clone = myself.clone();
        let mut receiver = args.account_receiver;

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                if
                    let Err(e) = myself_clone.send_message(
                        ProcessingSupervisorMessage::NewAccount(event)
                    )
                {
                    error!("Failed to send NewAccount message: {}", e);
                    break;
                }
            }
            info!("Account live query receiver ended");
        });

        // Load and process existing accounts
        let db_client = args.db_client.clone();
        let factory_clone = factory.clone();
        tokio::spawn(async move {
            if let Err(e) = process_existing_accounts(db_client, factory_clone).await {
                error!("Failed to process existing accounts: {}", e);
            }
        });

        // Spawn periodic cleanup task for stale claims
        let db_client_cleanup = args.db_client.clone();
        tokio::spawn(async move {
            let mut cleanup_interval = tokio::time::interval(tokio::time::Duration::from_secs(300)); // Every 5 minutes
            cleanup_interval.tick().await; // Skip first immediate tick

            loop {
                cleanup_interval.tick().await;
                match db_client_cleanup.reset_stale_claims(60).await {
                    Ok(count) if count > 0 => {
                        info!("Reset {} stale repo claims", count);
                    }
                    Ok(_) => {
                        debug!("No stale claims to reset");
                    }
                    Err(e) => {
                        error!("Failed to reset stale claims: {}", e);
                    }
                }
            }
        });

        Ok(ProcessingSupervisorState {
            db_client: args.db_client,
            factory,
            total_accounts_processed: 0,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State
    ) -> Result<(), ActorProcessingErr> {
        match message {
            ProcessingSupervisorMessage::NewAccount(event) => {
                info!(
                    "New GitHub account detected for user {} ({})",
                    event.user.id.key(),
                    event.user.name.as_deref().unwrap_or("Unknown")
                );

                // Wait for repo sync to stabilize before processing
                let db_client = state.db_client.clone();
                let factory = state.factory.clone();
                let user_id = event.user.id.clone();
                let access_token = event.account.access_token.clone();

                tokio::spawn(async move {
                    // Wait for repo sync to complete
                    if let Err(e) = wait_for_repo_sync_completion(&db_client, &user_id).await {
                        error!("Failed to wait for repo sync for user {}: {}", user_id.key(), e);
                        return;
                    }

                    // Process all repos for this account
                    if
                        let Err(e) = process_account_repos(
                            &db_client,
                            &factory,
                            &user_id,
                            &access_token
                        ).await
                    {
                        error!("Failed to process repos for user {}: {}", user_id.key(), e);
                    }
                });

                state.total_accounts_processed += 1;
            }

            ProcessingSupervisorMessage::GetStats(reply) => {
                // Get factory statistics
                let queue_depth = match
                    state.factory.call(
                        |reply| FactoryMessage::GetQueueDepth(reply),
                        Some(std::time::Duration::from_secs(5))
                    ).await
                {
                    Ok(ractor::rpc::CallResult::Success(depth)) => depth,
                    _ => 0,
                };

                let active_workers = match
                    state.factory.call(
                        |reply| FactoryMessage::GetNumActiveWorkers(reply),
                        Some(std::time::Duration::from_secs(5))
                    ).await
                {
                    Ok(ractor::rpc::CallResult::Success(count)) => count,
                    _ => 0,
                };

                let stats = ProcessingStats {
                    total_accounts_processed: state.total_accounts_processed,
                    factory_queue_depth: queue_depth,
                    factory_active_workers: active_workers,
                };

                if !reply.is_closed() {
                    let _ = reply.send(stats);
                }
            }

            ProcessingSupervisorMessage::Shutdown => {
                info!("Shutting down processing supervisor");

                // Drain factory requests
                state.factory
                    .send_message(FactoryMessage::DrainRequests)
                    .map_err(|e|
                        ActorProcessingErr::from(format!("Failed to drain factory: {:?}", e))
                    )?;

                // The factory will handle shutting down its workers
                myself.stop(Some("Shutdown requested".to_string()));
            }
        }

        Ok(())
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State
    ) -> Result<(), ActorProcessingErr> {
        info!(
            "Processing supervisor stopped. Total accounts processed: {}",
            state.total_accounts_processed
        );
        Ok(())
    }
}

/// Process all repos for a given account
async fn process_account_repos(
    db_client: &Arc<SurrealClient>,
    factory: &ActorRef<FactoryMessage<GitHubJobKey, GitHubJobPayload>>,
    user_id: &RecordId,
    access_token: &str
) -> Result<()> {
    // Get all starred repos for this user
    let mut repos = db_client
        .get_repos_for_processing(user_id).await
        .map_err(|e| anyhow::anyhow!("Failed to get repos: {}", e))?;

    info!("Found {} starred repos for user {}", repos.len(), user_id.key());

    if repos.is_empty() {
        return Ok(());
    }
    
    // Sort repos by star count (lowest first) for faster initial processing
    repos.sort_by_key(|r| r.stars);
    debug!("Sorted repos by star count - smallest repo: {} stars, largest: {} stars", 
        repos.first().map(|r| r.stars).unwrap_or(0),
        repos.last().map(|r| r.stars).unwrap_or(0));

    // Get processing status for all repos to filter on client side
    let repo_ids: Vec<String> = repos
        .iter()
        .map(|r| r.full_name.clone())
        .collect();

    let statuses = db_client
        .get_repos_processing_status(&repo_ids).await
        .map_err(|e| anyhow::anyhow!("Failed to get processing statuses: {}", e))?;

    // Create a set of repos that are already processed or being processed
    let processed_repos: std::collections::HashSet<String> = statuses
        .into_iter()
        .filter(|s|
            matches!(
                s.status,
                crate::models::ProcessingStatus::Completed |
                    crate::models::ProcessingStatus::Processing
            )
        )
        .map(|s| s.repo.key().to_string())
        .collect();

    let mut submitted_count = 0;
    let mut skipped_count = 0;

    // Submit jobs for repos that aren't already processed
    for repo in repos {
        // Skip if already processed or being processed
        if processed_repos.contains(&repo.full_name) {
            debug!("Skipping already processed repo: {}", repo.full_name);
            skipped_count += 1;
            continue;
        }

        // Don't claim here - let the worker claim when it starts processing
        // Just submit the job to the factory
        if
            let Err(e) = submit_repo_processing_job(
                factory,
                user_id,
                &repo.id,
                repo.full_name.clone(),
                access_token.to_string()
            ).await
        {
            error!("Failed to submit job for repo {}: {}", repo.full_name, e);
        } else {
            debug!("Submitted job for repo {}", repo.full_name);
            submitted_count += 1;
        }
    }

    info!(
        "Processed repos for user {}: {} submitted, {} skipped",
        user_id.key(),
        submitted_count,
        skipped_count
    );

    Ok(())
}

/// Wait for repo sync to complete by monitoring repo count stability
async fn wait_for_repo_sync_completion(
    db_client: &Arc<SurrealClient>,
    user_id: &RecordId
) -> Result<()> {
    use tokio::time::{ sleep, Duration };

    info!("Waiting for repo sync to complete for user {}", user_id.key());

    let mut last_count = 0;
    let mut stable_checks = 0;
    const REQUIRED_STABLE_CHECKS: u32 = 2; // Number of consecutive checks with same count
    const CHECK_INTERVAL: Duration = Duration::from_secs(1);
    const MAX_WAIT_TIME: Duration = Duration::from_secs(10); // 10 seconds max wait

    let start_time = tokio::time::Instant::now();

    loop {
        // Check if we've exceeded max wait time
        if start_time.elapsed() > MAX_WAIT_TIME {
            warn!("Repo sync timeout for user {} - proceeding anyway", user_id.key());
            break;
        }

        // Get current repo count
        let repos = db_client.get_repos_for_processing(user_id).await?;
        let current_count = repos.len();

        debug!("User {} currently has {} starred repos", user_id.key(), current_count);

        // Check if count has stabilized
        if current_count == last_count && current_count > 0 {
            stable_checks += 1;
            if stable_checks >= REQUIRED_STABLE_CHECKS {
                info!(
                    "Repo sync stabilized for user {} with {} repos",
                    user_id.key(),
                    current_count
                );
                break;
            }
        } else {
            // Reset stability counter if count changed
            stable_checks = 0;
            last_count = current_count;
        }

        // If we have no repos yet, log it
        if current_count == 0 {
            debug!("No repos yet for user {}, waiting...", user_id.key());
        }

        sleep(CHECK_INTERVAL).await;
    }

    Ok(())
}

/// Process existing accounts on startup
async fn process_existing_accounts(
    db_client: Arc<SurrealClient>,
    factory: ActorRef<FactoryMessage<GitHubJobKey, GitHubJobPayload>>
) -> Result<()> {
    info!("Processing existing accounts");

    // Get up to 100 GitHub accounts
    let accounts = db_client.get_github_accounts(100).await?;

    info!("Found {} existing GitHub accounts", accounts.len());

    for account in accounts {
        // Wait for any ongoing repo sync to complete
        if let Err(e) = wait_for_repo_sync_completion(&db_client, &account.user_id).await {
            error!("Failed to wait for repo sync for user {}: {}", account.user_id.key(), e);
            continue;
        }

        if
            let Err(e) = process_account_repos(
                &db_client,
                &factory,
                &account.user_id,
                &account.access_token
            ).await
        {
            error!("Failed to process repos for existing user {}: {}", account.user_id.key(), e);
        }
    }

    Ok(())
}
