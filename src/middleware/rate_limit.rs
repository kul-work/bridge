use axum::{
    extract::{ConnectInfo, Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::Database;
use crate::handlers::api_key::AppAuth;

const UNAUTHENTICATED_IP_LIMIT: usize = 10;
const UNAUTHENTICATED_IP_WINDOW_SECS: u64 = 60;

/// In-memory rate limit store
pub struct RateLimitStore {
    limits: Mutex<HashMap<String, Vec<i64>>>,
}

impl RateLimitStore {
    pub fn new() -> Self {
        RateLimitStore {
            limits: Mutex::new(HashMap::new()),
        }
    }

    async fn current_usage(&self, key: &str, window_secs: u64) -> (usize, u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let window_start = now - window_secs as i64;
        let mut limits = self.limits.lock().await;
        let timestamps = limits.entry(key.to_string()).or_insert_with(Vec::new);

        timestamps.retain(|&t| t > window_start);

        let reset_at = if timestamps.is_empty() {
            now as u64
        } else {
            (timestamps[0] + window_secs as i64) as u64
        };

        (timestamps.len(), reset_at)
    }

    async fn record_event(&self, key: &str, window_secs: u64) -> (usize, u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let window_start = now - window_secs as i64;
        let mut limits = self.limits.lock().await;
        let timestamps = limits.entry(key.to_string()).or_insert_with(Vec::new);

        timestamps.retain(|&t| t > window_start);
        timestamps.push(now);

        let reset_at = (timestamps[0] + window_secs as i64) as u64;
        (timestamps.len(), reset_at)
    }

    /// Check if request is allowed. Returns (allowed, remaining, reset_at)
    pub async fn check_rate_limit(
        &self,
        key: &str,
        limit: usize,
        window_secs: u64,
    ) -> (bool, usize, u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let window_start = now - window_secs as i64;
        let mut limits = self.limits.lock().await;

        let timestamps = limits.entry(key.to_string()).or_insert_with(Vec::new);

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

fn extract_client_ip(request: &Request) -> Option<String> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0.ip().to_string())
}

fn default_limit_for_group(group: &str) -> usize {
    match group {
        "checkout" => 20,
        "verify_purchase" => 20,
        "subscription_queries" => 100,
        "subscription_mutations" => 10,
        "payment_history" => 100,
        "purchase_registration" => 20,
        "agent" => 60,
        _ => 120,
    }
}

fn endpoint_group(method: &Method, path: &str) -> &'static str {
    if path.contains("/checkout") {
        return "checkout";
    }
    if path.contains("/verify-purchase") {
        return "verify_purchase";
    }
    if path.contains("/purchase/register") || path.contains("/purchases/register") {
        return "purchase_registration";
    }
    if path.contains("/agent/") || path.ends_with("/agent") {
        return "agent";
    }
    if path.contains("/subscriptions") {
        if *method == Method::GET {
            return "subscription_queries";
        }
        return "subscription_mutations";
    }
    if path.contains("/payments") {
        return "payment_history";
    }
    "default"
}

/// Per-API-key rate limit middleware (runs after auth)
pub async fn api_rate_limit_middleware(
    State(database): State<Arc<Database>>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    // Extract AppAuth (set by api_key_auth middleware)
    let auth = request.extensions().get::<AppAuth>().cloned();

    let auth = match auth {
        Some(a) => a,
        None => {
            // No auth context = middleware running before auth or on unauthenticated route; pass through
            return Ok(next.run(request).await);
        }
    };

    let group = endpoint_group(request.method(), request.uri().path());
    let key = format!("api:{}:{}", auth.api_key_id, group);

    // Load app config to get rate limit settings
    let effective_limit = match crate::db::apps::get_app(&database.pool, auth.app_id).await {
        Ok(app) => {
            // Check endpoint-specific override in api_rate_limit_rules JSONB
            let override_limit = app.api_rate_limit_rules.as_ref()
                .and_then(|rules| rules.get(group))
                .and_then(|v| v.as_i64())
                .map(|v| v as usize);

            override_limit
                .unwrap_or_else(|| {
                    if app.api_rate_limit_per_minute > 0 {
                        app.api_rate_limit_per_minute as usize
                    } else {
                        default_limit_for_group(group)
                    }
                })
        }
        Err(_) => default_limit_for_group(group),
    };

    // Global static store since middleware can't carry instance state easily
    static STORE: std::sync::OnceLock<RateLimitStore> = std::sync::OnceLock::new();
    let store = STORE.get_or_init(RateLimitStore::new);

    let (allowed, remaining, reset_at) = store.check_rate_limit(&key, effective_limit, 60).await;

    if !allowed {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "rate_limit_exceeded",
                "message": "Too many requests"
            })),
        ));
    }

    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "X-RateLimit-Limit",
        effective_limit.to_string().parse().unwrap(),
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

/// Per-IP rate limit middleware for unauthenticated and failed-auth requests.
/// This wraps the protected API stack and only counts responses that end in 401.
pub async fn unauthenticated_ip_rate_limit_middleware(
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let client_ip = match extract_client_ip(&request) {
        Some(ip) => ip,
        None => {
            // If we cannot identify the client IP, do not block the request.
            return Ok(next.run(request).await);
        }
    };

    let key = format!("ip:{}", client_ip);
    let has_auth_header = request.headers().get("authorization").is_some();

    static STORE: std::sync::OnceLock<RateLimitStore> = std::sync::OnceLock::new();
    let store = STORE.get_or_init(RateLimitStore::new);

    let (current_count, reset_at) = store
        .current_usage(&key, UNAUTHENTICATED_IP_WINDOW_SECS)
        .await;

    if !has_auth_header && current_count >= UNAUTHENTICATED_IP_LIMIT {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "rate_limit_exceeded",
                "message": "Too many unauthenticated requests",
                "reset_at": reset_at
            })),
        ));
    }

    let response = next.run(request).await;

    if response.status() == StatusCode::UNAUTHORIZED {
        if current_count >= UNAUTHENTICATED_IP_LIMIT {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "error": "rate_limit_exceeded",
                    "message": "Too many unauthenticated requests",
                    "reset_at": reset_at
                })),
            ));
        }

        let _ = store
            .record_event(&key, UNAUTHENTICATED_IP_WINDOW_SECS)
            .await;
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::{endpoint_group, extract_client_ip};
    use axum::extract::ConnectInfo;
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::Method;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn purchase_registration_uses_the_expected_rate_limit_bucket() {
        assert_eq!(
            endpoint_group(&Method::POST, "/api/v1/purchase/register"),
            "purchase_registration"
        );
        assert_eq!(
            endpoint_group(&Method::POST, "/api/v1/purchases/register"),
            "purchase_registration"
        );
    }

    #[test]
    fn client_ip_uses_connect_info() {
        let request = Request::builder()
            .uri("/api/v1/checkout")
            .body(Body::empty())
            .unwrap();
        let mut request = request;
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)),
            443,
        )));

        assert_eq!(extract_client_ip(&request).as_deref(), Some("203.0.113.10"));
    }
}
