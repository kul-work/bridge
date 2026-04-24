use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::{
    application::{
        app_context::{AppSnapshot, ProviderConfigSnapshot},
        checkout,
        checkout_types::CheckoutRequest,
    },
    db::checkout_idempotency::CachedCheckout,
    error::BridgeError,
    ports::traits::{
        AppConfigRepository, AppLookupRepository, CheckoutRepository,
        ProviderConfigLookupRepository,
    },
};

struct TestRepo {
    app: AppSnapshot,
    provider_config: ProviderConfigSnapshot,
}

#[async_trait]
impl AppLookupRepository for TestRepo {
    async fn get_app(&self, app_id: Uuid) -> Result<AppSnapshot, BridgeError> {
        assert_eq!(app_id, self.app.id);
        Ok(self.app.clone())
    }
}

#[async_trait]
impl ProviderConfigLookupRepository for TestRepo {
    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfigSnapshot, BridgeError> {
        assert_eq!(app_id, self.app.id);
        assert_eq!(provider, "creem");
        Ok(self.provider_config.clone())
    }
}

impl AppConfigRepository for TestRepo {}

#[async_trait]
impl CheckoutRepository for TestRepo {
    async fn get_cached_checkout(
        &self,
        _app_id: Uuid,
        _idempotency_key: &str,
    ) -> Result<Option<CachedCheckout>, BridgeError> {
        Ok(None)
    }

    async fn cache_checkout_response(
        &self,
        _app_id: Uuid,
        _idempotency_key: &str,
        _request_fingerprint: &str,
        _response_payload: &Value,
    ) -> Result<(), BridgeError> {
        Ok(())
    }
}

async fn record_checkout_request(
    State(captured): State<Arc<Mutex<Option<Value>>>>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    *captured.lock().expect("request capture mutex poisoned") = Some(payload);

    Json(json!({
        "id": "co_test_123",
        "checkout_url": "https://checkout.creem.test/session"
    }))
}

fn build_test_repo(app_id: Uuid, api_url: &str) -> TestRepo {
    TestRepo {
        app: AppSnapshot {
            id: app_id,
            slug: "test-app".to_string(),
            display_name: "Test App".to_string(),
            webhook_callback_url: "https://example.com/webhooks".to_string(),
            webhook_callback_secret: "secret".to_string(),
            api_rate_limit_per_minute: 120,
            api_rate_limit_rules: None,
            app_url: Some("https://example.com".to_string()),
            google_package_name: None,
            apple_bundle_id: None,
        },
        provider_config: ProviderConfigSnapshot {
            config: json!({
                "api_key": "sk_test_creem",
                "api_url": api_url,
                "offer_id": "prod_offer",
                "otp_id": "prod_otp"
            }),
        },
    }
}

async fn run_creem_checkout_test(request: CheckoutRequest) -> Value {
    let captured_request = Arc::new(Mutex::new(None));
    let app = Router::new()
        .route("/checkouts", post(record_checkout_request))
        .with_state(captured_request.clone());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let server_addr = listener
        .local_addr()
        .expect("test server should expose a local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("test Creem server should stay healthy");
    });

    let app_id = Uuid::new_v4();
    let repo = build_test_repo(app_id, &format!("http://{}", server_addr));

    let result = checkout::create_checkout(&repo, app_id, request).await;

    server.abort();

    let response = result.expect("checkout should succeed against the test Creem server");
    assert_eq!(response.checkout_id, "co_test_123");

    let recorded = captured_request
        .lock()
        .expect("request capture mutex poisoned")
        .clone()
        .expect("Creem request should be captured");

    recorded
}

#[tokio::test]
async fn creem_checkout_uses_requested_product_id_when_no_explicit_selector_is_provided() {
    let recorded = run_creem_checkout_test(CheckoutRequest {
        external_user_id: "user_123".to_string(),
        email: "user@example.com".to_string(),
        provider: "creem".to_string(),
        product_id: "prod_requested".to_string(),
        product_type: None,
        idempotency_key: None,
    })
    .await;

    assert_eq!(recorded["product_id"], "prod_requested");
    assert_eq!(recorded["metadata"]["product_id"], "prod_requested");
}

#[tokio::test]
async fn creem_checkout_uses_offer_id_when_offer_selector_is_requested() {
    let recorded = run_creem_checkout_test(CheckoutRequest {
        external_user_id: "user_123".to_string(),
        email: "user@example.com".to_string(),
        provider: "creem".to_string(),
        product_id: "prod_requested".to_string(),
        product_type: Some("offer".to_string()),
        idempotency_key: None,
    })
    .await;

    assert_eq!(recorded["product_id"], "prod_offer");
    assert_eq!(recorded["metadata"]["product_id"], "prod_requested");
}
