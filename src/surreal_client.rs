use anyhow::{ Context, Result };
use chrono::Utc;
use serde_json::json;
use surrealdb::engine::any::Any;
use surrealdb::opt::auth::Root;
use surrealdb::{ Surreal, RecordId };
use surrealdb::sql::Datetime;
use tracing::{ debug, error, info, warn };

use crate::models::{
    Account,
    NewAccountEvent,
    Repo,
    RepoProcessingStatus,
    User,
    GithubUser,
};

#[derive(Clone, Debug)]
pub struct SurrealClient {
    pub db: Surreal<Any>,
}

impl SurrealClient {
    /// Create a new SurrealDB client
    pub async fn new(
        connection_url: &str,
        username: &str,
        password: &str,
        namespace: &str,
        database: &str
    ) -> Result<Self> {
        info!("Connecting to SurrealDB at {}", connection_url);

        // Create a new Surreal client with Any engine
        let db: Surreal<Any> = Surreal::init();

        // Connect to the endpoint using the Any-specific connect method
        let connect_result = db.connect(connection_url);
        connect_result.await.context("Failed to connect to SurrealDB")?;

        // Sign in as root user
        db
            .signin(Root { username, password }).await
            .context("Failed to authenticate with SurrealDB")?;

        // Select namespace and database
        db
            .use_ns(namespace)
            .use_db(database).await
            .context("Failed to select namespace and database")?;

        info!("Successfully connected to SurrealDB");
        Ok(Self { db })
    }

    /// Get all starred repos for a user
    pub async fn get_repos_for_processing(&self, user_id: &RecordId) -> Result<Vec<Repo>> {
        // First, get the github_user linked to this user
        let github_user_query =
            r#"
            SELECT github_user FROM user WHERE id = $user_id
        "#;

        let mut result = self.db.query(github_user_query).bind(("user_id", user_id.clone())).await?;

        #[derive(Debug, serde::Deserialize)]
        struct UserResult {
            github_user: Option<RecordId>,
        }

        let users: Vec<UserResult> = result.take(0)?;

        if users.is_empty() || users[0].github_user.is_none() {
            info!("No github_user linked for user {}", user_id.key());
            return Ok(Vec::new());
        }

        let github_user_id = users[0].github_user.as_ref().unwrap();
        info!("Found github_user {} for user {}", github_user_id.key(), user_id.key());

        // Now query all starred repos for this github_user
        let query =
            r#"
            SELECT 
                out.id as id,
                out.github_id as github_id,
                out.name as name,
                out.full_name as full_name,
                out.description as description,
                out.url as url,
                out.stars as stars,
                out.language as language,
                out.owner as owner,
                out.is_private as is_private,
                out.created_at as created_at,
                out.updated_at as updated_at
            FROM starred 
            WHERE in = $github_user_id
        "#;

        let mut result = self.db
            .query(query)
            .bind(("github_user_id", github_user_id.clone())).await?;

        #[derive(Debug, serde::Deserialize)]
        struct StarredRepoResult {
            id: RecordId,
            github_id: i64,
            name: String,
            full_name: String,
            description: Option<String>,
            url: String,
            stars: i64,
            language: Option<String>,
            owner: serde_json::Value,
            is_private: bool,
            created_at: String,
            updated_at: String,
        }

        let starred_results: Vec<StarredRepoResult> = result.take(0)?;

        if starred_results.is_empty() {
            info!("No starred repositories found for user {}", user_id.key());
            return Ok(Vec::new());
        }

        info!("User {} has {} starred repos", user_id.key(), starred_results.len());

        // Convert to Repo structs
        let repos: Vec<Repo> = starred_results
            .into_iter()
            .filter_map(|sr| {
                // Parse owner from JSON value
                let owner = if
                    let Ok(owner) = serde_json::from_value::<crate::models::RepoOwner>(
                        sr.owner.clone()
                    )
                {
                    owner
                } else {
                    error!("Failed to parse owner for repo {}", sr.full_name);
                    return None;
                };

                Some(Repo {
                    id: sr.id,
                    github_id: sr.github_id,
                    name: sr.name,
                    full_name: sr.full_name,
                    description: sr.description,
                    url: sr.url,
                    stars: sr.stars as u32,
                    language: sr.language,
                    owner,
                    is_private: sr.is_private,
                    created_at: Datetime::from(
                        chrono::DateTime
                            ::parse_from_rfc3339(&sr.created_at)
                            .unwrap_or_else(|_| chrono::Utc::now().into())
                            .with_timezone(&chrono::Utc)
                    ),
                    updated_at: Datetime::from(
                        chrono::DateTime
                            ::parse_from_rfc3339(&sr.updated_at)
                            .unwrap_or_else(|_| chrono::Utc::now().into())
                            .with_timezone(&chrono::Utc)
                    ),
                })
            })
            .collect();

        Ok(repos)
    }

    /// Atomically claim a repo for processing
    pub async fn claim_repo_for_processing(
        &self,
        repo_id: &RecordId,
        user_id: &RecordId
    ) -> Result<bool> {
        let query =
            r#"
            BEGIN TRANSACTION;
            
            -- Check if repo is available by repo ID
            LET $existing = (
                SELECT * FROM repo_processing_status 
                WHERE repo = $repo_id 
                AND (
                    status = 'completed' 
                    OR (status = 'processing' AND claimed_at > (time::now() - duration::from::hours(1)))
                )
            );
            
            -- Only proceed if not already processed/processing
            IF !$existing {
                -- Find or create the processing status record
                -- Use UPSERT to ensure we only create if it doesn't exist
                UPSERT repo_processing_status SET
                    repo = $repo_id,
                    status = 'processing',
                    claimed_by = $user_id,
                    claimed_at = time::now(),
                    started_at = time::now(),
                    last_page_processed = 0,
                    processed_stargazers = 0,
                    error_count = 0
                WHERE repo = $repo_id 
                AND (status != 'processing' OR claimed_at < (time::now() - duration::from::hours(1)) OR status = NULL);
                RETURN true;
            } ELSE {
                RETURN false;
            };
            
            COMMIT TRANSACTION;
        "#;

        let mut result = self.db
            .query(query)
            .bind(("repo_id", repo_id.clone()))
            .bind(("user_id", user_id.clone())).await?;

        let claimed: Option<bool> = result.take(0)?;
        Ok(claimed.unwrap_or(false))
    }

    /// Update repo processing progress
    pub async fn update_repo_processing_progress(
        &self,
        repo_id: &RecordId,
        page: u32,
        stargazers_count: usize
    ) -> Result<()> {
        let query =
            r#"
            UPDATE repo_processing_status SET
                last_page_processed = $page,
                processed_stargazers += $count
            WHERE repo = $repo_id
        "#;

        self.db
            .query(query)
            .bind(("repo_id", repo_id.clone()))
            .bind(("page", page))
            .bind(("count", stargazers_count as u32)).await?;

        Ok(())
    }

    /// Mark repo processing as complete
    pub async fn mark_repo_processing_complete(
        &self,
        repo_id: &RecordId,
        total_stargazers: u32
    ) -> Result<()> {
        let query =
            r#"
            UPDATE repo_processing_status SET
                status = 'completed',
                completed_at = time::now(),
                total_stargazers = $total,
                processed_stargazers = $total
            WHERE repo = $repo_id
        "#;

        self.db
            .query(query)
            .bind(("repo_id", repo_id.clone()))
            .bind(("total", total_stargazers)).await?;

        info!("Marked repo {} as complete with {} stargazers", repo_id.key(), total_stargazers);
        Ok(())
    }

    /// Mark repo processing as failed and reset claim
    pub async fn mark_repo_processing_failed(&self, repo_id: &RecordId, error: &str) -> Result<()> {
        let query =
            r#"
            UPDATE repo_processing_status SET
                status = 'failed',
                last_error = $error,
                error_count += 1,
                claimed_by = NULL,
                claimed_at = NULL
            WHERE repo = $repo_id
        "#;

        self.db
            .query(query)
            .bind(("repo_id", repo_id.clone()))
            .bind(("error", error.to_string())).await?;

        Ok(())
    }

    /// Unclaim a repo (used on worker shutdown or error)
    pub async fn unclaim_repo(&self, repo_id: &RecordId) -> Result<()> {
        let query = r#"
            UPDATE repo_processing_status SET
                status = 'pending',
                claimed_by = NULL,
                claimed_at = NULL
            WHERE repo = $repo_id
            AND status = 'processing'
        "#;

        self.db
            .query(query)
            .bind(("repo_id", repo_id.clone()))
            .await?;

        Ok(())
    }

    /// Update repo status to rate limited
    pub async fn update_repo_rate_limited(
        &self,
        repo_id: &RecordId,
        retry_after: chrono::DateTime<Utc>
    ) -> Result<()> {
        let query =
            r#"
            UPDATE repo_processing_status SET
                status = 'rate_limited',
                next_retry_after = $retry_after
            WHERE repo = $repo_id
        "#;

        let retry_datetime = Datetime::from(retry_after);

        self.db
            .query(query)
            .bind(("repo_id", repo_id.clone()))
            .bind(("retry_after", retry_datetime)).await?;

        Ok(())
    }

    /// Batch insert stargazers and create relationships
    pub async fn insert_stargazers_batch(
        &self,
        repo_id: &RecordId,
        stargazers: &[crate::types::GitHubUser]
    ) -> Result<()> {
        if stargazers.is_empty() {
            return Ok(());
        }

        info!("Inserting batch of {} stargazers for repo {}", stargazers.len(), repo_id.key());

        // First, upsert all github users
        for stargazer in stargazers {
            let github_user_id = RecordId::from(("github_user", stargazer.login.clone()));

            let user_data =
                json!({
                "github_id": stargazer.id,
                "username": stargazer.login.clone(),
                "avatar_url": stargazer.avatar_url.clone(),
                "profile_url": stargazer.html_url.clone(),
                "synced_at": Datetime::from(Utc::now()),
            });

            debug!("Upserting github_user {}: {:?}", github_user_id.key(), user_data);

            let query = r#"
                UPSERT $user_id CONTENT $data
            "#;

            let mut result = self.db
                .query(query)
                .bind(("user_id", github_user_id.clone()))
                .bind(("data", user_data)).await?;

            // Check the result
            let upserted: Vec<GithubUser> = result.take(0)?;
            if upserted.is_empty() {
                warn!("No result returned for github_user upsert: {}", github_user_id.key());
            }
        }

        // Then create starred relationships (github_user -> starred -> repo)
        let mut starred_created = 0;
        for stargazer in stargazers {
            let query =
                r#"
                -- Create the starred relationship if it doesn't exist
                LET $existing = (
                    SELECT * FROM starred 
                    WHERE in = $github_user 
                    AND out = $repo
                );
                
                IF !$existing {
                    RELATE $github_user->starred->$repo SET
                        starred_at = time::now(),
                        source = 'stargazer_analysis'
                };
            "#;

            let github_user = RecordId::from(("github_user", stargazer.login.as_str()));

            debug!("Creating starred relationship: {} -> {}", github_user.key(), repo_id.key());

            let mut result = self.db
                .query(query)
                .bind(("github_user", github_user.clone()))
                .bind(("repo", repo_id.clone())).await?;

            // The query returns the existing relation or nothing, we don't need to parse it
            let _: Option<()> = result.take(0)?;
            starred_created += 1;
        }

        info!("Created {} starred relationships for repo {}", starred_created, repo_id.key());
        Ok(())
    }

    /// Set up live query for new GitHub accounts
    pub async fn setup_account_live_query(
        &self
    ) -> Result<tokio::sync::mpsc::Receiver<NewAccountEvent>> {
        use futures::StreamExt;

        let (tx, rx) = tokio::sync::mpsc::channel::<NewAccountEvent>(100);

        info!("Setting up live query for new GitHub accounts...");

        // Create a live query stream
        let stream = self.db
            .query("LIVE SELECT * FROM account").await?
            .stream::<surrealdb::Notification<Account>>(0)?;

        // Clone the db for the spawned task
        let db = self.db.clone();

        tokio::spawn(async move {
            futures::pin_mut!(stream);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(notification) => {
                        // Process CREATE events for GitHub accounts
                        if let surrealdb::Action::Create = notification.action {
                            let account = notification.data.clone();
                            if account.provider_id == "github" {
                                // Fetch the user data
                                let user_id = account.user_id.key().to_string();
                                match db.select::<Option<User>>(account.user_id.clone()).await {
                                    Ok(Some(user)) => {
                                        let event = NewAccountEvent { account, user };
                                        if let Err(e) = tx.send(event).await {
                                            error!("Failed to send account event: {}", e);
                                            break;
                                        }
                                    }
                                    Ok(None) => {
                                        warn!("User {} not found for new account", user_id);
                                    }
                                    Err(e) => {
                                        error!("Failed to fetch user {}: {}", user_id, e);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error in account live stream: {}", e);
                        break;
                    }
                }
            }
            info!("Account live query stream ended");
        });

        Ok(rx)
    }

    /// Get processing statistics
    pub async fn get_processing_stats(&self) -> Result<serde_json::Value> {
        let query =
            r#"
            SELECT 
                count(status = 'completed') as completed,
                count(status = 'processing') as processing,
                count(status = 'failed') as failed,
                count(status = 'rate_limited') as rate_limited,
                count(status = 'pending') as pending,
                math::sum(processed_stargazers) as total_stargazers_found
            FROM repo_processing_status
        "#;

        let mut result = self.db.query(query).await?;
        let stats: Option<serde_json::Value> = result.take(0)?;
        Ok(stats.unwrap_or(json!({})))
    }

    /// Get processing status for multiple repos
    pub async fn get_repos_processing_status(
        &self,
        repo_ids: &[String]
    ) -> Result<Vec<RepoProcessingStatus>> {
        if repo_ids.is_empty() {
            return Ok(Vec::new());
        }

        let query =
            r#"
            SELECT * FROM repo_processing_status 
            WHERE repo IN $repo_ids
        "#;

        let repo_records: Vec<RecordId> = repo_ids
            .iter()
            .map(|id| RecordId::from(("repo", id.as_str())))
            .collect();

        let mut result = self.db.query(query).bind(("repo_ids", repo_records)).await?;

        let statuses: Vec<RepoProcessingStatus> = result.take(0)?;
        Ok(statuses)
    }

    /// Reset stale repo claims (repos claimed but not processed within timeout)
    pub async fn reset_stale_claims(&self, timeout_minutes: i64) -> Result<u32> {
        let query =
            r#"
            UPDATE repo_processing_status SET
                status = 'failed',
                last_error = 'Processing timeout - claim expired',
                error_count += 1,
                claimed_by = NULL,
                claimed_at = NULL
            WHERE status = 'processing' 
            AND claimed_at < (time::now() - duration::from::mins($timeout_mins))
            RETURN AFTER
        "#;

        let mut result = self.db.query(query).bind(("timeout_mins", timeout_minutes)).await?;

        let updated: Vec<RepoProcessingStatus> = result.take(0)?;
        let count = updated.len() as u32;

        if count > 0 {
            info!("Reset {} stale repo claims", count);
        }

        Ok(count)
    }

    /// Get GitHub accounts with limit and offset for startup
    pub async fn get_github_accounts(&self, limit: usize, offset: usize) -> Result<Vec<Account>> {
        let query =
            r#"
            SELECT * FROM account 
            WHERE providerId = 'github' 
            AND access_token != NULL
            ORDER BY id ASC
            LIMIT $limit
            START $offset
        "#;

        let mut result = self.db
            .query(query)
            .bind(("limit", limit))
            .bind(("offset", offset))
            .await?;

        let accounts: Vec<Account> = result.take(0)?;
        Ok(accounts)
    }
}
