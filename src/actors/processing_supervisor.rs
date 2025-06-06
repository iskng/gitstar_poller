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
use sysinfo::System;
use tokio::sync::mpsc;
use tracing::{ debug, error, info, warn };

/// The main supervisor that coordinates all processing
pub struct ProcessingSupervisor;

impl ProcessingSupervisor {
    /// Check if system has enough resources to add more workers
    fn can_add_workers(system: &mut System) -> (bool, String) {
        // Refresh system info
        system.refresh_memory();
        system.refresh_cpu_usage();
        
        // Calculate average CPU usage across all CPUs
        let cpu_count = system.cpus().len() as f32;
        let total_cpu_usage: f32 = system.cpus().iter().map(|cpu| cpu.cpu_usage()).sum();
        let avg_cpu_usage = if cpu_count > 0.0 { total_cpu_usage / cpu_count } else { 0.0 };
        
        // Get memory usage
        let total_memory = system.total_memory();
        let used_memory = system.used_memory();
        let memory_usage_percent = if total_memory > 0 {
            (used_memory as f64 / total_memory as f64) * 100.0
        } else {
            0.0
        };
        
        // Define thresholds
        const MAX_CPU_PERCENT: f32 = 80.0;
        const MAX_MEMORY_PERCENT: f64 = 85.0;
        
        let cpu_ok = avg_cpu_usage < MAX_CPU_PERCENT;
        let memory_ok = memory_usage_percent < MAX_MEMORY_PERCENT;
        
        let status = format!(
            "CPU: {:.1}% (limit: {}%), Memory: {:.1}% (limit: {}%)",
            avg_cpu_usage, MAX_CPU_PERCENT, memory_usage_percent, MAX_MEMORY_PERCENT
        );
        
        if !cpu_ok {
            return (false, format!("CPU usage too high: {}", status));
        }
        
        if !memory_ok {
            return (false, format!("Memory usage too high: {}", status));
        }
        
        (true, status)
    }
}

/// State for the processing supervisor
pub struct ProcessingSupervisorState {
    db_pool: Arc<SurrealPool>,
    factory: ActorRef<FactoryMessage<GitHubJobKey, GitHubJobPayload>>,
    factory_config: GitHubFactoryConfig,
    total_accounts_processed: u64,
    current_worker_count: usize,
    system: System,
    // Hourly statistics tracking
    last_hour_accounts_processed: u64,
    last_diagnostic_time: std::time::Instant,
}

/// Messages the supervisor can handle
#[derive(Debug)]
pub enum ProcessingSupervisorMessage {
    /// New account detected via live query
    NewAccount(NewAccountEvent),
    /// Start processing existing accounts
    ProcessExisting,
    /// Check and adjust worker scaling
    CheckWorkerScaling,
    /// Refresh CPU statistics
    RefreshCpuStats,
    /// Print hourly diagnostics
    PrintDiagnostics,
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
    pub current_worker_count: usize,
    pub cpu_usage: f32,
    pub memory_usage_percent: f64,
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
        
        // Save values we need before moving factory_config
        let initial_worker_count = args.factory_config.num_initial_workers;
        let factory_config = args.factory_config.clone();

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

        // Spawn periodic worker scaling task
        let myself_for_scaling = myself.clone();
        tokio::spawn(async move {
            let mut scaling_interval = tokio::time::interval(tokio::time::Duration::from_secs(30)); // Every 30 seconds
            scaling_interval.tick().await; // Skip first immediate tick

            loop {
                scaling_interval.tick().await;
                // Send message to check and scale workers
                if let Err(e) = myself_for_scaling.send_message(ProcessingSupervisorMessage::CheckWorkerScaling) {
                    error!("Failed to send worker scaling check: {}", e);
                    break;
                }
            }
        });

        // Spawn periodic CPU refresh task for accurate readings
        let myself_for_cpu = myself.clone();
        tokio::spawn(async move {
            let mut cpu_interval = tokio::time::interval(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
            cpu_interval.tick().await; // Skip first immediate tick

            loop {
                cpu_interval.tick().await;
                // Send message to refresh CPU stats
                if let Err(e) = myself_for_cpu.send_message(ProcessingSupervisorMessage::RefreshCpuStats) {
                    error!("Failed to send CPU refresh: {}", e);
                    break;
                }
            }
        });

        // Spawn hourly diagnostic reporting task
        let myself_for_diagnostics = myself.clone();
        tokio::spawn(async move {
            let mut diagnostic_interval = tokio::time::interval(tokio::time::Duration::from_secs(3600)); // Every hour
            diagnostic_interval.tick().await; // Skip first immediate tick

            loop {
                diagnostic_interval.tick().await;
                // Send message to print diagnostics
                if let Err(e) = myself_for_diagnostics.send_message(ProcessingSupervisorMessage::PrintDiagnostics) {
                    error!("Failed to send diagnostic request: {}", e);
                    break;
                }
            }
        });

        // Initialize system with CPU info for monitoring
        let mut system = System::new_all();
        system.refresh_cpu_usage();
        system.refresh_memory();
        
        Ok(ProcessingSupervisorState {
            db_pool: args.db_pool,
            factory,
            factory_config,
            total_accounts_processed: 0,
            current_worker_count: initial_worker_count,
            system,
            last_hour_accounts_processed: 0,
            last_diagnostic_time: std::time::Instant::now(),
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

                // Check if we should scale workers
                if state.current_worker_count < state.factory_config.max_workers {
                    // Get current queue depth
                    let queue_depth = match state.factory.call(
                        |reply| FactoryMessage::GetQueueDepth(reply),
                        Some(std::time::Duration::from_secs(1))
                    ).await {
                        Ok(ractor::rpc::CallResult::Success(depth)) => depth,
                        _ => 0,
                    };

                    // If queue has pending work and we haven't hit max workers, scale up
                    if queue_depth > 0 && state.current_worker_count < state.factory_config.max_workers {
                        // Check system resources first
                        let (can_scale, resource_status) = ProcessingSupervisor::can_add_workers(&mut state.system);
                        
                        if can_scale {
                            let new_worker_count = std::cmp::min(
                                state.current_worker_count + 1,
                                state.factory_config.max_workers
                            );
                            
                            info!(
                                "Scaling workers from {} to {} (queue depth: {}, max: {}, resources: {})",
                                state.current_worker_count, new_worker_count, queue_depth, 
                                state.factory_config.max_workers, resource_status
                            );
                            
                            if let Err(e) = state.factory
                                .send_message(FactoryMessage::AdjustWorkerPool(new_worker_count))
                            {
                                error!("Failed to adjust worker pool: {:?}", e);
                            } else {
                                state.current_worker_count = new_worker_count;
                            }
                        } else {
                            warn!("Cannot scale workers due to resource constraints: {}", resource_status);
                        }
                    }
                }

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
                    state.last_hour_accounts_processed += 1;
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

            ProcessingSupervisorMessage::CheckWorkerScaling => {
                // Get current queue depth and active workers
                let queue_depth = match state.factory.call(
                    |reply| FactoryMessage::GetQueueDepth(reply),
                    Some(std::time::Duration::from_secs(1))
                ).await {
                    Ok(ractor::rpc::CallResult::Success(depth)) => depth,
                    _ => 0,
                };

                let active_workers = match state.factory.call(
                    |reply| FactoryMessage::GetNumActiveWorkers(reply),
                    Some(std::time::Duration::from_secs(1))
                ).await {
                    Ok(ractor::rpc::CallResult::Success(count)) => count,
                    _ => 0,
                };

                debug!(
                    "Worker scaling check: queue_depth={}, active_workers={}, current_workers={}, max_workers={}",
                    queue_depth, active_workers, state.current_worker_count, state.factory_config.max_workers
                );

                // Scale up if queue has work and we have capacity
                if queue_depth > 0 && state.current_worker_count < state.factory_config.max_workers {
                    // Check system resources first
                    let (can_scale, resource_status) = ProcessingSupervisor::can_add_workers(&mut state.system);
                    
                    if can_scale {
                        // Scale up by 10% of current workers or 1, whichever is greater
                        let scale_increment = std::cmp::max(1, state.current_worker_count / 10);
                        let new_worker_count = std::cmp::min(
                            state.current_worker_count + scale_increment,
                            state.factory_config.max_workers
                        );
                        
                        if new_worker_count > state.current_worker_count {
                            info!(
                                "Scaling workers from {} to {} (queue depth: {}, active: {}, resources: {})",
                                state.current_worker_count, new_worker_count, queue_depth, active_workers, resource_status
                            );
                            
                            if let Err(e) = state.factory
                                .send_message(FactoryMessage::AdjustWorkerPool(new_worker_count))
                            {
                                error!("Failed to adjust worker pool: {:?}", e);
                            } else {
                                state.current_worker_count = new_worker_count;
                            }
                        }
                    } else {
                        debug!("Skipping worker scale-up due to resource constraints: {}", resource_status);
                    }
                }
                // Scale down if no queue and many idle workers
                else if queue_depth == 0 && active_workers < state.current_worker_count / 2 
                    && state.current_worker_count > state.factory_config.num_initial_workers {
                    // Scale down by 10% but keep at least initial workers
                    let scale_decrement = std::cmp::max(1, state.current_worker_count / 10);
                    let new_worker_count = std::cmp::max(
                        state.factory_config.num_initial_workers,
                        state.current_worker_count.saturating_sub(scale_decrement)
                    );
                    
                    if new_worker_count < state.current_worker_count {
                        info!(
                            "Scaling down workers from {} to {} (queue empty, active: {})",
                            state.current_worker_count, new_worker_count, active_workers
                        );
                        
                        if let Err(e) = state.factory
                            .send_message(FactoryMessage::AdjustWorkerPool(new_worker_count))
                        {
                            error!("Failed to adjust worker pool: {:?}", e);
                        } else {
                            state.current_worker_count = new_worker_count;
                        }
                    }
                }
            }

            ProcessingSupervisorMessage::RefreshCpuStats => {
                // Refresh CPU usage for accurate readings
                state.system.refresh_cpu_usage();
            }

            ProcessingSupervisorMessage::PrintDiagnostics => {
                // Get current statistics
                let elapsed = state.last_diagnostic_time.elapsed();
                let hours_elapsed = elapsed.as_secs_f64() / 3600.0;
                
                // Get factory statistics
                let queue_depth = match state.factory.call(
                    |reply| FactoryMessage::GetQueueDepth(reply),
                    Some(std::time::Duration::from_secs(1))
                ).await {
                    Ok(ractor::rpc::CallResult::Success(depth)) => depth,
                    _ => 0,
                };

                let active_workers = match state.factory.call(
                    |reply| FactoryMessage::GetNumActiveWorkers(reply),
                    Some(std::time::Duration::from_secs(1))
                ).await {
                    Ok(ractor::rpc::CallResult::Success(count)) => count,
                    _ => 0,
                };

                // Get system resource info
                state.system.refresh_memory();
                let cpu_count = state.system.cpus().len() as f32;
                let total_cpu_usage: f32 = state.system.cpus().iter().map(|cpu| cpu.cpu_usage()).sum();
                let avg_cpu = if cpu_count > 0.0 { total_cpu_usage / cpu_count } else { 0.0 };
                
                let total_memory = state.system.total_memory();
                let memory_usage_percent = if total_memory > 0 {
                    (state.system.used_memory() as f64 / total_memory as f64) * 100.0
                } else {
                    0.0
                };

                // Get processing stats from database
                let db_stats = match state.db_pool.get().await {
                    Ok(db_conn) => {
                        match db_conn.get_processing_stats().await {
                            Ok(stats) => Some(stats),
                            Err(e) => {
                                error!("Failed to get processing stats: {}", e);
                                None
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to get DB connection for stats: {}", e);
                        None
                    }
                };

                // Calculate hourly rate
                let accounts_per_hour = if hours_elapsed > 0.0 {
                    (state.last_hour_accounts_processed as f64 / hours_elapsed) as u64
                } else {
                    state.last_hour_accounts_processed
                };

                debug!("=== HOURLY DIAGNOSTIC REPORT ===");
                debug!("Time since last report: {:.1} hours", hours_elapsed);
                debug!("");
                debug!("PROCESSING STATISTICS:");
                debug!("  Total accounts processed (all time): {}", state.total_accounts_processed);
                debug!("  Accounts processed (last period): {}", state.last_hour_accounts_processed);
                debug!("  Processing rate: {} accounts/hour", accounts_per_hour);
                debug!("");
                debug!("WORKER STATISTICS:");
                debug!("  Total workers: {}", state.current_worker_count);
                debug!("  Active workers: {}", active_workers);
                debug!("  Idle workers: {}", state.current_worker_count.saturating_sub(active_workers));
                debug!("  Queue depth: {}", queue_depth);
                debug!("  Dead man's switch timeout: 6 hours");
                debug!("");
                debug!("SYSTEM RESOURCES:");
                debug!("  CPU usage: {:.1}%", avg_cpu);
                debug!("  Memory usage: {:.1}%", memory_usage_percent);
                debug!("  Memory: {} MB / {} MB", 
                    state.system.used_memory() / 1024 / 1024,
                    total_memory / 1024 / 1024
                );
                debug!("");
                debug!("DATABASE STATISTICS:");
                if let Some(stats) = db_stats {
                    debug!("  Repos completed: {}", stats.completed);
                    debug!("  Repos processing: {}", stats.processing);
                    debug!("  Repos failed: {}", stats.failed);
                    debug!("  Repos pending: {}", stats.pending);
                    debug!("  Repos rate limited: {}", stats.rate_limited);
                    debug!("  Repos pagination limited: {}", stats.pagination_limited);
                    debug!("  Total stargazers found: {}", stats.total_stargazers_found);
                } else {
                    debug!("  Failed to retrieve database statistics");
                }
                debug!("================================");
                
                // Reset hourly counters
                state.last_hour_accounts_processed = 0;
                state.last_diagnostic_time = std::time::Instant::now();
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

                // Get system resource info
                state.system.refresh_memory();
                state.system.refresh_cpu_usage();
                
                // Calculate average CPU usage
                let cpu_count = state.system.cpus().len() as f32;
                let total_cpu_usage: f32 = state.system.cpus().iter().map(|cpu| cpu.cpu_usage()).sum();
                let cpu_usage = if cpu_count > 0.0 { total_cpu_usage / cpu_count } else { 0.0 };
                
                let total_memory = state.system.total_memory();
                let memory_usage_percent = if total_memory > 0 {
                    (state.system.used_memory() as f64 / total_memory as f64) * 100.0
                } else {
                    0.0
                };

                let stats = ProcessingStats {
                    total_accounts_processed: state.total_accounts_processed,
                    factory_queue_depth: queue_depth,
                    factory_active_workers: active_workers,
                    current_worker_count: state.current_worker_count,
                    cpu_usage,
                    memory_usage_percent,
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
