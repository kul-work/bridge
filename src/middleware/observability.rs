use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};
use http::{HeaderName, HeaderValue};
use std::time::Instant;
use uuid::Uuid;
use tracing::Instrument;
use regex::Regex;
use std::sync::OnceLock;

use crate::handlers::api_key::AppAuth;
use crate::middleware::admin_auth::AdminAuthContext;

const REQUEST_ID_HEADER: &str = "x-request-id";
const ERROR_CODE_HEADER: &str = "x-error-code";

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct RequestId(pub String);

pub async fn request_observability(mut req: Request, next: Next) -> Response {
    let request_id = request_id(&req);
    let method = req.method().clone();
    let raw_path = req.uri().path().to_string();
    let started_at = Instant::now();

    req.extensions_mut().insert(RequestId(request_id.clone()));

    let span = tracing::info_span!("http_request", request_id = %request_id);
    let mut response = next.run(req).instrument(span).await;
    let latency_ms = started_at.elapsed().as_millis() as u64;
    let status = response.status();

    let app_id = response
        .extensions()
        .get::<AppAuth>()
        .map(|auth| auth.app_id.to_string());
    let admin_subject = response
        .extensions()
        .get::<AdminAuthContext>()
        .map(|ctx| ctx.subject.clone());

    let error_code = response
        .headers()
        .get(ERROR_CODE_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let matched_path = response
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|mp| mp.as_str().to_string());
    let path = matched_path.unwrap_or_else(|| scrub_path(&raw_path));

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(REQUEST_ID_HEADER), value);
    }

    tracing::info!(
        request_id = %request_id,
        method = %method,
        path = %path,
        status = status.as_u16(),
        latency_ms,
        app_id = app_id.as_deref(),
        admin_subject = admin_subject.as_deref(),
        error_code = error_code.as_deref(),
        "HTTP request completed"
    );

    response
}

fn request_id(req: &Request) -> String {
    let provided = req.headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(id) = provided {
        // Strict pattern: alphanumeric characters and hyphens only, length between 8 and 64
        let is_safe = id.len() >= 8 && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if is_safe {
            return id.to_string();
        }
    }

    Uuid::new_v4().to_string()
}

fn scrub_path(path: &str) -> String {
    static PATH_REDAC_REGEXS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let rules = PATH_REDAC_REGEXS.get_or_init(|| {
        vec![
            (Regex::new(r"^/webhooks/[^/]+/(.*)$").unwrap(), "/webhooks/[redacted_token]/$1"),
            (Regex::new(r"^/api/v1/users/[^/]+/(.*)$").unwrap(), "/api/v1/users/[redacted_user_id]/$1"),
            (Regex::new(r"^/api/v1/subscriptions/[^/]+(.*)$").unwrap(), "/api/v1/subscriptions/[redacted_sub_id]$1"),
        ]
    });

    let mut scrubbed = path.to_string();
    for (re, replacement) in rules {
        if re.is_match(&scrubbed) {
            scrubbed = re.replace(&scrubbed, *replacement).to_string();
        }
    }
    scrubbed
}

pub async fn capture_matched_path(req: Request, next: Next) -> Response {
    let matched_path = req.extensions().get::<axum::extract::MatchedPath>().cloned();
    let mut response = next.run(req).await;
    if let Some(mp) = matched_path {
        response.extensions_mut().insert(mp);
    }
    response
}
