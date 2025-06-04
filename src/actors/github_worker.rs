use crate::error::{GitHubStarsError, Result};
use crate::github::GitHubClient;
use crate::models::RateLimitState;
use crate::pool::SurrealPool;
use chrono::Utc;
use ractor::{
    Actor, ActorProcessingErr, ActorRef,
    factory::{WorkerId, WorkerBuilder as RactorWorkerBuilder, WorkerMessage, WorkerStartContext},
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use surrealdb::RecordId;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

// Job key - identifies unique work items (by user)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GitHubJobKey {
    pub user_id: RecordId,
}

// Job payload - the actual work to do
#[derive(Debug, Clone)]
pub struct GitHubJobPayload {
    pub access_token: String,
}

/// Worker that processes GitHub repository stargazers
#[derive(Debug)]
pub struct GitHubWorker;

/// State for the GitHub worker
#[derive(Debug)]
pub struct GitHubWorkerState {
    worker_id: WorkerId,
    db_pool: Arc<SurrealPool>,
    jobs_processed: u64,
    rate_limit_state: RateLimitState,
    last_activity: Instant,
    // Queue of repos to process for current account
    repo_queue: Vec<(RecordId, String)>, // (repo_id, repo_full_name)
    current_account: Option<(RecordId, String)>, // (user_id, access_token)
}

impl GitHubWorker {
    pub fn new() -> Self {
        Self {}
    }
}

#[ractor::async_trait]
impl Actor for GitHubWorker {
    type Msg = WorkerMessage<GitHubJobKey, GitHubJobPayload>;
    type State = GitHubWorkerState;
    type Arguments = WorkerStartContext<GitHubJobKey, GitHubJobPayload, Arc<SurrealPool>>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> std::result::Result<Self::State, ActorProcessingErr> {
        info!(worker_id = ?args.wid, "GitHub worker starting");
        
        Ok(GitHubWorkerState {
            worker_id: args.wid,
            db_pool: args.custom_start,
            jobs_processed: 0,
            rate_limit_state: RateLimitState::default(),
            last_activity: Instant::now(),
            repo_queue: Vec::new(),
            current_account: None,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> std::result::Result<(), ActorProcessingErr> {
        match message {
            WorkerMessage::FactoryPing(_instant) => {
                // Factory ping received, no response needed
            }
            WorkerMessage::Dispatch(job) => {
                let job_key = job.key.clone();
                let payload = job.msg;

                debug!(
                    worker_id = ?state.worker_id,
                    ?job_key,
                    "Worker processing GitHub job"
                );

                state.last_activity = Instant::now();

                // Check rate limit before processing
                if state.rate_limit_state.is_limited {
                    let now = Utc::now();
                    if now < state.rate_limit_state.reset_time {
                        let wait_duration = (state.rate_limit_state.reset_time - now)
                            .to_std()
                            .unwrap_or(Duration::from_secs(60));
                        
                        warn!(
                            worker_id = ?state.worker_id,
                            "Rate limited, waiting {} seconds",
                            wait_duration.as_secs()
                        );
                        
                        sleep(wait_duration).await;
                        state.rate_limit_state.is_limited = false;
                    }
                }

                // Process the job
                let result = self.process_account(
                    &job_key.user_id,
                    &payload.access_token,
                    state
                ).await;

                state.jobs_processed += 1;

                match result {
                    Ok(_) => {
                        info!(
                            worker_id = ?state.worker_id,
                            ?job_key,
                            jobs_processed = state.jobs_processed,
                            "Job completed successfully"
                        );
                    }
                    Err(e) => {
                        error!(
                            worker_id = ?state.worker_id,
                            ?job_key,
                            "Job failed: {:?}",
                            e
                        );
                    }
                }
            }
        }
        
        Ok(())
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> std::result::Result<(), ActorProcessingErr> {
        info!(
            worker_id = ?state.worker_id,
            jobs_processed = state.jobs_processed,
            "GitHub worker stopped"
        );
        Ok(())
    }
}

impl GitHubWorker {
    async fn process_repository(
        &self,
        user_id: &RecordId,
        repo_id: &RecordId,
        repo_full_name: &str,
        access_token: &str,
        db_pool: &Arc<SurrealPool>,
        rate_limit_state: &mut RateLimitState,
    ) -> Result<()> {
        info!("Processing repository {} for user {}", repo_full_name, user_id.key());

        // Get a connection from the pool for this job
        let db_conn = db_pool.get().await
            .map_err(|e| GitHubStarsError::ApiError(format!("Failed to get DB connection: {}", e)))?;

        // Try to claim the repo for processing
        match db_conn.claim_repo_for_processing(repo_id, user_id).await {
            Ok(true) => {
                debug!("Successfully claimed repo {} for processing", repo_full_name);
            }
            Ok(false) => {
                info!("Repo {} already being processed by another worker", repo_full_name);
                return Ok(()); // Skip this repo
            }
            Err(e) => {
                error!("Failed to claim repo {}: {}", repo_full_name, e);
                return Err(GitHubStarsError::ApiError(format!("Failed to claim repo: {}", e)));
            }
        }

        // Create GitHub client for this specific job
        let github_client = GitHubClient::new(access_token.to_string())?;
        
        let mut page = 1;
        let mut total_stargazers = 0;

        loop {
            match github_client.fetch_stargazers_page(repo_full_name, page).await {
                Ok((stargazers, has_more, rate_limit)) => {
                    // Update rate limit state
                    *rate_limit_state = rate_limit;
                    
                    let stargazers_count = stargazers.len();
                    total_stargazers += stargazers_count;

                    info!(
                        "Fetched page {} with {} stargazers for repo {}",
                        page, stargazers_count, repo_full_name
                    );

                    if !stargazers.is_empty() {
                        // Insert stargazers in batch
                        info!("Inserting {} stargazers for repo {}", stargazers_count, repo_full_name);
                        db_conn
                            .insert_stargazers_batch(repo_id, &stargazers)
                            .await
                            .map_err(|e| GitHubStarsError::ApiError(format!("Failed to insert stargazers: {}", e)))?;

                        // Update processing progress
                        db_conn
                            .update_repo_processing_progress(repo_id, page, stargazers_count)
                            .await
                            .map_err(|e| GitHubStarsError::ApiError(format!("Failed to update progress: {}", e)))?;
                    } else {
                        warn!("No stargazers found on page {} for repo {}", page, repo_full_name);
                    }

                    if !has_more {
                        break;
                    }

                    page += 1;

                    // Add small delay between pages to be respectful
                    if rate_limit_state.remaining < 100 {
                        sleep(Duration::from_millis(500)).await;
                    }
                }
                Err(GitHubStarsError::RateLimitExceeded(msg)) => {
                    warn!("Rate limit exceeded for repo {}: {}", repo_full_name, msg);
                    
                    // Update repo status to rate limited
                    let retry_time = rate_limit_state.reset_time;
                    db_conn
                        .update_repo_rate_limited(repo_id, retry_time)
                        .await
                        .map_err(|e| GitHubStarsError::ApiError(format!("Failed to update rate limit status: {}", e)))?;
                    
                    return Err(GitHubStarsError::RateLimitExceeded(msg));
                }
                Err(e) => {
                    error!("Error processing repo {}: {:?}", repo_full_name, e);
                    
                    // Mark repo as failed
                    db_conn
                        .mark_repo_processing_failed(repo_id, &e.to_string())
                        .await
                        .map_err(|e2| GitHubStarsError::ApiError(format!("Failed to mark repo as failed: {}", e2)))?;
                    
                    return Err(e);
                }
            }
        }

        // Mark repo as complete
        db_conn
            .mark_repo_processing_complete(repo_id, total_stargazers as u32)
            .await
            .map_err(|e| GitHubStarsError::ApiError(format!("Failed to mark repo complete: {}", e)))?;

        info!(
            "Successfully processed {} stargazers from {} pages for repo {}",
            total_stargazers, page, repo_full_name
        );

        Ok(())
    }

    async fn process_account(
        &self,
        user_id: &RecordId,
        access_token: &str,
        state: &mut GitHubWorkerState,
    ) -> Result<()> {
        info!("Processing account for user {}", user_id.key());

        // Get a connection from the pool
        let db_conn = state.db_pool.get().await
            .map_err(|e| GitHubStarsError::ApiError(format!("Failed to get DB connection: {}", e)))?;

        // Wait for repo sync to complete
        self.wait_for_repo_sync(&state.db_pool, user_id).await?;

        // Get all repos for this user
        let mut repos = db_conn
            .get_repos_for_processing(user_id).await
            .map_err(|e| GitHubStarsError::ApiError(format!("Failed to get repos: {}", e)))?;

        if repos.is_empty() {
            info!("No repos found for user {}", user_id.key());
            return Ok(());
        }

        // Sort repos by star count (lowest first) for faster initial processing
        repos.sort_by_key(|r| r.stars);
        info!("Found {} repos for user {}, processing from smallest to largest", repos.len(), user_id.key());

        // Get processing status for all repos
        let repo_ids: Vec<String> = repos.iter().map(|r| r.full_name.clone()).collect();
        let statuses = db_conn
            .get_repos_processing_status(&repo_ids).await
            .map_err(|e| GitHubStarsError::ApiError(format!("Failed to get processing statuses: {}", e)))?;

        // Create a set of repos that are already processed or being processed
        let processed_repos: std::collections::HashSet<String> = statuses
            .into_iter()
            .filter(|s| matches!(
                s.status,
                crate::models::ProcessingStatus::Completed | crate::models::ProcessingStatus::Processing
            ))
            .map(|s| s.repo.key().to_string())
            .collect();

        // Build queue of repos to process
        state.repo_queue.clear();
        let mut queued_count = 0;
        let mut skipped_count = 0;

        for repo in repos {
            if processed_repos.contains(&repo.full_name) {
                debug!("Skipping already processed repo: {}", repo.full_name);
                skipped_count += 1;
                continue;
            }

            state.repo_queue.push((repo.id.clone(), repo.full_name.clone()));
            queued_count += 1;
        }

        info!(
            "Queued {} repos for user {} ({} skipped as already processed)",
            queued_count, user_id.key(), skipped_count
        );

        // Store account info for processing
        state.current_account = Some((user_id.clone(), access_token.to_string()));

        // Process all repos in the queue
        while let Some((repo_id, repo_full_name)) = state.repo_queue.pop() {
            info!("Processing repo {} ({} remaining in queue)", repo_full_name, state.repo_queue.len());

            // Check rate limit before each repo
            if state.rate_limit_state.is_limited {
                let now = Utc::now();
                if now < state.rate_limit_state.reset_time {
                    let wait_duration = (state.rate_limit_state.reset_time - now)
                        .to_std()
                        .unwrap_or(Duration::from_secs(60));
                    
                    warn!(
                        "Rate limited, waiting {} seconds until reset",
                        wait_duration.as_secs()
                    );
                    
                    sleep(wait_duration).await;
                    state.rate_limit_state.is_limited = false;
                }
            }

            // Process the repository
            if let Err(e) = self.process_repository(
                user_id,
                &repo_id,
                &repo_full_name,
                access_token,
                &state.db_pool,
                &mut state.rate_limit_state
            ).await {
                error!("Failed to process repo {}: {:?}", repo_full_name, e);
                // Continue with next repo even if one fails
            }

            state.last_activity = Instant::now();
        }

        // Clear account info when done
        state.current_account = None;
        
        info!("Completed processing all repos for user {}", user_id.key());
        Ok(())
    }

    async fn wait_for_repo_sync(
        &self,
        db_pool: &Arc<SurrealPool>,
        user_id: &RecordId,
    ) -> Result<()> {
        use tokio::time::{sleep, Duration, timeout};

        const MAX_WAIT: Duration = Duration::from_secs(300); // 5 minutes max
        const CHECK_INTERVAL: Duration = Duration::from_secs(2);
        const REQUIRED_STABLE_CHECKS: u32 = 15; // 30 seconds of stability

        info!("Waiting for repo sync to complete for user {}", user_id.key());

        let wait_result = timeout(MAX_WAIT, async {
            let mut last_count = 0;
            let mut stable_checks = 0;

            loop {
                // Get a connection from the pool
                let db_conn = db_pool.get().await
                    .map_err(|e| GitHubStarsError::ApiError(format!("Failed to get DB connection: {}", e)))?;

                // Get current repo count
                let repos = db_conn
                    .get_repos_for_processing(user_id).await
                    .map_err(|e| GitHubStarsError::ApiError(format!("Failed to get repos: {}", e)))?;
                let current_count = repos.len();

                debug!(
                    "User {} has {} repos (last count: {}, stable checks: {})",
                    user_id.key(), current_count, last_count, stable_checks
                );

                // Check if count is stable
                if current_count == last_count && current_count > 0 {
                    stable_checks += 1;
                    if stable_checks >= REQUIRED_STABLE_CHECKS {
                        info!(
                            "Repo sync appears complete for user {} with {} repos",
                            user_id.key(), current_count
                        );
                        break;
                    }
                } else {
                    stable_checks = 0;
                    last_count = current_count;
                }

                sleep(CHECK_INTERVAL).await;
            }

            Ok::<(), GitHubStarsError>(())
        }).await;

        match wait_result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                warn!("Timeout waiting for repo sync for user {}, proceeding anyway", user_id.key());
                Ok(())
            }
        }
    }
}

/// Builder for GitHub workers
#[derive(Debug, Clone)]
pub struct GitHubWorkerBuilder {
    db_pool: Arc<SurrealPool>,
}

impl GitHubWorkerBuilder {
    pub fn new(db_pool: Arc<SurrealPool>) -> Self {
        Self { db_pool }
    }
}

impl RactorWorkerBuilder<GitHubWorker, Arc<SurrealPool>> for GitHubWorkerBuilder {
    fn build(&mut self, _wid: WorkerId) -> (GitHubWorker, Arc<SurrealPool>) {
        (GitHubWorker::new(), self.db_pool.clone())
    }
}