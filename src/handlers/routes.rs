//! Route index + OpenAPI spec for security scanners (OWASP ZAP) and quick API
//! discovery. Available only outside production — see the `ROUTES_INDEX_*`
//! mount gate in `build_app`. The single source of truth for the route list is
//! `known_routes()` in `lib.rs`; this module renders it as either an
//! interactive Swagger UI page (browsers) or a JSON index (everything else),
//! plus a minimal OpenAPI 3.0.3 document for ZAP's "Import → OpenAPI" flow.
//!
//! Ported from `household/src/handlers/routes.rs` with Bridge-specific groups
//! (`liveness/public/admin/api_key/webhook/test`), security schemes
//! (`apiKeyAuth`/`adminAuth`; webhook token is a path param, not a header
//! scheme), and per-route request-body examples anchored in Bridge's actual
//! `Json<T>` shapes.

use axum::{
    response::{Html, IntoResponse, Response},
    Json,
};
use http::{header, HeaderMap};
use serde::Serialize;

use crate::known_routes;

// ─── Response types ──────────────────────────────────────────────────────────

/// One entry in the route index. `group` and `auth` are short labels kept in
/// sync with `known_routes()` in `lib.rs`.
#[derive(Serialize)]
pub struct RouteDescriptor {
    pub method: String,
    pub path: String,
    pub group: &'static str,
    pub auth: &'static str,
}

#[derive(Serialize)]
pub struct RoutesIndexResponse {
    pub routes: Vec<RouteDescriptor>,
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /routes` — content-negotiated route index. Browsers
/// (`Accept: text/html`) get the interactive Swagger UI page; everything else
/// gets the JSON payload that programmatic scanners consume.
pub async fn list_routes(headers: HeaderMap) -> Response {
    let wants_html = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.contains("text/html"))
        .unwrap_or(false);

    if wants_html {
        return Html(render_swagger_ui(
            "Bridge API",
            "/routes/openapi",
            // Bridge authenticates protected routes with `Authorization: Bearer
            // <api_key>` and admin routes with `Authorization: Bearer <jwt>`.
            // Both share the same header, so the dev bar pre-seeds just that one.
            &[("Authorization", "")],
        ))
        .into_response();
    }

    Json(RoutesIndexResponse {
        routes: known_routes(),
    })
    .into_response()
}

/// `GET /routes/openapi` — minimal OpenAPI 3.0.3 document built from
/// `known_routes()`. Intended for ZAP's "Import → OpenAPI/Swagger" flow so
/// scanners get correct HTTP methods and `{name}` path params.
pub async fn openapi_spec(headers: HeaderMap) -> Response {
    Json(build_openapi(&headers)).into_response()
}

fn build_openapi(headers: &HeaderMap) -> serde_json::Value {
    let routes = known_routes();
    let mut paths = serde_json::Map::new();

    for route in &routes {
        let oa_path = openapi_path(&route.path);
        let method_key = route.method.to_ascii_lowercase();
        let path_item = paths
            .entry(oa_path)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .expect("path item is an object");

        let mut operation = serde_json::Map::new();
        operation.insert("tags".into(), serde_json::json!([route.group]));
        operation.insert("summary".into(), serde_json::json!(summary_for(route.group)));
        operation.insert(
            "operationId".into(),
            serde_json::json!(operation_id(&route.method, &route.path)),
        );
        if let Some(params) = combined_parameters(&route.method, &route.path) {
            operation.insert("parameters".into(), params);
        }
        if let Some(body) = request_body_for(&route.method, &route.path) {
            operation.insert("requestBody".into(), body);
        }
        if let Some(security) = security_for(route.group) {
            operation.insert("security".into(), security);
        }
        // `responses` is required by OpenAPI 3.x and Swagger UI needs it to
        // render the "Server response" panel after Execute.
        operation.insert("responses".into(), responses_for(route.group));

        path_item.insert(method_key, serde_json::Value::Object(operation));
    }

    let mut doc = serde_json::json!({
        "openapi": "3.0.3",
        "info": {
            "title": "Bridge API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Auto-generated route index for security scanning. Minimal schema — intended for discovery, not as a full API contract. Available in development/staging only."
        },
        "tags": [
            {"name": "liveness", "description": "Unauthenticated liveness/readiness probes."},
            {"name": "admin", "description": "Admin dashboard API requiring a Clerk admin JWT."},
            {"name": "api_key", "description": "App-facing API requiring a valid app API key sent as `Authorization: Bearer <key>`."},
            {"name": "webhook", "description": "Provider webhook ingress; the webhook secret token is the `:token` path segment."},
            {"name": "test", "description": "Loopback-only diagnostic endpoint, mounted only with MOCK_EXTERNAL_APIS."}
        ],
        "paths": serde_json::Value::Object(paths),
        "components": {
            "securitySchemes": {
                "apiKeyAuth": {"type": "http", "scheme": "bearer", "bearerFormat": "API key"},
                "adminAuth": {"type": "http", "scheme": "bearer", "bearerFormat": "Clerk JWT"}
            }
        }
    });

    if let Some(server) = server_url(headers) {
        doc.as_object_mut()
            .expect("doc is an object")
            .insert("servers".into(), serde_json::json!([{"url": server}]));
    }

    doc
}

// ─── OpenAPI shape helpers ───────────────────────────────────────────────────

/// Converts an Axum path template (`:param`) to OpenAPI style (`{param}`).
fn openapi_path(path: &str) -> String {
    path.split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(name) => format!("{{{name}}}"),
            None => seg.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Path-param entries with concrete `example` values so ZAP/Swagger issue real
/// requests without manual substitution. Schema is derived from param
/// semantics: db IDs are UUIDs; external/provider/token IDs are free-form
/// strings. Empty when the path has no params.
fn path_params(path: &str) -> Vec<serde_json::Value> {
    path.split('/')
        .filter_map(|seg| {
            seg.strip_prefix(':').map(|name| {
                let (schema, example): (serde_json::Value, &'static str) = match name {
                    // Database IDs (uuid format + zero-UUID example).
                    "app_id" | "webhook_id" => (
                        serde_json::json!({"type": "string", "format": "uuid"}),
                        "00000000-0000-0000-0000-000000000000",
                    ),
                    // External/provider/subscription IDs and webhook tokens are
                    // free-form strings, not UUIDs.
                    "subscription_id" => (
                        serde_json::json!({"type": "string"}),
                        "demo_subscription_id",
                    ),
                    "external_user_id" => (
                        serde_json::json!({"type": "string"}),
                        "user_demoExternalUserId",
                    ),
                    // Webhook secret token (path segment). Not a header-based
                    // security scheme — OpenAPI 3.0 has no `in: path` security,
                    // so the token is declared here as a path param.
                    "token" => (
                        serde_json::json!({"type": "string"}),
                        "demo-webhook-token",
                    ),
                    _ => (serde_json::json!({"type": "string"}), "sample"),
                };
                serde_json::json!({
                    "name": name,
                    "in": "path",
                    "required": true,
                    "schema": schema,
                    "example": example,
                })
            })
        })
        .collect()
}

/// Query-param entries for routes whose handlers extract `Query<T>`. Each
/// carries a concrete `example` so Swagger UI pre-fills "Try it out" and ZAP
/// sends a real query string instead of an empty one (which would trip
/// required-field deserialization).
fn query_params_for(method: &str, path: &str) -> Vec<serde_json::Value> {
    match (method, path) {
        // Payments list: external_user_id required, limit/after optional.
        ("GET", "/api/v1/payments") => vec![
            serde_json::json!({
                "name": "external_user_id", "in": "query", "required": true,
                "schema": {"type": "string"}, "example": "user_demoExternalUserId"
            }),
            serde_json::json!({
                "name": "limit", "in": "query", "required": false,
                "schema": {"type": "integer"}, "example": 20
            }),
            serde_json::json!({
                "name": "after", "in": "query", "required": false,
                "schema": {"type": "string"}, "example": ""
            }),
        ],
        // Subscription actions (cancel/resume/portal) all share
        // SubscriptionActionQuery { external_user_id, provider }, both required.
        ("POST", "/api/v1/subscriptions/:subscription_id/cancel")
        | ("POST", "/api/v1/subscriptions/:subscription_id/resume")
        | ("POST", "/api/v1/subscriptions/:subscription_id/portal") => vec![
            serde_json::json!({
                "name": "external_user_id", "in": "query", "required": true,
                "schema": {"type": "string"}, "example": "user_demoExternalUserId"
            }),
            serde_json::json!({
                "name": "provider", "in": "query", "required": true,
                "schema": {"type": "string"}, "example": "google_play"
            }),
        ],
        // Admin webhook list: page optional.
        ("GET", "/admin/apps/:app_id/webhooks") => vec![serde_json::json!({
            "name": "page", "in": "query", "required": false,
            "schema": {"type": "integer"}, "example": 1
        })],
        _ => Vec::new(),
    }
}

fn combined_parameters(method: &str, path: &str) -> Option<serde_json::Value> {
    let mut params = path_params(path);
    params.extend(query_params_for(method, path));
    if params.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(params))
    }
}

/// Declares `requestBody` (application/json) for routes whose handlers extract
/// `Json<T>`. Returns `None` for GET/DELETE and body-less POSTs so ZAP won't
/// set `Content-Type` where it doesn't belong. The body carries a concrete
/// demo `example` so Swagger UI pre-fills "Try it out" and ZAP's active scan
/// uses a real object as baseline.
fn request_body_for(method: &str, path: &str) -> Option<serde_json::Value> {
    let (example, required) = request_body_example(method, path)?;
    Some(serde_json::json!({
        "required": required,
        "content": {
            "application/json": {
                "schema": {"type": "object"},
                "example": example
            }
        }
    }))
}

/// Static demo JSON object per JSON-body route, plus whether the body is
/// required. Values pass Rust deserialization (matching the `Json<T>` struct)
/// but don't aim to satisfy deeper business validation. `None` for body-less
/// routes. Add a new arm here when a new JSON-body route is added to
/// `known_routes()`.
fn request_body_example(method: &str, path: &str) -> Option<(serde_json::Value, bool)> {
    Some(match (method, path) {
        ("POST", "/api/v1/app/verify") => (serde_json::json!({
            "expected_slug": "demo-app",
            "webhook_secret_nonce": "demo-nonce",
            "webhook_secret_issued_at": 1700000000_i64,
            "webhook_secret_proof": "sha256=demo-proof"
        }), true),
        ("POST", "/api/v1/payment/checkout") => (serde_json::json!({
            "external_user_id": "user_demoExternalUserId",
            "email": "demo@example.test",
            "provider": "creem",
            "product_id": "demo_product_id",
            "product_type": "subscription",
            "idempotency_key": ""
        }), true),
        ("POST", "/api/v1/verify-purchase") => (serde_json::json!({
            "external_user_id": "user_demoExternalUserId",
            "provider": "google_play",
            "subscription_id": "demo_subscription_id",
            "purchase_token": "demo_purchase_token",
            "product_type": "subscription",
            "currency": "USD"
        }), true),
        ("POST", "/api/v1/purchase/register") => (serde_json::json!({
            "external_user_id": "user_demoExternalUserId",
            "subscription_id": "demo_subscription_id",
            "provider": "google_play"
        }), true),
        // cancel_subscription takes Option<Json<CancelSubscriptionRequest>> —
        // body is optional; all its fields are optional too.
        ("POST", "/api/v1/subscriptions/:subscription_id/cancel") => (serde_json::json!({
            "mode": "immediate",
            "purchase_token": "",
            "on_execute": ""
        }), false),
        ("POST", "/api/v1/subscriptions/:subscription_id/acknowledge") => (serde_json::json!({
            "external_user_id": "user_demoExternalUserId"
        }), true),
        ("POST", "/api/v1/subscriptions/:subscription_id/price-step-up/accept")
        | ("POST", "/api/v1/subscriptions/:subscription_id/price-step-up/decline") => (serde_json::json!({
            "external_user_id": "user_demoExternalUserId"
        }), true),
        // anonymize takes Json<AnonymizeRequest { reason: Option<String> }>.
        ("POST", "/api/v1/users/:external_user_id/anonymize") => (serde_json::json!({
            "reason": "demo"
        }), true),
        // Webhook ingress consumes a raw provider payload (Pub/Sub envelope for
        // Google Play, raw JSON for Creem), not a fixed Json<T>. Declaring a
        // minimal application/json body keeps ZAP from skipping these routes.
        ("POST", "/webhooks/:token/google_play") => (serde_json::json!({
            "message": {"data": "", "messageId": "demo-message-id"}
        }), true),
        ("POST", "/webhooks/:token/creem") => (serde_json::json!({
            "event_type": "checkout.completed",
            "object_id": "demo-object-id"
        }), true),
        ("POST", "/internal/test/log-marker") => (serde_json::json!({
            "message": "demo marker"
        }), true),
        ("PATCH", "/admin/apps/:app_id/notes") => (serde_json::json!({
            "notes": "Demo notes"
        }), true),
        ("POST", "/admin/trigger-jobs") => (serde_json::json!({
            "jobs": ["webhook_retry", "reconciliation"]
        }), true),
        _ => return None,
    })
}

/// Maps a route group to its OpenAPI `security` requirement. `None` means no
/// declared security (liveness/public/webhook/test). Webhook auth is the
/// `:token` path param, not a header scheme, so it has no security entry.
fn security_for(group: &str) -> Option<serde_json::Value> {
    match group {
        "api_key" => Some(serde_json::json!([{"apiKeyAuth": []}])),
        "admin" => Some(serde_json::json!([{"adminAuth": []}])),
        _ => None,
    }
}

/// Minimal `responses` per group. Secured routes declare 401; the loopback-only
/// `test` group declares 403 (non-loopback clients get forbidden).
fn responses_for(group: &str) -> serde_json::Value {
    let ok = serde_json::json!({"description": "OK"});
    let unauthorized = serde_json::json!({"description": "Unauthorized — see security scheme"});
    match group {
        "api_key" | "admin" => serde_json::json!({"200": ok, "401": unauthorized}),
        "webhook" => serde_json::json!({
            "200": ok,
            "401": serde_json::json!({"description": "Unauthorized — invalid or unknown webhook token"})
        }),
        "test" => serde_json::json!({
            "200": ok,
            "403": serde_json::json!({"description": "Forbidden — loopback clients only"})
        }),
        _ => serde_json::json!({"200": ok}),
    }
}

fn summary_for(group: &str) -> &'static str {
    match group {
        "liveness" => "Liveness/readiness probe",
        "admin" => "Admin route (Clerk admin JWT required)",
        "api_key" => "App-facing API route (API key required)",
        "webhook" => "Provider webhook ingress (path token required)",
        "test" => "Loopback-only diagnostic (mock mode only)",
        _ => "Route",
    }
}

fn operation_id(method: &str, path: &str) -> String {
    let slug = path
        .trim_start_matches('/')
        .split('/')
        .map(|seg| seg.strip_prefix(':').unwrap_or(seg).replace('-', "_"))
        .collect::<Vec<_>>()
        .join("_");
    format!("{}_{}", method.to_ascii_lowercase(), slug)
}

/// Derives `servers[0].url` from the request `Host` header (+ `x-forwarded-proto`,
/// defaulting to `http` for dev). `None` when there's no Host (in-process tests),
/// so the `servers` array is omitted and ZAP prompts for the target on import.
fn server_url(headers: &HeaderMap) -> Option<String> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())?
        .trim();
    if host.is_empty() {
        return None;
    }
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_lowercase())
        .filter(|value| matches!(value.as_str(), "http" | "https"))
        .unwrap_or_else(|| "http".to_string());
    Some(format!("{scheme}://{host}"))
}

// ─── Swagger UI page ──────────────────────────────────────────────────────────
//
// Canonical template — keep in sync across household/bridge/hiha `routes.rs`.
// The page is parameterized by `{TITLE}`, `{SPEC_URL}`, and `{DEFAULT_HEADERS_JS}`
// so each app renders the same UI with its own title, spec endpoint, and
// pre-seeded dev headers.

fn render_swagger_ui(title: &str, spec_url: &str, default_headers: &[(&str, &str)]) -> String {
    let defaults_js = default_headers
        .iter()
        .map(|(name, value)| format!("{{ name: {:?}, value: {:?} }}", name, value))
        .collect::<Vec<_>>()
        .join(",\n      ");
    SWAGGER_UI_TEMPLATE
        .replace("{TITLE}", title)
        .replace("{SPEC_URL}", spec_url)
        .replace("{DEFAULT_HEADERS_JS}", &defaults_js)
}

const SWAGGER_UI_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{TITLE} — Swagger UI</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5.18.2/swagger-ui.css">
  <style>
    body { margin: 0; }
    .dev-bar {
      background: #1b1b2f;
      color: #e8e8f0;
      font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
      padding: 8px 14px;
      border-bottom: 1px solid #333;
      display: flex;
      flex-wrap: wrap;
      gap: 6px 10px;
      align-items: center;
      position: sticky;
      top: 0;
      z-index: 20;
    }
    .dev-bar h3 { margin: 0; font-size: 12px; font-weight: 600; color: #ffb86c; letter-spacing: 0.02em; }
    .dev-bar .hint { font-size: 11px; color: #8a8aa0; }
    .dev-bar .row { display: flex; gap: 4px; align-items: center; }
    .dev-bar input {
      background: #0f0f23;
      border: 1px solid #444;
      color: #e8e8f0;
      padding: 3px 6px;
      font-size: 12px;
      border-radius: 3px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      width: 150px;
    }
    .dev-bar input.name { width: 140px; color: #8be9fd; }
    .dev-bar button {
      background: #3a3a5e;
      border: 1px solid #555;
      color: #fff;
      padding: 3px 8px;
      font-size: 12px;
      border-radius: 3px;
      cursor: pointer;
    }
    .dev-bar button:hover { background: #4a4a7e; }
    .dev-bar button.danger { background: #5e2a2a; border-color: #7a3a3a; }
    .dev-bar button.danger:hover { background: #7a3a3a; }
    #header-rows { display: contents; }
    .swagger-ui-wrap { padding: 0 12px; }
  </style>
</head>
<body>
  <div class="dev-bar">
    <h3>Dev Headers</h3>
    <span class="hint">injected on every request · saved in localStorage</span>
    <div id="header-rows"></div>
    <button onclick="addHeader()" title="Add header">+ Add</button>
  </div>
  <div id="swagger-ui" class="swagger-ui-wrap"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5.18.2/swagger-ui-bundle.js" charset="UTF-8"></script>
  <script>
    const STORAGE_KEY = "bridge.devHeaders";
    const DEFAULTS = [
      {DEFAULT_HEADERS_JS}
    ];

    function load() {
      try {
        const s = JSON.parse(localStorage.getItem(STORAGE_KEY) || "null");
        // A saved empty array would hide the seeded DEFAULTS forever; treat it
        // as "no state" so the pre-seeded auth headers resurface.
        if (Array.isArray(s) && s.length > 0) return s;
      } catch (e) {}
      return DEFAULTS.slice();
    }
    function save(rows) {
      if (rows.length === 0) localStorage.removeItem(STORAGE_KEY);
      else localStorage.setItem(STORAGE_KEY, JSON.stringify(rows));
    }
    function esc(s) {
      return (s || "").replace(/[&<>"']/g, c => ({ "&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;" }[c]));
    }
    function render() {
      const rows = load();
      const c = document.getElementById("header-rows");
      c.innerHTML = "";
      rows.forEach((r, i) => {
        const d = document.createElement("div");
        d.className = "row";
        d.innerHTML =
          '<input class="name" placeholder="header" value="' + esc(r.name) + '" data-i="' + i + '">' +
          '<input placeholder="value" value="' + esc(r.value) + '" data-i="' + i + '">' +
          '<button class="danger" onclick="removeRow(' + i + ')" title="Remove">\u00d7</button>';
        c.appendChild(d);
      });
    }
    function collect() {
      const rows = [];
      document.querySelectorAll("#header-rows .row").forEach(d => {
        const inputs = d.querySelectorAll("input");
        const name = inputs[0].value.trim();
        const value = inputs[1].value;
        rows.push({ name, value });
      });
      save(rows);
      return rows;
    }
    function addHeader() {
      const rows = load(); rows.push({ name: "", value: "" }); save(rows); render();
    }
    function removeRow(i) {
      const rows = load(); rows.splice(i, 1); save(rows); render();
    }
    document.addEventListener("input", e => {
      if (e.target.closest && e.target.closest("#header-rows")) collect();
    });
    render();
    window.onload = () => {
      window.ui = SwaggerUIBundle({
        url: "{SPEC_URL}",
        dom_id: "#swagger-ui",
        deepLinking: true,
        requestInterceptor: (req) => {
          // Inject every dev header that has a name+value. Empty values are
          // skipped so the browser doesn't send bare header names.
          load().forEach(h => {
            if (h.name && h.value) req.headers[h.name] = h.value;
          });
          return req;
        }
      });
    };
  </script>
</body>
</html>"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_params_emits_uuid_schema_for_db_ids_and_string_for_others() {
        let params = path_params("/api/v1/subscriptions/:subscription_id");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0]["name"], "subscription_id");
        assert!(params[0]["schema"].get("format").is_none());
        assert_eq!(params[0]["example"], "demo_subscription_id");

        let params = path_params("/admin/apps/:app_id/notes");
        assert_eq!(params[0]["name"], "app_id");
        assert_eq!(params[0]["schema"]["format"], "uuid");
        assert_eq!(params[0]["example"], "00000000-0000-0000-0000-000000000000");

        let params = path_params("/webhooks/:token/google_play");
        assert_eq!(params[0]["name"], "token");
        assert!(params[0]["schema"].get("format").is_none());
        assert_eq!(params[0]["example"], "demo-webhook-token");
    }

    #[test]
    fn path_params_empty_for_routes_without_params() {
        assert!(path_params("/health").is_empty());
        assert!(path_params("/api/v1/payments").is_empty());
    }

    #[test]
    fn query_params_for_payments_declares_required_external_user_id() {
        let params = query_params_for("GET", "/api/v1/payments");
        assert_eq!(params.len(), 3);
        assert_eq!(params[0]["name"], "external_user_id");
        assert_eq!(params[0]["required"], true);
        assert_eq!(params[1]["name"], "limit");
        assert_eq!(params[1]["required"], false);
    }

    #[test]
    fn query_params_for_subscription_actions_share_required_pair() {
        for path in [
            "/api/v1/subscriptions/:subscription_id/cancel",
            "/api/v1/subscriptions/:subscription_id/resume",
            "/api/v1/subscriptions/:subscription_id/portal",
        ] {
            let params = query_params_for("POST", path);
            assert_eq!(params.len(), 2, "{path}");
            assert_eq!(params[0]["name"], "external_user_id");
            assert_eq!(params[0]["required"], true);
            assert_eq!(params[1]["name"], "provider");
            assert_eq!(params[1]["required"], true);
        }
    }

    #[test]
    fn query_params_for_routes_without_query_extractor_is_empty() {
        assert!(query_params_for("GET", "/health").is_empty());
        assert!(query_params_for("POST", "/api/v1/payment/checkout").is_empty());
        assert!(query_params_for("GET", "/admin/apps").is_empty());
    }

    #[test]
    fn request_body_for_declares_json_only_for_body_routes() {
        let with_body = [
            ("POST", "/api/v1/app/verify"),
            ("POST", "/api/v1/payment/checkout"),
            ("POST", "/api/v1/verify-purchase"),
            ("POST", "/api/v1/purchase/register"),
            ("POST", "/api/v1/subscriptions/:subscription_id/cancel"),
            ("POST", "/api/v1/subscriptions/:subscription_id/acknowledge"),
            ("POST", "/api/v1/subscriptions/:subscription_id/price-step-up/accept"),
            ("POST", "/api/v1/subscriptions/:subscription_id/price-step-up/decline"),
            ("POST", "/api/v1/users/:external_user_id/anonymize"),
            ("POST", "/webhooks/:token/google_play"),
            ("POST", "/webhooks/:token/creem"),
            ("POST", "/internal/test/log-marker"),
            ("PATCH", "/admin/apps/:app_id/notes"),
            ("POST", "/admin/trigger-jobs"),
        ];
        for (method, path) in with_body {
            let body = request_body_for(method, path)
                .unwrap_or_else(|| panic!("{method} {path} should declare a request body"));
            assert_eq!(
                body["content"]["application/json"]["schema"]["type"],
                "object"
            );
            assert!(
                body["content"]["application/json"]["example"].is_object(),
                "{method} {path} must include a non-null `example` object"
            );
        }

        // Body-less POSTs and all GET/DELETE must return None so ZAP won't set
        // Content-Type on them.
        let without_body = [
            ("POST", "/api/v1/subscriptions/:subscription_id/resume"),
            ("POST", "/api/v1/subscriptions/:subscription_id/portal"),
            ("POST", "/admin/webhooks/:webhook_id/retry"),
            ("GET", "/api/v1/subscriptions"),
            ("GET", "/api/v1/users/:external_user_id/data-export"),
            ("GET", "/admin/apps"),
        ];
        for (method, path) in without_body {
            assert!(
                request_body_for(method, path).is_none(),
                "{method} {path} should NOT declare a request body"
            );
        }
    }

    #[test]
    fn cancel_subscription_body_is_optional_others_required() {
        // cancel takes Option<Json<...>> → required: false.
        let cancel = request_body_for("POST", "/api/v1/subscriptions/:subscription_id/cancel").unwrap();
        assert_eq!(cancel["required"], false);
        // acknowledge takes Json<...> → required: true.
        let ack = request_body_for("POST", "/api/v1/subscriptions/:subscription_id/acknowledge").unwrap();
        assert_eq!(ack["required"], true);
    }

    #[test]
    fn security_for_maps_groups_to_schemes() {
        assert_eq!(security_for("api_key").unwrap()[0]["apiKeyAuth"], serde_json::json!([]));
        assert_eq!(security_for("admin").unwrap()[0]["adminAuth"], serde_json::json!([]));
        assert!(security_for("liveness").is_none());
        assert!(security_for("public").is_none());
        assert!(security_for("webhook").is_none());
        assert!(security_for("test").is_none());
    }

    #[test]
    fn responses_for_secured_routes_declare_401() {
        assert!(responses_for("api_key")["401"].is_object());
        assert!(responses_for("admin")["401"].is_object());
        assert!(responses_for("webhook")["401"].is_object());
        assert!(responses_for("test")["403"].is_object());
        assert!(responses_for("liveness")["200"].is_object());
        assert!(responses_for("liveness").get("401").is_none());
    }

    #[test]
    fn openapi_path_converts_axum_params_to_openapi() {
        assert_eq!(
            openapi_path("/api/v1/subscriptions/:subscription_id"),
            "/api/v1/subscriptions/{subscription_id}"
        );
        assert_eq!(openapi_path("/webhooks/:token/google_play"), "/webhooks/{token}/google_play");
        assert_eq!(openapi_path("/health"), "/health");
    }

    #[test]
    fn render_swagger_ui_substitutes_title_spec_url_and_defaults() {
        let html = render_swagger_ui("Bridge API", "/routes/openapi", &[("Authorization", "")]);
        assert!(html.contains("<title>Bridge API — Swagger UI</title>"));
        assert!(html.contains("\"/routes/openapi\""));
        assert!(html.contains("\"Authorization\""));
        assert!(html.contains("requestInterceptor"));
    }

    #[test]
    fn build_openapi_has_paths_and_security_schemes() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:5566".parse().unwrap());
        let doc = build_openapi(&headers);
        assert_eq!(doc["openapi"], "3.0.3");
        assert_eq!(doc["info"]["title"], "Bridge API");
        // Path params in OpenAPI {name} form.
        assert!(doc["paths"].get("/api/v1/subscriptions/{subscription_id}").is_some());
        assert!(doc["paths"].get("/webhooks/{token}/google_play").is_some());
        // Security schemes declared.
        assert!(doc["components"]["securitySchemes"]["apiKeyAuth"].is_object());
        assert!(doc["components"]["securitySchemes"]["adminAuth"].is_object());
        // Server URL derived from Host.
        assert_eq!(doc["servers"][0]["url"], "http://127.0.0.1:5566");
    }
}
