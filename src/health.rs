use crate::actors::{ProcessingSupervisorMessage, ProcessingStats};
use crate::pool::SurrealPool;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use ractor::ActorRef;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

/// Health check status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub version: String,
    pub uptime_seconds: u64,
    pub checks: HealthChecks,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<ProcessingStats>,
}

/// Individual health checks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthChecks {
    pub database: CheckResult,
    pub supervisor: CheckResult,
    pub workers: CheckResult,
}

/// Result of an individual check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Liveness probe response (minimal, just indicates the process is running)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivenessResponse {
    pub status: String,
}

/// Readiness probe response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessResponse {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Application state for health checks
#[derive(Clone)]
pub struct AppState {
    pub db_pool: Arc<SurrealPool>,
    pub supervisor: ActorRef<ProcessingSupervisorMessage>,
    pub start_time: std::time::Instant,
}

/// Start the health check HTTP server
pub async fn start_health_server(
    app_state: AppState,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/healthz", get(health_check))  // Kubernetes convention
        .route("/livez", get(liveness_check))   // Kubernetes liveness probe
        .route("/readyz", get(readiness_check)) // Kubernetes readiness probe
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    
    info!("Health check server listening on http://{}", addr);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}

/// Main health check endpoint
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    
    // Check database connectivity
    let database_check = match state.db_pool.get().await {
        Ok(conn) => {
            // Try a simple query to verify the connection works
            match conn.db.query("SELECT 1").await {
                Ok(_) => CheckResult {
                    status: HealthStatus::Healthy,
                    message: None,
                },
                Err(e) => CheckResult {
                    status: HealthStatus::Unhealthy,
                    message: Some(format!("Database query failed: {}", e)),
                },
            }
        }
        Err(e) => CheckResult {
            status: HealthStatus::Unhealthy,
            message: Some(format!("Failed to get database connection: {}", e)),
        },
    };
    
    // Check supervisor and get stats
    let (supervisor_check, stats) = match state.supervisor.call(
        |reply| ProcessingSupervisorMessage::GetStats(reply),
        Some(Duration::from_secs(5))
    ).await {
        Ok(call_result) => {
            match call_result {
                ractor::rpc::CallResult::Success(stats) => {
                    let worker_status = if stats.factory_active_workers == 0 && stats.factory_queue_depth > 0 {
                        HealthStatus::Degraded
                    } else {
                        HealthStatus::Healthy
                    };
                    
                    (
                        CheckResult {
                            status: HealthStatus::Healthy,
                            message: None,
                        },
                        Some(stats)
                    )
                }
                _ => (
                    CheckResult {
                        status: HealthStatus::Unhealthy,
                        message: Some("Supervisor not responding".to_string()),
                    },
                    None
                ),
            }
        }
        Err(e) => (
            CheckResult {
                status: HealthStatus::Unhealthy,
                message: Some(format!("Failed to contact supervisor: {}", e)),
            },
            None
        ),
    };
    
    // Check workers based on stats
    let workers_check = if let Some(ref stats) = stats {
        if stats.current_worker_count == 0 {
            CheckResult {
                status: HealthStatus::Unhealthy,
                message: Some("No workers available".to_string()),
            }
        } else if stats.factory_active_workers == 0 && stats.factory_queue_depth > 10 {
            CheckResult {
                status: HealthStatus::Degraded,
                message: Some(format!("No active workers with {} items in queue", stats.factory_queue_depth)),
            }
        } else if stats.cpu_usage > 90.0 || stats.memory_usage_percent > 90.0 {
            CheckResult {
                status: HealthStatus::Degraded,
                message: Some(format!("High resource usage - CPU: {:.1}%, Memory: {:.1}%", 
                    stats.cpu_usage, stats.memory_usage_percent)),
            }
        } else {
            CheckResult {
                status: HealthStatus::Healthy,
                message: None,
            }
        }
    } else {
        CheckResult {
            status: HealthStatus::Unhealthy,
            message: Some("Unable to get worker statistics".to_string()),
        }
    };
    
    // Determine overall status
    let overall_status = if matches!(database_check.status, HealthStatus::Unhealthy)
        || matches!(supervisor_check.status, HealthStatus::Unhealthy)
        || matches!(workers_check.status, HealthStatus::Unhealthy) {
        HealthStatus::Unhealthy
    } else if matches!(database_check.status, HealthStatus::Degraded)
        || matches!(supervisor_check.status, HealthStatus::Degraded)
        || matches!(workers_check.status, HealthStatus::Degraded) {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    };
    
    let response = HealthResponse {
        status: overall_status.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: uptime,
        checks: HealthChecks {
            database: database_check,
            supervisor: supervisor_check,
            workers: workers_check,
        },
        stats,
    };
    
    let status_code = match overall_status {
        HealthStatus::Healthy => StatusCode::OK,
        HealthStatus::Degraded => StatusCode::OK, // Still return 200 for degraded
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    };
    
    (status_code, Json(response))
}

/// Kubernetes liveness probe - just checks if the process is alive
async fn liveness_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(LivenessResponse {
            status: "alive".to_string(),
        }),
    )
}

/// Kubernetes readiness probe - checks if the service is ready to handle requests
async fn readiness_check(State(state): State<AppState>) -> impl IntoResponse {
    // For this service, we're ready if:
    // 1. Database connection pool is available
    // 2. Supervisor is responsive
    
    // Quick database check
    let db_ready = match state.db_pool.get().await {
        Ok(_) => true,
        Err(_) => false,
    };
    
    // Quick supervisor check
    let supervisor_ready = match state.supervisor.call(
        |reply| ProcessingSupervisorMessage::GetStats(reply),
        Some(Duration::from_secs(1))
    ).await {
        Ok(ractor::rpc::CallResult::Success(_)) => true,
        _ => false,
    };
    
    let ready = db_ready && supervisor_ready;
    
    let response = ReadinessResponse {
        ready,
        message: if ready {
            None
        } else {
            Some(format!(
                "Not ready - DB: {}, Supervisor: {}",
                if db_ready { "OK" } else { "Failed" },
                if supervisor_ready { "OK" } else { "Failed" }
            ))
        },
    };
    
    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    
    (status_code, Json(response))
}