use axum::{
    extract::ConnectInfo,
    http::StatusCode,
    middleware::Next,
    response::Response,
    Json,
};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Rate limit store (in-memory, per IP)
pub struct RateLimitStore {
    limits: Arc<Mutex<HashMap<String, Vec<i64>>>>,
}

impl RateLimitStore {
    pub fn new() -> Self {
        RateLimitStore {
            limits: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if request is allowed. Returns (allowed, remaining, reset_at)
    pub async fn check_rate_limit(
        &self,
        ip: &str,
        limit: usize,
        window_secs: u64,
    ) -> (bool, usize, u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let window_start = now - window_secs as i64;
        let mut limits = self.limits.lock().await;

        let timestamps = limits.entry(ip.to_string()).or_insert_with(Vec::new);

        // Remove old timestamps
        timestamps.retain(|&t| t > window_start);

        let allowed = timestamps.len() < limit;
        if allowed {
            timestamps.push(now);
        }

        let remaining = limit.saturating_sub(timestamps.len());
        let reset_at = if timestamps.is_empty() {
            now as u64
        } else {
            (timestamps[0] + window_secs as i64) as u64
        };

        (allowed, remaining, reset_at)
    }
}

impl Default for RateLimitStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Rate limit middleware
#[allow(dead_code)]
pub async fn rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    store: Arc<RateLimitStore>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let ip = addr.ip().to_string();
    let (allowed, remaining, reset_at) = store.check_rate_limit(&ip, 100, 60).await;

    if !allowed {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "rate_limit_exceeded",
                "message": "Too many requests"
            })),
        ));
    }

    // Add rate limit headers
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        "X-RateLimit-Limit",
        "100".parse().unwrap(),
    );
    response.headers_mut().insert(
        "X-RateLimit-Remaining",
        remaining.to_string().parse().unwrap(),
    );
    response.headers_mut().insert(
        "X-RateLimit-Reset",
        reset_at.to_string().parse().unwrap(),
    );

    Ok(response)
}
