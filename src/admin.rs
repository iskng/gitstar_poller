use crate::actors::{ProcessingSupervisorMessage, ProcessingStats};
use crate::pool::SurrealPool;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use ractor::ActorRef;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use surrealdb::RecordId;
use tracing::{error, info, warn};

/// Admin API state
#[derive(Clone)]
pub struct AdminState {
    pub db_pool: Arc<SurrealPool>,
    pub supervisor: ActorRef<ProcessingSupervisorMessage>,
}

/// Response for successful operations
#[derive(Debug, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Response for errors
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Query parameters for filtering
#[derive(Debug, Deserialize)]
pub struct FilterParams {
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Repository status information
#[derive(Debug, Serialize, Deserialize)]
pub struct RepoStatus {
    pub repo_id: String,
    pub status: String,
    pub last_page_processed: u32,
    pub processed_stargazers: u32,
    pub total_stargazers: Option<u32>,
    pub claimed_by: Option<String>,
    pub last_error: Option<String>,
    pub error_count: u32,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// Detailed statistics
#[derive(Debug, Serialize, Deserialize)]
pub struct DetailedStats {
    pub processing: ProcessingStats,
    pub database: DatabaseStats,
    pub queue: QueueStats,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DatabaseStats {
    pub total_repos: u64,
    pub completed_repos: u64,
    pub failed_repos: u64,
    pub processing_repos: u64,
    pub pending_repos: u64,
    pub rate_limited_repos: u64,
    pub pagination_limited_repos: u64,
    pub total_stargazers: u64,
    pub total_github_users: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueueStats {
    pub depth: usize,
    pub active_workers: usize,
    pub total_workers: usize,
}

/// Worker scaling request
#[derive(Debug, Deserialize)]
pub struct ScaleRequest {
    pub workers: usize,
}

/// Reprocess request
#[derive(Debug, Deserialize)]
pub struct ReprocessRequest {
    pub force: Option<bool>,
}

/// Create the admin API router
pub fn create_admin_router(state: AdminState) -> Router {
    Router::new()
        // Statistics endpoints
        .route("/stats", get(get_detailed_stats))
        .route("/stats/summary", get(get_summary_stats))
        
        // Repository management
        .route("/repos", get(list_repos))
        .route("/repos/:repo_id", get(get_repo_status))
        .route("/repos/:repo_id/reprocess", post(reprocess_repo))
        .route("/repos/:repo_id/reset", post(reset_repo))
        
        // Worker management
        .route("/workers", get(get_worker_info))
        .route("/workers/scale", post(scale_workers))
        
        // Queue management
        .route("/queue", get(get_queue_info))
        .route("/queue/clear", post(clear_queue))
        
        // User/Account management
        .route("/users/:user_id/reprocess", post(reprocess_user))
        
        // System control
        .route("/pause", post(pause_processing))
        .route("/resume", post(resume_processing))
        
        .with_state(state)
}

/// Get detailed statistics
async fn get_detailed_stats(State(state): State<AdminState>) -> impl IntoResponse {
    // Get processing stats from supervisor
    let processing_stats = match state.supervisor.call(
        |reply| ProcessingSupervisorMessage::GetStats(reply),
        Some(Duration::from_secs(5))
    ).await {
        Ok(ractor::rpc::CallResult::Success(stats)) => stats,
        _ => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Failed to get processing statistics".to_string(),
                }),
            ).into_response();
        }
    };
    
    // Get database stats
    let db_stats = match get_database_stats(&state.db_pool).await {
        Ok(stats) => stats,
        Err(e) => {
            error!("Failed to get database stats: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to get database statistics: {}", e),
                }),
            ).into_response();
        }
    };
    
    let detailed_stats = DetailedStats {
        processing: processing_stats.clone(),
        database: db_stats,
        queue: QueueStats {
            depth: processing_stats.factory_queue_depth,
            active_workers: processing_stats.factory_active_workers,
            total_workers: processing_stats.current_worker_count,
        },
    };
    
    (StatusCode::OK, Json(detailed_stats)).into_response()
}

/// Get summary statistics
async fn get_summary_stats(State(state): State<AdminState>) -> impl IntoResponse {
    match state.supervisor.call(
        |reply| ProcessingSupervisorMessage::GetStats(reply),
        Some(Duration::from_secs(5))
    ).await {
        Ok(ractor::rpc::CallResult::Success(stats)) => {
            (StatusCode::OK, Json(stats)).into_response()
        }
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Failed to get processing statistics".to_string(),
            }),
        ).into_response(),
    }
}

/// List repositories with optional filtering
async fn list_repos(
    State(state): State<AdminState>,
    Query(params): Query<FilterParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(100).min(1000);
    let offset = params.offset.unwrap_or(0);
    
    let query = if let Some(status) = params.status {
        format!(
            r#"
            SELECT * FROM repo_processing_status 
            WHERE status = '{}' 
            ORDER BY started_at DESC
            LIMIT {} START {}
            "#,
            status, limit, offset
        )
    } else {
        format!(
            r#"
            SELECT * FROM repo_processing_status 
            ORDER BY started_at DESC
            LIMIT {} START {}
            "#,
            limit, offset
        )
    };
    
    match state.db_pool.get().await {
        Ok(db_conn) => {
            match db_conn.db.query(&query).await {
                Ok(mut result) => {
                    match result.take::<Vec<serde_json::Value>>(0) {
                        Ok(repos) => (StatusCode::OK, Json(repos)).into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to parse results: {}", e),
                            }),
                        ).into_response(),
                    }
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Database query failed: {}", e),
                    }),
                ),
            }
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: format!("Failed to get database connection: {}", e),
            }),
        ),
    }
}

/// Get status of a specific repository
async fn get_repo_status(
    State(state): State<AdminState>,
    Path(repo_id): Path<String>,
) -> impl IntoResponse {
    let repo_record = RecordId::from(("repo", repo_id.as_str()));
    
    match state.db_pool.get().await {
        Ok(db_conn) => {
            let query = r#"
                SELECT * FROM repo_processing_status 
                WHERE repo = $repo_id
            "#;
            
            match db_conn.db.query(query)
                .bind(("repo_id", repo_record))
                .await {
                Ok(mut result) => {
                    match result.take::<Option<serde_json::Value>>(0) {
                        Ok(Some(status)) => (StatusCode::OK, Json(status)),
                        Ok(None) => (
                            StatusCode::NOT_FOUND,
                            Json(ErrorResponse {
                                error: format!("Repository {} not found", repo_id),
                            }),
                        ),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to parse result: {}", e),
                            }),
                        ),
                    }
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Database query failed: {}", e),
                    }),
                ),
            }
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: format!("Failed to get database connection: {}", e),
            }),
        ),
    }
}

/// Reprocess a specific repository
async fn reprocess_repo(
    State(state): State<AdminState>,
    Path(repo_id): Path<String>,
    Json(request): Json<ReprocessRequest>,
) -> impl IntoResponse {
    let repo_record = RecordId::from(("repo", repo_id.as_str()));
    let force = request.force.unwrap_or(false);
    
    info!("Admin API: Reprocessing repo {} (force: {})", repo_id, force);
    
    match state.db_pool.get().await {
        Ok(db_conn) => {
            // First, reset the repo status
            let reset_query = if force {
                r#"
                UPDATE $repo_id SET 
                    status = 'pending',
                    claimed_by = NULL,
                    claimed_at = NULL,
                    last_page_processed = 0,
                    processed_stargazers = 0,
                    total_stargazers = NULL,
                    completed_at = NULL,
                    last_error = NULL,
                    notes = 'Manually reprocessed via admin API'
                WHERE repo = $repo_id
                "#
            } else {
                r#"
                UPDATE $repo_id SET 
                    status = 'pending',
                    claimed_by = NULL,
                    claimed_at = NULL,
                    notes = 'Manually reprocessed via admin API'
                WHERE repo = $repo_id 
                AND status IN ('failed', 'completed', 'pagination_limited')
                "#
            };
            
            match db_conn.db.query(reset_query)
                .bind(("repo_id", repo_record.clone()))
                .await {
                Ok(mut result) => {
                    let updated: Vec<serde_json::Value> = match result.take(0) {
                        Ok(u) => u,
                        Err(e) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: format!("Failed to parse update result: {}", e),
                                }),
                            );
                        }
                    };
                    
                    if updated.is_empty() {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "Repository not found or not eligible for reprocessing".to_string(),
                            }),
                        );
                    }
                    
                    info!("Reset repo {} status for reprocessing", repo_id);
                    
                    (
                        StatusCode::OK,
                        Json(SuccessResponse {
                            success: true,
                            message: format!("Repository {} queued for reprocessing", repo_id),
                        }),
                    )
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to reset repository: {}", e),
                    }),
                ),
            }
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: format!("Failed to get database connection: {}", e),
            }),
        ),
    }
}

/// Reset a repository (clear all processing data)
async fn reset_repo(
    State(state): State<AdminState>,
    Path(repo_id): Path<String>,
) -> impl IntoResponse {
    let repo_record = RecordId::from(("repo", repo_id.as_str()));
    
    info!("Admin API: Resetting repo {}", repo_id);
    
    match state.db_pool.get().await {
        Ok(db_conn) => {
            // Delete the processing status record
            let delete_query = r#"
                DELETE repo_processing_status 
                WHERE repo = $repo_id
            "#;
            
            match db_conn.db.query(delete_query)
                .bind(("repo_id", repo_record))
                .await {
                Ok(_) => {
                    info!("Reset processing status for repo {}", repo_id);
                    
                    (
                        StatusCode::OK,
                        Json(SuccessResponse {
                            success: true,
                            message: format!("Repository {} processing status reset", repo_id),
                        }),
                    )
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to reset repository: {}", e),
                    }),
                ),
            }
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: format!("Failed to get database connection: {}", e),
            }),
        ),
    }
}

/// Get worker information
async fn get_worker_info(State(state): State<AdminState>) -> impl IntoResponse {
    match state.supervisor.call(
        |reply| ProcessingSupervisorMessage::GetStats(reply),
        Some(Duration::from_secs(5))
    ).await {
        Ok(ractor::rpc::CallResult::Success(stats)) => {
            let info = serde_json::json!({
                "total_workers": stats.current_worker_count,
                "active_workers": stats.factory_active_workers,
                "idle_workers": stats.current_worker_count - stats.factory_active_workers,
                "queue_depth": stats.factory_queue_depth,
                "cpu_usage": stats.cpu_usage,
                "memory_usage_percent": stats.memory_usage_percent,
            });
            
            (StatusCode::OK, Json(info))
        }
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Failed to get worker information".to_string(),
            }),
        ),
    }
}

/// Scale workers
async fn scale_workers(
    State(_state): State<AdminState>,
    Json(request): Json<ScaleRequest>,
) -> impl IntoResponse {
    // Note: This would require adding a new message type to the supervisor
    // For now, return not implemented
    warn!("Worker scaling requested to {} workers - not implemented", request.workers);
    
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: "Worker scaling not yet implemented".to_string(),
        }),
    )
}

/// Get queue information
async fn get_queue_info(State(state): State<AdminState>) -> impl IntoResponse {
    match state.supervisor.call(
        |reply| ProcessingSupervisorMessage::GetStats(reply),
        Some(Duration::from_secs(5))
    ).await {
        Ok(ractor::rpc::CallResult::Success(stats)) => {
            let info = serde_json::json!({
                "depth": stats.factory_queue_depth,
                "active_workers": stats.factory_active_workers,
                "processing_rate": "N/A", // Would need to track this
            });
            
            (StatusCode::OK, Json(info))
        }
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Failed to get queue information".to_string(),
            }),
        ),
    }
}

/// Clear the queue (emergency use)
async fn clear_queue(State(_state): State<AdminState>) -> impl IntoResponse {
    // This would require adding queue management to the factory
    warn!("Queue clear requested - not implemented");
    
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: "Queue clearing not yet implemented".to_string(),
        }),
    )
}

/// Reprocess all repos for a user
async fn reprocess_user(
    State(state): State<AdminState>,
    Path(user_id): Path<String>,
) -> impl IntoResponse {
    let user_record = RecordId::from(("user", user_id.as_str()));
    
    info!("Admin API: Reprocessing all repos for user {}", user_id);
    
    // Get the user's access token
    match state.db_pool.get().await {
        Ok(db_conn) => {
            // Get account info
            let account_query = r#"
                SELECT * FROM account 
                WHERE userId = $user_id 
                AND providerId = 'github'
                AND access_token != NULL
            "#;
            
            match db_conn.db.query(account_query)
                .bind(("user_id", user_record.clone()))
                .await {
                Ok(mut result) => {
                    let accounts: Vec<serde_json::Value> = match result.take(0) {
                        Ok(a) => a,
                        Err(e) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: format!("Failed to parse accounts: {}", e),
                                }),
                            );
                        }
                    };
                    
                    if accounts.is_empty() {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(ErrorResponse {
                                error: format!("No GitHub account found for user {}", user_id),
                            }),
                        );
                    }
                    
                    // Extract access token
                    let access_token = match accounts[0].get("access_token").and_then(|v| v.as_str()) {
                        Some(token) => token.to_string(),
                        None => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: "Failed to get access token".to_string(),
                                }),
                            );
                        }
                    };
                    
                    // Reset all repos for this user
                    let reset_query = r#"
                        UPDATE repo_processing_status SET 
                            status = 'pending',
                            claimed_by = NULL,
                            claimed_at = NULL,
                            notes = 'User reprocessed via admin API'
                        WHERE repo IN (
                            SELECT out FROM starred WHERE in = (
                                SELECT github_user FROM user WHERE id = $user_id
                            )
                        )
                    "#;
                    
                    match db_conn.db.query(reset_query)
                        .bind(("user_id", user_record.clone()))
                        .await {
                        Ok(_) => {
                            info!("Reset all repos for user {} for reprocessing", user_id);
                            
                            // Note: This would require access to the factory through the supervisor
                            // For now, just reset the status
                            
                            (
                                StatusCode::OK,
                                Json(SuccessResponse {
                                    success: true,
                                    message: format!("All repositories for user {} queued for reprocessing", user_id),
                                }),
                            )
                        }
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to reset repositories: {}", e),
                            }),
                        ),
                    }
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to get account: {}", e),
                    }),
                ),
            }
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: format!("Failed to get database connection: {}", e),
            }),
        ),
    }
}

/// Pause processing (stop accepting new jobs)
async fn pause_processing(State(_state): State<AdminState>) -> impl IntoResponse {
    // This would require adding pause functionality to the supervisor
    warn!("Processing pause requested - not implemented");
    
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: "Pause functionality not yet implemented".to_string(),
        }),
    )
}

/// Resume processing
async fn resume_processing(State(_state): State<AdminState>) -> impl IntoResponse {
    // This would require adding resume functionality to the supervisor
    warn!("Processing resume requested - not implemented");
    
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: "Resume functionality not yet implemented".to_string(),
        }),
    )
}

/// Get database statistics
async fn get_database_stats(pool: &Arc<SurrealPool>) -> Result<DatabaseStats, anyhow::Error> {
    let db_conn = pool.get().await?;
    
    // Get repo counts
    let stats_query = r#"
        SELECT 
            count() as total_repos,
            count(status = 'completed') as completed_repos,
            count(status = 'failed') as failed_repos,
            count(status = 'processing') as processing_repos,
            count(status = 'pending') as pending_repos,
            count(status = 'rate_limited') as rate_limited_repos,
            count(status = 'pagination_limited') as pagination_limited_repos
        FROM repo_processing_status
    "#;
    
    let mut result = db_conn.db.query(stats_query).await?;
    let repo_stats: Option<serde_json::Value> = result.take(0)?;
    
    // Get stargazer count
    let stargazer_query = r#"
        SELECT count() as total FROM github_user
    "#;
    
    let mut result = db_conn.db.query(stargazer_query).await?;
    let stargazer_stats: Option<serde_json::Value> = result.take(0)?;
    
    // Get total stargazers from processing stats using the corrected query
    let processed_query = r#"
        -- Get all processed_stargazers values
        LET $stargazers = SELECT processed_stargazers FROM repo_processing_status WHERE processed_stargazers IS NOT NULL;
        
        -- Extract the processed_stargazers values into an array
        LET $stargazersArray = $stargazers.map(|$record| $record.processed_stargazers);
        
        -- Calculate the sum of the array
        RETURN math::sum($stargazersArray) as total;
    "#;
    
    let mut result = db_conn.db.query(processed_query).await?;
    let processed_stats: Option<serde_json::Value> = result.take(0)?;
    
    let total_stargazers = processed_stats
        .as_ref()
        .and_then(|v| {
            // Handle both direct value and object with total field
            if v.is_object() {
                v.get("total").and_then(|t| t.as_u64())
            } else {
                v.as_u64()
            }
        })
        .unwrap_or(0);
    
    let total_github_users = stargazer_stats
        .as_ref()
        .and_then(|s| s.get("total"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    
    if let Some(stats) = repo_stats {
        Ok(DatabaseStats {
            total_repos: stats.get("total_repos").and_then(|v| v.as_u64()).unwrap_or(0),
            completed_repos: stats.get("completed_repos").and_then(|v| v.as_u64()).unwrap_or(0),
            failed_repos: stats.get("failed_repos").and_then(|v| v.as_u64()).unwrap_or(0),
            processing_repos: stats.get("processing_repos").and_then(|v| v.as_u64()).unwrap_or(0),
            pending_repos: stats.get("pending_repos").and_then(|v| v.as_u64()).unwrap_or(0),
            rate_limited_repos: stats.get("rate_limited_repos").and_then(|v| v.as_u64()).unwrap_or(0),
            pagination_limited_repos: stats.get("pagination_limited_repos").and_then(|v| v.as_u64()).unwrap_or(0),
            total_stargazers,
            total_github_users,
        })
    } else {
        Ok(DatabaseStats {
            total_repos: 0,
            completed_repos: 0,
            failed_repos: 0,
            processing_repos: 0,
            pending_repos: 0,
            rate_limited_repos: 0,
            pagination_limited_repos: 0,
            total_stargazers: 0,
            total_github_users: 0,
        })
    }
}