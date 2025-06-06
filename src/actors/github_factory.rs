use anyhow::Result;
use ractor::{
    Actor,
    ActorRef,
    factory::{
        Factory,
        FactoryArguments,
        FactoryMessage,
        queues,
        routing,
        discard::DiscardHandler,
    },
};
use std::sync::Arc;
use tracing::{ info, error, warn };

use crate::actors::github_worker::{
    GitHubWorker,
    GitHubWorkerBuilder,
    GitHubJobKey,
    GitHubJobPayload,
    JobPriority,
};
use crate::pool::SurrealPool;

/// Configuration for the GitHub processing factory
#[derive(Debug, Clone)]
pub struct GitHubFactoryConfig {
    /// Number of workers to start initially
    pub num_initial_workers: usize,
    /// Maximum number of workers
    pub max_workers: usize,
    /// Maximum queue capacity
    pub queue_capacity: usize,
    /// Time in seconds before considering a worker stuck
    pub dead_mans_switch_timeout_seconds: u64,
}

impl Default for GitHubFactoryConfig {
    fn default() -> Self {
        Self {
            num_initial_workers: 5,
            max_workers: 1000,
            queue_capacity: 1000,
            dead_mans_switch_timeout_seconds: 21600, // 6 hours
        }
    }
}

/// Custom discard handler for dropped jobs
#[derive(Debug, Clone)]
pub struct GitHubJobDiscardHandler;

impl DiscardHandler<GitHubJobKey, GitHubJobPayload> for GitHubJobDiscardHandler {
    fn discard(
        &self,
        reason: ractor::factory::discard::DiscardReason,
        job: &mut ractor::factory::Job<GitHubJobKey, GitHubJobPayload>
    ) {
        warn!("Discarding job {:?} for reason: {:?}", job.key, reason);

        // In a production system, you might want to:
        // - Log to metrics
        // - Send to a dead letter queue
        // - Retry with exponential backoff
        // - Alert on high discard rates
    }
}

/// Priority manager that prioritizes new accounts over existing ones
#[derive(Debug, Clone)]
pub struct GitHubPriorityManager;

impl queues::PriorityManager<GitHubJobKey, JobPriority> for GitHubPriorityManager {
    fn is_discardable(&self, _key: &GitHubJobKey) -> bool {
        false // Never discard jobs
    }
    
    fn get_priority(&self, key: &GitHubJobKey) -> Option<JobPriority> {
        // New accounts get high priority, existing accounts get low priority
        Some(if key.is_new_account {
            JobPriority::High
        } else {
            JobPriority::Low
        })
    }
}

/// Type alias for our factory with priority queue
pub type GitHubProcessingFactory = Factory<
    GitHubJobKey,
    GitHubJobPayload,
    Arc<SurrealPool>, // Worker startup args
    GitHubWorker,
    routing::StickyQueuerRouting<GitHubJobKey, GitHubJobPayload>, // Sticky routing for same user
    queues::PriorityQueue<GitHubJobKey, GitHubJobPayload, JobPriority, GitHubPriorityManager, 2> // 2 priority levels
>;

/// Spawns a GitHub processing factory
pub async fn spawn_github_factory(
    config: GitHubFactoryConfig,
    db_pool: Arc<SurrealPool>
) -> Result<ActorRef<FactoryMessage<GitHubJobKey, GitHubJobPayload>>> {
    info!("Spawning GitHub processing factory with {} initial workers", config.num_initial_workers);

    // Create priority queue with our priority manager
    let queue = queues::PriorityQueue::<GitHubJobKey, GitHubJobPayload, JobPriority, GitHubPriorityManager, 2>::new(
        GitHubPriorityManager
    );

    // Use sticky routing so same user's repos go to same worker
    let router = routing::StickyQueuerRouting::<GitHubJobKey, GitHubJobPayload>::default();

    // Create worker builder
    let worker_builder = GitHubWorkerBuilder::new(db_pool);

    // Configure dead man's switch
    let dead_mans_switch = ractor::factory::DeadMansSwitchConfiguration {
        detection_timeout: std::time::Duration::from_secs(config.dead_mans_switch_timeout_seconds),
        kill_worker: true, // Kill stuck workers
    };

    // Build factory arguments
    let factory_args = FactoryArguments::builder()
        .worker_builder(Box::new(worker_builder))
        .queue(queue)
        .router(router)
        .num_initial_workers(config.num_initial_workers)
        .discard_handler(Arc::new(GitHubJobDiscardHandler))
        .dead_mans_switch(dead_mans_switch)
        .build();

    // Create the factory actor
    let factory_actor = GitHubProcessingFactory::default();

    // Spawn the factory
    match Actor::spawn(None, factory_actor, factory_args).await {
        Ok((actor_ref, _actor_handle)) => {
            info!("GitHub processing factory spawned successfully");
            Ok(actor_ref)
        }
        Err(spawn_err) => {
            error!("Failed to spawn GitHub processing factory: {:?}", spawn_err);
            Err(anyhow::anyhow!("Failed to spawn factory: {:?}", spawn_err))
        }
    }
}

/// Helper to submit an account processing job to the factory
pub async fn submit_account_processing_job(
    factory: &ActorRef<FactoryMessage<GitHubJobKey, GitHubJobPayload>>,
    user_id: &surrealdb::RecordId,
    access_token: String,
    is_new_account: bool,
) -> Result<()> {
    let job_key = GitHubJobKey {
        user_id: user_id.clone(),
        is_new_account,
    };
    let payload = GitHubJobPayload {
        access_token,
    };

    let job = ractor::factory::Job {
        key: job_key,
        msg: payload,
        options: Default::default(),
        accepted: None,
    };

    factory
        .send_message(FactoryMessage::Dispatch(job))
        .map_err(|e| anyhow::anyhow!("Failed to dispatch account job: {:?}", e))?;

    Ok(())
}
