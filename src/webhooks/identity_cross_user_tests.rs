use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::{routing::post, Router};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::db;
use crate::webhooks::forwarding::{forward_webhook, WEBHOOK_DELIVERY_LEASE_SECS};
use crate::webhooks::processor::{build_canonical_payload, CanonicalWebhookPayload};

const SUBSCRIPTION_ID: &str = "hiha_monthly";
const PROVIDER: &str = "google_play";
const TA: i64 = 1_000_000;
const TB: i64 = 2_000_000;
const T_EVT: i64 = 1_500_000;

#[tokio::test]
async fn reconciliation_id_keyed_update_leaves_same_sku_other_row_untouched() -> Result<(), Box<dyn Error>> {
    let database = test_database().await?;
    let pool = database.pool();
    let app_id = insert_test_app(pool, "reconcile").await?;
    let result = run_reconciliation_regression(pool, app_id).await;
    cleanup_test_app(pool, app_id).await;
    result
}

async fn run_reconciliation_regression(pool: &PgPool, app_id: Uuid) -> Result<(), Box<dyn Error>> {
    let token_a = format!("tok_a_{app_id}");
    let token_b = format!("tok_b_{app_id}");
    let row_a = insert_google_subscription(pool, app_id, "user_a", &token_a, "active", TA).await?;
    let row_b = insert_google_subscription(pool, app_id, "user_b", &token_b, "cancelled", TB).await?;

    let updated = db::subscriptions::update_reconciled_subscription_status(
        pool,
        app_id,
        row_a.id,
        "expired",
        None,
        3_000_000,
    )
    .await?;
    assert!(updated, "expected the targeted row to be updated");

    let a_after = load_subscription_state(pool, row_a.id).await?;
    let b_after = load_subscription_state(pool, row_b.id).await?;

    assert_eq!(a_after.status, "expired");
    assert_eq!(a_after.version, row_a.version + 1);
    assert_eq!(a_after.last_event_time, 3_000_000);

    let duplicate = db::subscriptions::update_reconciled_subscription_status(
        pool,
        app_id,
        row_a.id,
        "expired",
        None,
        3_000_100,
    )
    .await?;
    assert!(!duplicate, "same-status reconciliation must not move the row again");

    let a_after_duplicate = load_subscription_state(pool, row_a.id).await?;
    assert_eq!(a_after_duplicate.version, a_after.version);
    assert_eq!(a_after_duplicate.last_event_time, a_after.last_event_time);

    assert_eq!(b_after.status, "cancelled");
    assert_eq!(b_after.version, row_b.version, "other same-SKU row version must not move");
    assert_eq!(b_after.last_event_time, TB, "other same-SKU row last_event_time must not move");

    Ok(())
}

#[tokio::test]
async fn forward_stale_check_compares_against_same_purchase_token_row() -> Result<(), Box<dyn Error>> {
    let database = test_database().await?;
    let pool = database.pool();
    let (callback_url, callback_count, server) = spawn_callback_server().await?;
    let app_id = insert_test_app_with_url(pool, "forward-stale", &callback_url).await?;
    let result = run_forward_stale_regression(&database, pool, app_id, callback_count.clone()).await;
    cleanup_test_app(pool, app_id).await;
    server.abort();
    result
}

async fn run_forward_stale_regression(
    database: &crate::db::Database,
    pool: &PgPool,
    app_id: Uuid,
    callback_count: Arc<AtomicUsize>,
) -> Result<(), Box<dyn Error>> {
    let token_a = format!("tok_a_{app_id}");
    let token_b = format!("tok_b_{app_id}");
    let _row_a = insert_google_subscription(pool, app_id, "user_a", &token_a, "active", TA).await?;
    let _row_b = insert_google_subscription(pool, app_id, "user_b", &token_b, "active", TB).await?;

    let provider_a = insert_webhook_provider(pool, app_id, &token_a, "evt_fwd_a").await?;
    let delivery_a = insert_webhook_delivery(pool, app_id, provider_a).await?;
    let provider_b = insert_webhook_provider(pool, app_id, &token_b, "evt_fwd_b").await?;
    let delivery_b = insert_webhook_delivery(pool, app_id, provider_b).await?;

    let claim_a = claim_delivery(pool, app_id, delivery_a).await?;
    let claim_b = claim_delivery(pool, app_id, delivery_b).await?;

    forward_webhook(database, app_id, delivery_a, claim_a, google_canonical_payload(&token_a, T_EVT, "subscription.cancelled")).await?;
    forward_webhook(database, app_id, delivery_b, claim_b, google_canonical_payload(&token_b, T_EVT, "subscription.cancelled")).await?;

    assert_eq!(callback_count.load(Ordering::SeqCst), 1, "only the same-token not-stale event must reach the app");

    let delivery_a_after = load_delivery(pool, delivery_a).await?;
    let delivery_b_after = load_delivery(pool, delivery_b).await?;
    let provider_b_after = load_webhook_provider_suppression(pool, provider_b).await?;

    assert!(delivery_a_after.forwarded, "token_a event (newer than its own row) must forward");
    assert_eq!(delivery_a_after.last_http_status, Some(200));
    assert!(delivery_b_after.forwarded, "suppression marks the delivery terminal");
    assert!(provider_b_after.0, "token_b event (older than its own row) must be suppressed before forward");
    assert_eq!(provider_b_after.1.as_deref(), Some("superseded_before_forward"));

    Ok(())
}

#[tokio::test]
async fn forward_stale_check_with_unmatched_token_does_not_suppress_against_same_sku_row() -> Result<(), Box<dyn Error>> {
    let database = test_database().await?;
    let pool = database.pool();
    let (callback_url, callback_count, server) = spawn_callback_server().await?;
    let app_id = insert_test_app_with_url(pool, "forward-unmatched", &callback_url).await?;
    let result = run_forward_unmatched_regression(&database, pool, app_id, callback_count).await;
    cleanup_test_app(pool, app_id).await;
    server.abort();
    result
}

#[tokio::test]
async fn forward_with_lost_claim_does_not_post_callback() -> Result<(), Box<dyn Error>> {
    let database = test_database().await?;
    let pool = database.pool();
    let (callback_url, callback_count, server) = spawn_callback_server().await?;
    let app_id = insert_test_app_with_url(pool, "lost-claim", &callback_url).await?;
    let result = run_forward_lost_claim_regression(&database, pool, app_id, callback_count).await;
    cleanup_test_app(pool, app_id).await;
    server.abort();
    result
}

async fn run_forward_unmatched_regression(
    database: &crate::db::Database,
    pool: &PgPool,
    app_id: Uuid,
    callback_count: Arc<AtomicUsize>,
) -> Result<(), Box<dyn Error>> {
    let token_a = format!("tok_a_{app_id}");
    let _row_a = insert_google_subscription(pool, app_id, "user_a", &token_a, "active", TA).await?;

    let unmatched_token = format!("tok_unmatched_{app_id}");
    let provider = insert_webhook_provider(pool, app_id, &unmatched_token, "evt_fwd_unmatched").await?;
    let delivery = insert_webhook_delivery(pool, app_id, provider).await?;

    let claim = claim_delivery(pool, app_id, delivery).await?;
    forward_webhook(database, app_id, delivery, claim, google_canonical_payload(&unmatched_token, T_EVT, "subscription.cancelled")).await?;

    assert_eq!(callback_count.load(Ordering::SeqCst), 1, "unmatched-token Google event must forward");
    let provider_after = load_webhook_provider_suppression(pool, provider).await?;
    assert!(!provider_after.0, "unmatched Google token must not suppress against a same-SKU row");

    Ok(())
}

async fn run_forward_lost_claim_regression(
    database: &crate::db::Database,
    pool: &PgPool,
    app_id: Uuid,
    callback_count: Arc<AtomicUsize>,
) -> Result<(), Box<dyn Error>> {
    let token = format!("tok_lost_claim_{app_id}");
    let provider = insert_webhook_provider(pool, app_id, &token, "evt_lost_claim").await?;
    let delivery = insert_webhook_delivery(pool, app_id, provider).await?;
    let stale_claim = claim_delivery(pool, app_id, delivery).await?;

    replace_delivery_claim(pool, delivery).await?;

    forward_webhook(
        database,
        app_id,
        delivery,
        stale_claim,
        google_canonical_payload(&token, T_EVT, "subscription.cancelled"),
    )
    .await?;

    assert_eq!(callback_count.load(Ordering::SeqCst), 0, "lost claim must not emit callback POST");

    let delivery_after = load_delivery(pool, delivery).await?;
    assert!(!delivery_after.forwarded);
    assert_eq!(delivery_after.forward_attempts, 0);

    Ok(())
}

#[tokio::test]
async fn canonical_rebuild_uses_purchase_token_row_for_google_play() -> Result<(), Box<dyn Error>> {
    let database = test_database().await?;
    let pool = database.pool();
    let app_id = insert_test_app(pool, "canonical-rebuild").await?;
    let result = run_canonical_rebuild_regression(&database, pool, app_id).await;
    cleanup_test_app(pool, app_id).await;
    result
}

async fn run_canonical_rebuild_regression(
    database: &crate::db::Database,
    pool: &PgPool,
    app_id: Uuid,
) -> Result<(), Box<dyn Error>> {
    let token_a = format!("tok_a_{app_id}");
    let token_b = format!("tok_b_{app_id}");
    let row_a = insert_google_subscription(pool, app_id, "user_a", &token_a, "active", TA).await?;
    let row_b = insert_google_subscription(pool, app_id, "user_b", &token_b, "past_due", TB).await?;

    let webhook_a = insert_webhook_provider(pool, app_id, &token_a, "evt_rebuild_a").await?;
    let webhook_b = insert_webhook_provider(pool, app_id, &token_b, "evt_rebuild_b").await?;

    let payload_a = build_canonical_payload(database, webhook_a, app_id).await?
        .expect("token_a rebuild must produce a payload");
    let payload_b = build_canonical_payload(database, webhook_b, app_id).await?
        .expect("token_b rebuild must produce a payload");

    assert_eq!(payload_a.provider, PROVIDER);
    assert_eq!(payload_a.purchase_token.as_deref(), Some(token_a.as_str()));
    assert_eq!(payload_a.external_user_id.as_deref(), Some("user_a"));
    assert_eq!(payload_a.status.as_deref(), Some(row_a.status.as_str()));

    assert_eq!(payload_b.provider, PROVIDER);
    assert_eq!(payload_b.purchase_token.as_deref(), Some(token_b.as_str()));
    assert_eq!(payload_b.external_user_id.as_deref(), Some("user_b"));
    assert_eq!(payload_b.status.as_deref(), Some(row_b.status.as_str()));

    let webhook_no_token = insert_webhook_provider_no_purchase_token(pool, app_id, "evt_rebuild_no_token").await?;
    let payload_missing = build_canonical_payload(database, webhook_no_token, app_id).await?;
    match payload_missing {
        None => (),
        Some(payload) => {
            assert_ne!(payload.purchase_token.as_deref(), Some(token_a.as_str()), "missing-token rebuild must not borrow same-SKU row's purchase_token");
            assert_ne!(payload.status.as_deref(), Some("active"), "missing-token rebuild must not borrow same-SKU row's status");
            assert_ne!(payload.external_user_id.as_deref(), Some("user_a"), "missing-token rebuild must not borrow same-SKU row's external_user_id");
        }
    }

    Ok(())
}

struct SubscriptionRow {
    id: Uuid,
    version: i32,
    status: String,
}

struct LoadState {
    status: String,
    version: i32,
    last_event_time: i64,
}

async fn load_subscription_state(pool: &PgPool, id: Uuid) -> Result<LoadState, sqlx::Error> {
    let row: (String, i32, i64) = sqlx::query_as(
        "SELECT status, version, last_event_time FROM pay.subscriptions WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(LoadState { status: row.0, version: row.1, last_event_time: row.2 })
}

async fn load_delivery(pool: &PgPool, id: Uuid) -> Result<crate::db::webhooks::WebhookDelivery, sqlx::Error> {
    let row: crate::db::webhooks::WebhookDelivery = sqlx::query_as(
        "SELECT * FROM pay.webhook_delivery WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

async fn claim_delivery(
    pool: &PgPool,
    app_id: Uuid,
    delivery_id: Uuid,
) -> Result<Uuid, Box<dyn Error>> {
    let delivery = db::webhooks::claim_webhook_delivery_by_id(
        pool,
        app_id,
        delivery_id,
        "identity-cross-user-test",
        WEBHOOK_DELIVERY_LEASE_SECS,
    )
    .await?
    .ok_or("expected delivery to be claimable")?;

    delivery
        .claim_token
        .ok_or_else(|| "expected claimed delivery to have a claim token".into())
}

async fn replace_delivery_claim(pool: &PgPool, delivery_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE pay.webhook_delivery
         SET claim_token = gen_random_uuid(),
             claimed_by = 'replacement-worker',
             claimed_until = NOW() + INTERVAL '10 minutes'
         WHERE id = $1",
    )
    .bind(delivery_id)
    .execute(pool)
    .await?;

    Ok(())
}

async fn load_webhook_provider_suppression(pool: &PgPool, id: Uuid) -> Result<(bool, Option<String>), sqlx::Error> {
    let row: (bool, Option<String>) = sqlx::query_as(
        "SELECT suppressed, suppressed_reason FROM pay.webhook_provider WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

async fn insert_google_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    purchase_token: &str,
    status: &str,
    last_event_time: i64,
) -> Result<SubscriptionRow, sqlx::Error> {
    let id = Uuid::new_v4();
    let row: (Uuid, i32, String) = sqlx::query_as(
        "INSERT INTO pay.subscriptions
            (id, app_id, external_user_id, subscription_id, provider, purchase_token, status, last_event_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, version, status",
    )
    .bind(id)
    .bind(app_id)
    .bind(external_user_id)
    .bind(SUBSCRIPTION_ID)
    .bind(PROVIDER)
    .bind(purchase_token)
    .bind(status)
    .bind(last_event_time)
    .fetch_one(pool)
    .await?;

    Ok(SubscriptionRow { id: row.0, version: row.1, status: row.2 })
}

async fn insert_webhook_provider(
    pool: &PgPool,
    app_id: Uuid,
    purchase_token: &str,
    provider_webhook_id: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let payload = serde_json::json!({
        "subscriptionNotification": {
            "subscriptionId": SUBSCRIPTION_ID,
            "purchaseToken": purchase_token,
            "autoRenewing": true,
            "expiryTimeMillis": T_EVT
        }
    });

    sqlx::query(
        "INSERT INTO pay.webhook_provider
            (id, app_id, provider, provider_webhook_id, event_type, subscription_id, purchase_token, payload, processed, timestamp_epoch_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, true, $9)",
    )
    .bind(id)
    .bind(app_id)
    .bind(PROVIDER)
    .bind(provider_webhook_id)
    .bind("SUBSCRIPTION_CANCELED")
    .bind(SUBSCRIPTION_ID)
    .bind(purchase_token)
    .bind(payload)
    .bind(T_EVT)
    .execute(pool)
    .await?;

    Ok(id)
}

async fn insert_webhook_delivery(pool: &PgPool, app_id: Uuid, webhook_provider_id: Uuid) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pay.webhook_delivery
            (id, app_id, webhook_provider_id, forward_attempts, forwarded, dead_lettered, next_attempt_at)
         VALUES ($1, $2, $3, 0, false, false, NOW())",
    )
    .bind(id)
    .bind(app_id)
    .bind(webhook_provider_id)
    .execute(pool)
    .await?;

    Ok(id)
}

async fn insert_webhook_provider_no_purchase_token(
    pool: &PgPool,
    app_id: Uuid,
    provider_webhook_id: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let payload = serde_json::json!({
        "subscriptionNotification": {
            "subscriptionId": SUBSCRIPTION_ID,
            "autoRenewing": true,
            "expiryTimeMillis": T_EVT
        }
    });

    sqlx::query(
        "INSERT INTO pay.webhook_provider
            (id, app_id, provider, provider_webhook_id, event_type, subscription_id, purchase_token, payload, processed, timestamp_epoch_ms)
         VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, true, $8)",
    )
    .bind(id)
    .bind(app_id)
    .bind(PROVIDER)
    .bind(provider_webhook_id)
    .bind("SUBSCRIPTION_CANCELED")
    .bind(SUBSCRIPTION_ID)
    .bind(payload)
    .bind(T_EVT)
    .execute(pool)
    .await?;

    Ok(id)
}

fn google_canonical_payload(purchase_token: &str, timestamp_epoch_ms: i64, event_type: &str) -> CanonicalWebhookPayload {
    let now = chrono::Utc::now();
    CanonicalWebhookPayload {
        event_id: format!("test-event-{}", Uuid::new_v4()),
        event_type: event_type.to_string(),
        timestamp: now.to_rfc3339(),
        timestamp_epoch_ms,
        app_slug: "identity-cross-user-regression".to_string(),
        product_id: Some(SUBSCRIPTION_ID.to_string()),
        subscription_id: Some(SUBSCRIPTION_ID.to_string()),
        external_user_id: None,
        amount_cents: None,
        new_price_cents: None,
        auto_renewing: Some(true),
        purchase_token: Some(purchase_token.to_string()),
        current_period_end: None,
        status: None,
        provider: PROVIDER.to_string(),
        provider_event_id: format!("evt_{}", Uuid::new_v4()),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: None,
        cancellation_mode: None,
        google_price_step_up_consent_deadline: None,
        google_pause_scheduled_at: None,
        google_deferred_until: None,
        google_pending_price_change_new_price_cents: None,
        google_pending_price_change_currency: None,
        google_pending_price_change_mode: None,
        google_pending_price_change_state: None,
        google_pending_price_change_expected_at: None,
    }
}

async fn test_database() -> Result<crate::db::Database, Box<dyn Error>> {
    dotenvy::dotenv().ok();
    let admin_database_url = std::env::var("ADMIN_DATABASE_URL").ok();
    let environment = std::env::var("ENVIRONMENT").unwrap_or_default().to_ascii_lowercase();
    let is_production = matches!(environment.as_str(), "production" | "prod");
    let database_url = match std::env::var("BRIDGE_TEST_DATABASE_URL") {
        Ok(url) => url,
        Err(_) if !is_production => admin_database_url
            .clone()
            .or_else(|| std::env::var("DATABASE_URL").ok())
            .ok_or("set BRIDGE_TEST_DATABASE_URL or a non-production DATABASE_URL for Google identity regression tests")?,
        Err(_) => return Err("BRIDGE_TEST_DATABASE_URL must be set in production-like environments".into()),
    };

    Ok(crate::db::Database::new(&database_url, admin_database_url.as_deref()).await?)
}

async fn insert_test_app(pool: &PgPool, label_suffix: &str) -> Result<Uuid, sqlx::Error> {
    insert_test_app_with_url(pool, label_suffix, "http://127.0.0.1:9/unused").await
}

async fn insert_test_app_with_url(pool: &PgPool, label_suffix: &str, callback_url: &str) -> Result<Uuid, sqlx::Error> {
    let app_id = Uuid::new_v4();
    let slug = format!("identity-cross-user-{label_suffix}-{app_id}");

    sqlx::query(
        "INSERT INTO pay.apps (id, slug, display_name, webhook_callback_url, webhook_callback_secret)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(app_id)
    .bind(slug)
    .bind("Identity Cross-User Regression")
    .bind(callback_url)
    .bind("test_callback_secret")
    .execute(pool)
    .await?;

    Ok(app_id)
}

async fn cleanup_test_app(pool: &PgPool, app_id: Uuid) {
    let _ = sqlx::query("DELETE FROM pay.webhook_delivery WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM pay.webhook_provider WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM pay.subscriptions WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM pay.apps WHERE id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
}

async fn callback_handler(State(count): State<Arc<AtomicUsize>>) -> axum::http::StatusCode {
    count.fetch_add(1, Ordering::SeqCst);
    axum::http::StatusCode::OK
}

async fn spawn_callback_server() -> Result<(String, Arc<AtomicUsize>, JoinHandle<()>), Box<dyn Error>> {
    let count = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/callback", post(callback_handler))
        .with_state(count.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("test callback server should stay healthy");
    });

    Ok((format!("http://{}/callback", address), count, server))
}
