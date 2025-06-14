use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Middleware to check API key authentication
pub async fn auth_middleware(
    api_key: String,
    req: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Check for API key in Authorization header
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = &header[7..]; // Skip "Bearer "
            if token == api_key {
                Ok(next.run(req).await)
            } else {
                Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "error": "Invalid API key"
                    })),
                ))
            }
        }
        Some(header) if header == api_key => {
            // Also accept raw API key for compatibility
            Ok(next.run(req).await)
        }
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "Missing or invalid Authorization header"
            })),
        )),
    }
}