use axum::{
    extract::{ConnectInfo, Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use serde_json::json;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::handlers::api_key::AppAuth;
use crate::ports::AppLookupRepository;
use crate::state::AppState;

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
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').find_map(parse_ip_candidate))
        .or_else(|| {
            request
                .headers()
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .and_then(parse_ip_candidate)
        })
        .or_else(|| {
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|connect_info| connect_info.0.ip().to_string())
        })
}

fn parse_ip_candidate(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    value
        .parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .ok()
        .or_else(|| {
            value
                .parse::<SocketAddr>()
                .ok()
                .map(|socket_addr| socket_addr.ip().to_string())
        })
}

fn rate_limit_override(rules: Option<&serde_json::Value>, group: &str) -> Option<usize> {
    rules
        .and_then(|rules| rules.get(group))
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn effective_limit_for_group(
    group: &str,
    api_rate_limit_per_minute: i32,
    rules: Option<&serde_json::Value>,
) -> usize {
    let group_limit = rate_limit_override(rules, group).unwrap_or_else(|| default_limit_for_group(group));

    match usize::try_from(api_rate_limit_per_minute).ok().filter(|limit| *limit > 0) {
        Some(app_limit) if group == "default" => app_limit,
        Some(app_limit) => group_limit.min(app_limit),
        None => group_limit,
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
    State(state): State<AppState>,
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
    let database = state.database();
    let effective_limit = match database.get_app(auth.app_id).await {
        Ok(app) => effective_limit_for_group(
            group,
            app.api_rate_limit_per_minute,
            app.api_rate_limit_rules.as_ref(),
        ),
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
    use super::{effective_limit_for_group, endpoint_group, extract_client_ip};
    use axum::extract::ConnectInfo;
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::Method;
    use serde_json::json;
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
    fn client_ip_prefers_x_forwarded_for() {
        let mut request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-forwarded-for", "198.51.100.5, 203.0.113.10")
            .header("x-real-ip", "203.0.113.10")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9)),
            443,
        )));

        assert_eq!(extract_client_ip(&request).as_deref(), Some("198.51.100.5"));
    }

    #[test]
    fn client_ip_falls_back_to_x_real_ip() {
        let request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-real-ip", "203.0.113.10")
            .body(Body::empty())
            .unwrap();

        assert_eq!(extract_client_ip(&request).as_deref(), Some("203.0.113.10"));
    }

    #[test]
    fn client_ip_falls_back_to_connect_info() {
        let request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .body(Body::empty())
            .unwrap();
        let mut request = request;
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)),
            443,
        )));

        assert_eq!(extract_client_ip(&request).as_deref(), Some("203.0.113.10"));
    }

    #[test]
    fn checkout_uses_documented_default_limit_when_only_global_default_exists() {
        assert_eq!(effective_limit_for_group("checkout", 120, None), 20);
    }

    #[test]
    fn tighter_app_limit_caps_endpoint_default() {
        assert_eq!(effective_limit_for_group("checkout", 15, None), 15);
    }

    #[test]
    fn endpoint_override_wins_but_stays_within_app_budget() {
        let rules = json!({ "checkout": 90 });

        assert_eq!(effective_limit_for_group("checkout", 60, Some(&rules)), 60);
        assert_eq!(effective_limit_for_group("checkout", 120, Some(&rules)), 90);
    }
}
