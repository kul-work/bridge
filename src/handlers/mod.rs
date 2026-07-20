pub mod api_key;
pub mod checkout;
pub mod verify_purchase;
pub mod subscriptions;
pub mod subscriptions_actions;
pub mod payments;
pub mod admin;
pub mod users;
pub mod test_log;
pub mod routes;

pub use routes::{list_routes, openapi_spec, plain_routes, RouteDescriptor, RoutesIndexResponse};

/// Health check handler
pub async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION"),
        "revision": crate::BUILD_GIT_SHA,
    }))
}

fn readiness_body(status: &str, enabled_provider_configs: i64) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "database": "ok",
        "enabled_provider_configs": enabled_provider_configs,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION"),
        "revision": crate::BUILD_GIT_SHA,
    })
}

/// Readiness check handler
pub async fn readiness_check(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
) -> Result<(axum::http::StatusCode, axum::Json<serde_json::Value>), crate::error::BridgeError> {
    let db = state.database();
    let count = crate::db::readiness::count_enabled_provider_configs(db.pool()).await
        .map_err(|err| {
            tracing::error!(
                signal_class = "alert_signal",
                alert_key = "bridge.db.readiness_failed",
                alert_severity = "page",
                alert_subject = "Bridge database readiness failed",
                error = %err,
                "Database readiness check failed"
            );
            crate::error::BridgeError::DbError("Database readiness check failed".to_string())
        })?;

    if count == 0 {
        Ok((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(readiness_body("not_ready", 0))
        ))
    } else {
        Ok((
            axum::http::StatusCode::OK,
            axum::Json(readiness_body("ready", count))
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{health_check, readiness_body};

    #[tokio::test]
    async fn health_response_exposes_build_metadata() {
        let body = health_check().await.0;
        let expected_revision = option_env!("BRIDGE_BUILD_GIT_SHA").unwrap_or("unknown");

        assert_eq!(body["status"], "healthy");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(crate::BUILD_GIT_SHA, expected_revision);
        assert_eq!(body["revision"], expected_revision);
        assert!(body["timestamp"].is_string());
    }

    #[test]
    fn readiness_responses_expose_build_metadata() {
        let expected_revision = option_env!("BRIDGE_BUILD_GIT_SHA").unwrap_or("unknown");

        for (status, count) in [("not_ready", 0), ("ready", 2)] {
            let body = readiness_body(status, count);

            assert_eq!(body["status"], status);
            assert_eq!(body["database"], "ok");
            assert_eq!(body["enabled_provider_configs"], count);
            assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
            assert_eq!(body["revision"], expected_revision);
            assert!(body["timestamp"].is_string());
        }
    }
}
