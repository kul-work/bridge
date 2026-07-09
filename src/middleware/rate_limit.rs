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
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::handlers::api_key::AppAuth;
use crate::middleware::admin_auth::AdminAuthContext;
use crate::ports::AppLookupRepository;
use crate::state::AppState;

const UNAUTHENTICATED_IP_LIMIT: usize = 10;
const UNAUTHENTICATED_IP_WINDOW_SECS: u64 = 60;
const ADMIN_AUTH_IP_LIMIT_DEFAULT: usize = 10;
const ADMIN_AUTH_IP_WINDOW_SECS: u64 = 60;
const ADMIN_READ_LIMIT_DEFAULT: usize = 120;
const ADMIN_MUTATION_LIMIT_DEFAULT: usize = 10;

struct AdminRateLimits {
    read: usize,
    mutation: usize,
}

struct AdminAuthIpRateLimit {
    limit: usize,
}

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

        // Read-only path: do NOT insert a key for an unseen bucket. Inserting
        // here would let an attacker grow the map just by probing distinct
        // source IPs (or spoofed forwarded headers), since idle keys are only
        // removed when they are next touched.
        let (count, reset_at_opt) = match limits.get_mut(key) {
            Some(timestamps) => {
                timestamps.retain(|&t| t > window_start);
                let reset_at = if timestamps.is_empty() {
                    None
                } else {
                    Some((timestamps[0] + window_secs as i64) as u64)
                };
                (timestamps.len(), reset_at)
            }
            None => (0, None),
        };

        // Evict buckets that drained during this read so the map cannot grow
        // without bound from one-shot probes.
        if reset_at_opt.is_none() {
            limits.remove(key);
        }

        (count, reset_at_opt.unwrap_or(now as u64))
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

        let (allowed, remaining, reset_at) = {
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
        };

        // Evict buckets that drained to empty (e.g. a zero limit, or clock
        // skew leaving no live timestamps) so the map does not retain dead keys.
        let drained = limits.get(key).is_some_and(|ts| ts.is_empty());
        if drained {
            limits.remove(key);
        }

        (allowed, remaining, reset_at)
    }
}

impl Default for RateLimitStore {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_client_ip(request: &Request) -> Option<String> {
    extract_client_ip_with(request, trusted_proxies())
}

/// Resolve the client IP, honoring forwarded headers only when the immediate
/// TCP peer is a configured trusted proxy.
///
/// Forwarded headers (`X-Forwarded-For`, `X-Real-IP`) are client-supplied and
/// trivially spoofable. Trusting them unconditionally lets an attacker rotate
/// the apparent source IP to bypass per-IP rate limiting and the admin-auth IP
/// guard. We therefore anchor on the socket peer from `ConnectInfo` and only
/// consult the forwarded chain when that peer is an explicitly trusted proxy
/// (the reverse proxy in front of Bridge, which is expected to strip/replace
/// these headers). When no trusted proxies are configured, the peer IP is used
/// directly and forwarded headers are ignored.
fn extract_client_ip_with(request: &Request, trusted_proxies: &[IpAddr]) -> Option<String> {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0.ip());

    let honor_forwarded = peer.is_some_and(|ip| trusted_proxies.contains(&ip));

    if honor_forwarded {
        if let Some(forwarded) = forwarded_client_ip(request, trusted_proxies) {
            return Some(forwarded);
        }
    }

    peer.map(|ip| ip.to_string())
}

/// Resolve the client IP from forwarded headers set by a trusted proxy.
///
/// `X-Forwarded-For` is parsed right-to-left, skipping entries that are
/// themselves trusted proxies, and returns the nearest untrusted hop. This
/// defeats spoofing when the trusted proxy *appends* (rather than overwrites)
/// the header — e.g. nginx's `$proxy_add_x_forwarded_for` produces
/// `<client-supplied>, <real-client-ip>`, so the leftmost entry is
/// attacker-controlled and must not be selected. The rightmost entry not in
/// the trusted set is the IP the proxy actually observed.
///
/// `X-Real-IP` is trusted directly: nginx overwrites it with `$remote_addr`,
/// so a client cannot inject it.
fn forwarded_client_ip(request: &Request, trusted_proxies: &[IpAddr]) -> Option<String> {
    let xff = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());

    if let Some(value) = xff {
        let nearest_untrusted = value
            .split(',')
            .rev()
            .filter_map(|entry| entry.trim().parse::<IpAddr>().ok())
            .find(|ip| !trusted_proxies.contains(ip));
        if let Some(ip) = nearest_untrusted {
            return Some(ip.to_string());
        }
        // All XFF entries were trusted proxies (e.g. proxy chaining with no
        // recorded client). Fall through to X-Real-IP, then to the peer.
    }

    request
        .headers()
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_ip_candidate)
}

/// Parse `TRUSTED_PROXIES` strictly: every non-empty token must be a valid IP
/// literal. CIDR ranges (e.g. `10.0.0.0/8`) and typos are rejected with a
/// clear error rather than silently dropped, so a security-sensitive setting
/// cannot partially collapse without the operator knowing.
fn parse_trusted_proxies(raw: &str) -> Result<Vec<IpAddr>, String> {
    let mut proxies = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        match entry.parse::<IpAddr>() {
            Ok(ip) => proxies.push(ip),
            Err(_) => {
                return Err(format!(
                    "TRUSTED_PROXIES contains invalid entry '{}': only IP literals are supported \
                     (CIDR ranges like '10.0.0.0/8' are not supported). Fix the env var or unset it.",
                    entry
                ));
            }
        }
    }
    Ok(proxies)
}

/// Trusted proxy peers allowed to set forwarded IP headers. Parsed once from
/// the `TRUSTED_PROXIES` env var (comma-separated IP literals) and cached for
/// the process lifetime. Empty when unset, which means forwarded headers are
/// never honored and the socket peer is used directly. An invalid entry
/// fails loudly at first use rather than silently degrading.
fn trusted_proxies() -> &'static Vec<IpAddr> {
    static TRUSTED_PROXIES: OnceLock<Vec<IpAddr>> = OnceLock::new();
    TRUSTED_PROXIES.get_or_init(|| {
        match env::var("TRUSTED_PROXIES") {
            Ok(raw) => parse_trusted_proxies(&raw).expect("invalid TRUSTED_PROXIES configuration"),
            Err(_) => Vec::new(),
        }
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

fn admin_endpoint_group(method: &Method) -> &'static str {
    match *method {
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE => "mutation",
        _ => "read",
    }
}

fn admin_limit_for_group(group: &str) -> usize {
    static ADMIN_RATE_LIMITS: OnceLock<AdminRateLimits> = OnceLock::new();
    let limits = ADMIN_RATE_LIMITS.get_or_init(|| AdminRateLimits {
        read: configured_limit("ADMIN_READ_RATE_LIMIT_PER_MINUTE", ADMIN_READ_LIMIT_DEFAULT),
        mutation: configured_limit("ADMIN_MUTATION_RATE_LIMIT_PER_MINUTE", ADMIN_MUTATION_LIMIT_DEFAULT),
    });

    admin_limit_for_group_with_values(
        group,
        limits.read,
        limits.mutation,
    )
}

fn admin_limit_for_group_with_values(group: &str, read_limit: usize, mutation_limit: usize) -> usize {
    match group {
        "mutation" => mutation_limit,
        _ => read_limit,
    }
}

fn configured_limit(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
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

/// Per-admin-user rate limit middleware. Runs after Clerk auth so mutating
/// admin routes are limited by verified actor instead of client-supplied IP.
pub async fn admin_rate_limit_middleware(
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let admin = match request.extensions().get::<AdminAuthContext>() {
        Some(admin) => admin,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "admin_context_missing",
                    "message": "Admin authentication context is missing"
                })),
            ));
        }
    };

    let group = admin_endpoint_group(request.method());
    let limit = admin_limit_for_group(group);
    let key = format!("admin:{}:{}", admin.subject, group);

    static STORE: std::sync::OnceLock<RateLimitStore> = std::sync::OnceLock::new();
    let store = STORE.get_or_init(RateLimitStore::new);

    let (allowed, remaining, reset_at) = store.check_rate_limit(&key, limit, 60).await;

    if !allowed {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "rate_limit_exceeded",
                "message": "Too many admin requests",
                "reset_at": reset_at
            })),
        ));
    }

    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "X-RateLimit-Limit",
        limit.to_string().parse().unwrap(),
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

/// Per-IP rate limit for the public `/health` liveness probe.
/// Reuses the unauthenticated-IP budget (10/min) since `/health` is the same
/// threat model: an unauthenticated public endpoint. Docker's HEALTHCHECK
/// probes from 127.0.0.1 ~2/min, well under this cap.
pub async fn health_ip_rate_limit_middleware(
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

    let key = format!("health-ip:{}", client_ip);

    static STORE: std::sync::OnceLock<RateLimitStore> = std::sync::OnceLock::new();
    let store = STORE.get_or_init(RateLimitStore::new);

    let (allowed, remaining, reset_at) = store
        .check_rate_limit(&key, UNAUTHENTICATED_IP_LIMIT, UNAUTHENTICATED_IP_WINDOW_SECS)
        .await;

    if !allowed {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "rate_limit_exceeded",
                "message": "Too many health check requests",
                "reset_at": reset_at
            })),
        ));
    }

    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "X-RateLimit-Limit",
        UNAUTHENTICATED_IP_LIMIT.to_string().parse().unwrap(),
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

/// Per-IP guard for admin auth attempts. This runs before Clerk JWT parsing so
/// repeated bad admin tokens cannot force unbounded signature/JWKS work.
pub async fn admin_auth_ip_rate_limit_middleware(
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let client_ip = match extract_client_ip(&request) {
        Some(ip) => ip,
        None => {
            return Ok(next.run(request).await);
        }
    };

    let key = format!("admin-auth-ip:{}", client_ip);

    static CONFIG: std::sync::OnceLock<AdminAuthIpRateLimit> = std::sync::OnceLock::new();
    let config = CONFIG.get_or_init(|| AdminAuthIpRateLimit {
        limit: configured_limit("ADMIN_AUTH_IP_LIMIT", ADMIN_AUTH_IP_LIMIT_DEFAULT),
    });

    static STORE: std::sync::OnceLock<RateLimitStore> = std::sync::OnceLock::new();
    let store = STORE.get_or_init(RateLimitStore::new);

    let (current_count, reset_at) = store
        .current_usage(&key, ADMIN_AUTH_IP_WINDOW_SECS)
        .await;

    if current_count >= config.limit {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "rate_limit_exceeded",
                "message": "Too many admin authentication failures",
                "reset_at": reset_at
            })),
        ));
    }

    let response = next.run(request).await;

    if response.status() == StatusCode::UNAUTHORIZED {
        let _ = store
            .record_event(&key, ADMIN_AUTH_IP_WINDOW_SECS)
            .await;
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::{admin_endpoint_group, admin_limit_for_group_with_values, effective_limit_for_group, endpoint_group, extract_client_ip_with, parse_trusted_proxies, RateLimitStore};
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
    fn client_ip_honors_forwarded_for_when_peer_is_trusted_proxy() {
        // Right-to-left: the rightmost untrusted entry is selected. nginx
        // appends the real client IP, so it is the rightmost hop, not the
        // client-supplied leftmost.
        let mut request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-forwarded-for", "198.51.100.5, 203.0.113.10")
            .header("x-real-ip", "203.0.113.10")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            443,
        )));
        let trusted = [IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))];

        assert_eq!(extract_client_ip_with(&request, &trusted).as_deref(), Some("203.0.113.10"));
    }

    #[test]
    fn client_ip_ignores_spoofed_leftmost_xff_entry() {
        // Regression for the XFF-append spoofing vector: a client injects a
        // leftmost XFF entry; nginx appends the real client IP to the right.
        // The spoofed value must NOT be selected as the bucket key.
        let mut request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-forwarded-for", "1.2.3.4, 198.51.100.5")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            443,
        )));
        let trusted = [IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))];

        assert_eq!(extract_client_ip_with(&request, &trusted).as_deref(), Some("198.51.100.5"));
    }

    #[test]
    fn client_ip_skips_trusted_proxy_hops_in_xff_chain() {
        // Proxy chaining: real_client -> trusted_proxy2 -> trusted_proxy1 ->
        // Bridge. XFF = "real_client, proxy2, proxy1". Right-to-left skips
        // proxy1 and proxy2 (both trusted), selects real_client.
        let mut request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-forwarded-for", "198.51.100.5, 10.0.0.2, 10.0.0.1")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            443,
        )));
        let trusted = [
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
        ];

        assert_eq!(extract_client_ip_with(&request, &trusted).as_deref(), Some("198.51.100.5"));
    }

    #[test]
    fn client_ip_honors_x_real_ip_when_peer_is_trusted_and_xff_absent() {
        let mut request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-real-ip", "203.0.113.10")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            443,
        )));
        let trusted = [IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))];

        assert_eq!(extract_client_ip_with(&request, &trusted).as_deref(), Some("203.0.113.10"));
    }

    #[test]
    fn client_ip_ignores_forwarded_headers_when_peer_is_untrusted() {
        // Spoofed X-Forwarded-For from a peer that is not a configured trusted
        // proxy must map to the real peer IP, not the attacker-supplied value.
        let mut request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-forwarded-for", "198.51.100.5")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9)),
            443,
        )));
        let trusted = [IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))];

        assert_eq!(extract_client_ip_with(&request, &trusted).as_deref(), Some("192.0.2.9"));
    }

    #[test]
    fn client_ip_uses_peer_when_no_trusted_proxies_configured() {
        // Default posture: no trusted proxies => forwarded headers ignored,
        // socket peer used directly.
        let mut request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-forwarded-for", "198.51.100.5")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)),
            443,
        )));
        let trusted: [IpAddr; 0] = [];

        assert_eq!(extract_client_ip_with(&request, &trusted).as_deref(), Some("203.0.113.10"));
    }

    #[test]
    fn client_ip_uses_connect_info_when_no_forwarded_headers() {
        let mut request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)),
            443,
        )));
        let trusted = [IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))];

        assert_eq!(extract_client_ip_with(&request, &trusted).as_deref(), Some("203.0.113.10"));
    }

    #[test]
    fn client_ip_returns_none_without_socket_peer_even_with_forwarded_headers() {
        // Without a socket peer to anchor trust on, forwarded headers must not
        // be honored: the middleware treats a missing IP as "do not block".
        let request = Request::builder()
            .uri("/api/v1/payment/checkout")
            .header("x-forwarded-for", "198.51.100.5")
            .body(Body::empty())
            .unwrap();
        let trusted = [IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))];

        assert_eq!(extract_client_ip_with(&request, &trusted), None);
    }

    #[tokio::test]
    async fn current_usage_does_not_create_a_key_for_unseen_bucket() {
        let store = RateLimitStore::new();
        let (count, _) = store.current_usage("ip:198.51.100.5", 60).await;
        assert_eq!(count, 0);

        let limits = store.limits.lock().await;
        assert!(!limits.contains_key("ip:198.51.100.5"));
    }

    #[tokio::test]
    async fn current_usage_evicts_bucket_once_its_window_drains() {
        let store = RateLimitStore::new();
        // A zero-length window makes the recorded timestamp immediately stale
        // for the next read, exercising the drain-and-evict path without time
        // travel.
        let _ = store.record_event("ip:9.9.9.9", 0).await;
        let (count, _) = store.current_usage("ip:9.9.9.9", 0).await;
        assert_eq!(count, 0);

        let limits = store.limits.lock().await;
        assert!(!limits.contains_key("ip:9.9.9.9"));
    }

    #[test]
    fn parse_trusted_proxies_accepts_valid_ip_literals() {
        let proxies = parse_trusted_proxies("127.0.0.1, ::1 , 10.0.0.5").unwrap();
        assert_eq!(proxies.len(), 3);
        assert_eq!(proxies[0], IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
        assert_eq!(proxies[2], IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)));
    }

    #[test]
    fn parse_trusted_proxies_ignores_empty_tokens() {
        // Trailing/leading commas and whitespace must not be treated as
        // invalid entries.
        let proxies = parse_trusted_proxies("127.0.0.1, , ").unwrap();
        assert_eq!(proxies, vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))]);
    }

    #[test]
    fn parse_trusted_proxies_rejects_cidr_range() {
        // CIDR is a common way to express a proxy network; rejecting it loudly
        // (rather than silently dropping) tells the operator to list literals.
        let err = parse_trusted_proxies("127.0.0.1, 10.0.0.0/8").unwrap_err();
        assert!(err.contains("10.0.0.0/8"), "error must name the bad entry");
        assert!(err.contains("CIDR"), "error must explain CIDR is unsupported");
    }

    #[test]
    fn parse_trusted_proxies_rejects_typo() {
        let err = parse_trusted_proxies("127.0.0.1, typo").unwrap_err();
        assert!(err.contains("typo"), "error must name the bad entry");
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

    #[test]
    fn admin_mutations_use_tighter_rate_limit_bucket() {
        assert_eq!(admin_endpoint_group(&Method::PATCH), "mutation");
        assert_eq!(admin_limit_for_group_with_values("mutation", 240, 30), 30);
        assert_eq!(admin_endpoint_group(&Method::GET), "read");
        assert_eq!(admin_limit_for_group_with_values("read", 240, 30), 240);
    }
}
