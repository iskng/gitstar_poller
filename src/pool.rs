use deadpool::{managed, Runtime};
use surrealdb::engine::any::connect;
use surrealdb::opt::auth::Root;
use std::time::Duration;

use crate::error::GitHubStarsError as ServerError;
use crate::surreal_client::SurrealClient;

#[derive(Debug, Clone)]
pub struct SurrealConnectionConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    pub namespace: String,
    pub database: String,
}

#[derive(Debug)]
pub struct SurrealConnectionManager {
    config: SurrealConnectionConfig,
}

impl SurrealConnectionManager {
    pub fn new(config: SurrealConnectionConfig) -> Self {
        Self { config }
    }
}

impl managed::Manager for SurrealConnectionManager {
    type Type = SurrealClient;
    type Error = ServerError;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        // Create a new SurrealDB connection
        let db = connect(&self.config.url).await
            .map_err(|e| ServerError::ApiError(format!("Failed to connect: {}", e)))?;
        
        // Sign in as root user
        db.signin(Root {
            username: &self.config.username,
            password: &self.config.password,
        })
        .await
            .map_err(|e| ServerError::ApiError(format!("Failed to signin: {}", e)))?;

        // Select namespace and database
        db.use_ns(&self.config.namespace)
            .use_db(&self.config.database)
            .await
            .map_err(|e| ServerError::ApiError(format!("Failed to select namespace/database: {}", e)))?;

        Ok(SurrealClient { db })
    }

    async fn recycle(
        &self,
        conn: &mut Self::Type,
        _: &managed::Metrics,
    ) -> managed::RecycleResult<Self::Error> {
        // Test the connection with a simple query
        match conn.db.query("SELECT 1").await {
            Ok(_) => Ok(()),
            Err(e) => Err(managed::RecycleError::Backend(ServerError::ApiError(
                format!("Failed to recycle connection: {}", e),
            ))),
        }
    }
}

pub type SurrealPool = managed::Pool<SurrealConnectionManager>;

#[derive(Clone)]
pub struct PoolConfig {
    pub max_size: usize,
    pub min_idle: Option<usize>,
    pub max_lifetime: Option<Duration>,
    pub idle_timeout: Option<Duration>,
    pub connection_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_size: 10,
            min_idle: Some(2),
            max_lifetime: Some(Duration::from_secs(3600)), // 1 hour
            idle_timeout: Some(Duration::from_secs(600)),   // 10 minutes
            connection_timeout: Duration::from_secs(30),
        }
    }
}

pub fn create_pool(
    connection_config: SurrealConnectionConfig,
    pool_config: PoolConfig,
) -> Result<SurrealPool, ServerError> {
    let manager = SurrealConnectionManager::new(connection_config);
    
    let mut builder = managed::Pool::builder(manager)
        .max_size(pool_config.max_size)
        .runtime(Runtime::Tokio1);

    // Note: deadpool 0.12 doesn't have all these options in the builder
    // We'll use what's available
    builder = builder.create_timeout(Some(pool_config.connection_timeout));
    
    if let Some(idle_timeout) = pool_config.idle_timeout {
        builder = builder.recycle_timeout(Some(idle_timeout));
    }

    builder
        .build()
        .map_err(|e| ServerError::ApiError(format!("Failed to create connection pool: {}", e)))
}