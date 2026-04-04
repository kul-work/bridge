use axum::{
    extract::State,
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::Database;
use crate::handlers::api_key::AppAuth;

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
    request: axum::http::Request<axum::body::Body>,
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

#[cfg(test)]
mod tests {
    use super::endpoint_group;
    use axum::http::Method;

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
}
