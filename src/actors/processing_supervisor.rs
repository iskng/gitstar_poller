use crate::actors::github_factory::{
    spawn_github_factory,
    submit_account_processing_job,
    GitHubFactoryConfig,
};
use crate::actors::github_worker::{ GitHubJobKey, GitHubJobPayload };
use crate::models::NewAccountEvent;
use crate::pool::SurrealPool;
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
use tokio::sync::mpsc;
use tracing::{ debug, error, info };

/// The main supervisor that coordinates all processing
pub struct ProcessingSupervisor;

/// State for the processing supervisor
pub struct ProcessingSupervisorState {
    db_pool: Arc<SurrealPool>,
    factory: ActorRef<FactoryMessage<GitHubJobKey, GitHubJobPayload>>,
    total_accounts_processed: u64,
}

/// Messages the supervisor can handle
#[derive(Debug)]
pub enum ProcessingSupervisorMessage {
    /// New account detected via live query
    NewAccount(NewAccountEvent),
    /// Start processing existing accounts
    ProcessExisting,
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
    pub db_pool: Arc<SurrealPool>,
    pub factory_config: GitHubFactoryConfig,
    pub account_receiver: mpsc::Receiver<NewAccountEvent>,
}

impl ProcessingSupervisor {
    /// Spawn the supervisor with live query integration
    pub async fn spawn_with_live_query(
        db_pool: Arc<SurrealPool>,
        factory_config: GitHubFactoryConfig
    ) -> Result<ActorRef<ProcessingSupervisorMessage>, SpawnErr> {
        // Get a connection from the pool to set up live query
        let db_conn = db_pool.get().await
            .map_err(|e| SpawnErr::StartupFailed(format!("Failed to get connection from pool: {}", e).into()))?;

        // Set up live query for new accounts
        let account_receiver = db_conn
            .setup_account_live_query().await
            .map_err(|e| SpawnErr::StartupFailed(e.to_string().into()))?;

        let args = ProcessingSupervisorArgs {
            db_pool,
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
            args.db_pool.clone()
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

        // Send message to start processing existing accounts
        let myself_clone = myself.clone();
        tokio::spawn(async move {
            // Give the system a moment to fully initialize
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            let _ = myself_clone.send_message(ProcessingSupervisorMessage::ProcessExisting);
        });


        // Spawn periodic cleanup task for stale claims
        let db_pool_cleanup = args.db_pool.clone();
        tokio::spawn(async move {
            let mut cleanup_interval = tokio::time::interval(tokio::time::Duration::from_secs(300)); // Every 5 minutes
            cleanup_interval.tick().await; // Skip first immediate tick

            loop {
                cleanup_interval.tick().await;
                // Get a connection from the pool for cleanup
                match db_pool_cleanup.get().await {
                    Ok(db_conn) => {
                        match db_conn.reset_stale_claims(60).await {
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
                    Err(e) => {
                        error!("Failed to get connection for cleanup: {}", e);
                    }
                }
            }
        });

        Ok(ProcessingSupervisorState {
            db_pool: args.db_pool,
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

                // Submit account processing job for new accounts (is_new_account = true)
                if let Err(e) = submit_account_processing_job(
                    &state.factory,
                    &event.user.id,
                    event.account.access_token.clone(),
                    true, // is_new_account
                ).await {
                    error!("Failed to submit job for new user {}: {}", event.user.id.key(), e);
                } else {
                    info!("Submitted high priority account processing job for new user {}", event.user.id.key());
                    state.total_accounts_processed += 1;
                }
            }

            ProcessingSupervisorMessage::ProcessExisting => {
                info!("Starting to process existing accounts");
                
                // Spawn a task to fetch and submit all existing accounts
                let db_pool = state.db_pool.clone();
                let factory = state.factory.clone();
                
                tokio::spawn(async move {
                    // Get a connection from the pool
                    match db_pool.get().await {
                        Ok(db_conn) => {
                            let mut total_submitted = 0;
                            let mut offset = 0;
                            let batch_size = 100;
                            
                            loop {
                                // Fetch accounts in batches with offset
                                match db_conn.get_github_accounts(batch_size, offset).await {
                                    Ok(accounts) => {
                                        if accounts.is_empty() {
                                            info!("Finished submitting all existing accounts. Total: {}", total_submitted);
                                            break;
                                        }
                                        
                                        info!("Fetched {} existing accounts to process (offset: {})", accounts.len(), offset);
                                        
                                        // Submit all existing accounts (is_new_account = false)
                                        for account in accounts {
                                            if let Err(e) = submit_account_processing_job(
                                                &factory,
                                                &account.user_id,
                                                account.access_token,
                                                false, // is_new_account
                                            ).await {
                                                error!("Failed to submit job for user {}: {}", account.user_id.key(), e);
                                            } else {
                                                total_submitted += 1;
                                            }
                                        }
                                        
                                        // Update offset for next batch
                                        offset += batch_size;
                                        
                                        // Brief pause between batches to avoid overwhelming the system
                                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                                    }
                                    Err(e) => {
                                        error!("Failed to fetch accounts: {}", e);
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to get DB connection for processing existing accounts: {}", e);
                        }
                    }
                });
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

                // Drain factory requests to prevent new jobs
                state.factory
                    .send_message(FactoryMessage::DrainRequests)
                    .map_err(|e|
                        ActorProcessingErr::from(format!("Failed to drain factory: {:?}", e))
                    )?;
                
                info!("Factory requests drained");

                // Give workers a moment to finish current work
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                // The factory will handle shutting down its workers
                // Workers will unclaim their current repos in post_stop
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
