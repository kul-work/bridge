# Consolidated Common Gaps & Implementation Strategies

**Summary**: This document distills the three prior audits into five common gaps affecting Bridge and HiHa, with implementation strategies for each.

**Reconfirmation note (2026-05-07)**:
- Gap 1 is stale as a HiHa-breaking bug. Current HiHa calls Bridge's dedicated `/api/v1/users/{external_user_id}/subscription-status` snapshot endpoint and deserializes the full snapshot shape. The remaining confirmed Bridge issue is narrower: `GET /api/v1/subscriptions` still returns a wrapped list with thin list items, so consumers of that list endpoint do not receive the full lifecycle fields.
- Gap 2 is partially stale. Bridge now calls lifecycle emails for `payment.failed`, `subscription.price_step_up`, and `subscription.deferred`, and resolves emails through an app callback lookup rather than a Bridge `email_contacts` table.
- Gap 5 is partially stale. Bridge already exposes subscription acknowledge and price-step-up accept/decline routes, and some lifecycle email dispatch is wired. Missing Bridge email coverage remains for `subscription.paused`, `subscription.resumed`, and `payment.refunded`.

---

## Gap 1: Bridge List Schema Still Thin; HiHa Parsing Bug Is Stale

**Reconfirmed status (2026-05-07 code dive)**: Partially stale. The HiHa-breaking parsing mismatch is no longer present in current code. Bridge's raw list endpoint still has a thin list-item schema.

### Finding
The original finding said HiHa called Bridge `GET /api/v1/subscriptions` and expected a flat single-subscription object with UX fields like `google_requires_price_step_up_consent`, `google_pause_scheduled_at`, `payment_failure_notification`, etc. That is stale.

Current HiHa code calls Bridge's dedicated snapshot endpoint:
- Bridge route: `GET /api/v1/users/{external_user_id}/subscription-status`
- HiHa client: `src/services/bridge_client.rs`
- HiHa handler: `src/handlers/content.rs`

The Bridge snapshot response includes the lifecycle fields HiHa needs:
- `payment_failure_notification`
- `revoked_at`
- `revocation_reason`
- `google_requires_price_step_up_consent`
- `google_new_price_cents`
- `google_price_step_up_consent_deadline`
- `google_pause_scheduled_at`
- `google_deferred_until`
- `last_event_time`

HiHa deserializes this typed snapshot directly and forwards those fields in `SubscriptionStatusResponse`.

The remaining confirmed issue is limited to Bridge's raw list endpoint. Bridge `GET /api/v1/subscriptions` still returns:
- `subscriptions: Vec<SubscriptionDetail>`
- `pagination: PaginationMeta`

The list endpoint's `SubscriptionDetail` currently includes only:
- `id`
- `subscription_id`
- `provider`
- `status`
- `current_period_end`
- `auto_renewing`
- `payment_failure_notification`

The single-subscription endpoint uses `SubscriptionDetailFull` and does expose more Google and revocation fields, and the newer snapshot endpoint exposes the fields needed by HiHa. The raw list endpoint does not.

### Impact
- Current HiHa subscription-status UX is not blocked by this gap.
- Any consumer using Bridge `GET /api/v1/subscriptions` cannot get full lifecycle state from list results alone.
- Bridge API consistency is still weaker than ideal because list, detail, and snapshot endpoints expose different subscription field sets.

### Code Implementation Strategy

#### Step 1: Optional Bridge Cleanup - Expand SubscriptionDetail list response
**File**: `src/handlers/subscriptions.rs`

Only needed if the list endpoint is intended to expose full lifecycle status. In `ListSubscriptionsResponse`, either:
- **Option A**: Expand `SubscriptionDetail` to include the lifecycle fields already available from `SubscriptionDetailFull` and `SubscriptionStatusSnapshot`:
  ```rust
  #[derive(Serialize)]
  pub struct SubscriptionDetail {
      pub id: String,
      pub subscription_id: String,
      pub provider: String,
      pub status: String,
      pub current_period_end: Option<i64>,
      pub auto_renewing: bool,
      pub payment_failure_notification: bool,
      // Add these:
      pub google_requires_price_step_up_consent: Option<bool>,
      pub google_price_step_up_consent_deadline: Option<i64>,
      pub google_new_price_cents: Option<i32>,
      pub google_pause_scheduled_at: Option<i64>,
      pub google_deferred_until: Option<i64>,
      pub revoked_at: Option<i64>,
      pub revocation_reason: Option<String>,
  }
  ```
- **Option B**: Introduce a `SubscriptionSummary` wrapper containing the intended list fields and return it from the list endpoint.

**Verify**: Run `cargo build` and confirm list endpoint schema includes new fields.

#### Step 2: No HiHa Parsing Fix Required For Current Flow
**Files checked**:
- `hiha/src/services/bridge_client.rs`
- `hiha/src/handlers/content.rs`
- `hiha/src/handlers/types.rs`

Current HiHa already consumes the snapshot endpoint and forwards the relevant lifecycle fields. Do not apply the old recommendation to parse `subscriptions[0]` unless HiHa intentionally switches back to Bridge's list endpoint.

**Verify**: Build both repos. For API behavior, mock or call Bridge `/api/v1/users/{external_user_id}/subscription-status` and confirm HiHa `/api/v1/subscription-status` forwards the lifecycle fields.

---

## Gap 2: Bridge Lifecycle Emails Partially Wired

**Reconfirmed status**: Partially stale. Bridge now sends some lifecycle emails, but not via the `email_contacts` storage strategy described below.

### Finding
Bridge contains `src/services/google_play/notifications.rs` with functions like `send_email_payment_failed`, `send_email_price_step_up`, `send_email_deferred`, etc.

This original finding is no longer fully accurate. Current Bridge code calls:
- `send_email_payment_failed` for `payment.failed`
- `send_email_price_step_up` for `subscription.price_step_up`
- `send_email_deferred` for `subscription.deferred`

Current email lookup is implemented in `src/services/email_lookup.rs`: Bridge calls the tenant app's callback origin at `/internal/bridge/email-lookup`, signs the request with `X-Pay-Signature`, and sends `{ "clerk_id": external_user_id }`. There is no Bridge-owned `email_contacts` table in the current repo.

### Remaining Impact
- Users may still miss lifecycle notifications if the tenant app does not implement the signed email lookup endpoint.
- Events without Bridge email dispatch (`subscription.paused`, `subscription.resumed`, `payment.refunded`) still lack direct user notification.
- Bridge owns email infrastructure, but coverage remains incomplete across all lifecycle events.

The remaining risk is narrower: emails depend on each app implementing the signed email lookup endpoint, and several lifecycle events are still not covered by Bridge email dispatch (see Gap 5).

### Code Implementation Strategy

#### Step 1: Bridge - Add tenant-scoped email resolution
**Status**: Superseded by current callback-based lookup unless a product decision is made to store email contacts in Bridge.

**File**: Create `src/services/email_contact.rs` or extend existing contact module

Implement a mapping layer to resolve `app_id + external_user_id -> email`:
```rust
pub async fn resolve_user_email(
    pool: &PgPool,
    app_id: &str,
    external_user_id: &str,
) -> Result<Option<String>> {
    // Query from a persisted mapping table (see Migration step)
    let email = sqlx::query_scalar!(
        "SELECT user_email FROM email_contacts 
         WHERE app_id = $1 AND external_user_id = $2",
        app_id,
        external_user_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(email)
}
```

Create migration:
```sql
CREATE TABLE IF NOT EXISTS email_contacts (
    id SERIAL PRIMARY KEY,
    app_id TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    user_email TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (app_id, external_user_id)
);
CREATE INDEX idx_email_contacts_app_user ON email_contacts(app_id, external_user_id);
```

**HiHa supplies emails**: When HiHa creates a user or subscription, it calls Bridge's new endpoint:
```rust
POST /api/v1/internal/email-contact
{ app_id, external_user_id, user_email }
```

**Verify**: Test email resolution; confirm mapping persists and retrieves correctly.

#### Step 2: Bridge - Wire notifications into event handlers
**File**: `src/webhooks/processor/event_handlers.rs`

In handlers for `payment.failed`, `subscription.price_step_up`, `subscription.deferred`:
```rust
async fn handle_payment_failed(
    state: &AppState,
    event: &CanonicalWebhookPayload,
) -> Result<Effects> {
    // ... existing DB mutation ...
    
    // NEW: Resolve and send email
    if let Some(email) = resolve_user_email(
        &state.db,
        &event.app_id,
        &event.external_user_id,
    )
    .await?
    {
        send_email_payment_failed(&state.email_client, &email, &event).await?;
    }
    
    Ok(effects)
}
```

**Prevent spam**: Ensure emails are sent **only once** per state transition. Use webhook idempotency key + state change detection:
```rust
// Only send if status *changed to* failure, not on retry of same failure
if previous_status != "payment_failure" && new_status == "payment_failure" {
    send_email_payment_failed(...).await?;
}
```

**Verify**: End-to-end test with mock email client; confirm emails fire only on state change.

#### Step 3: HiHa - Register email-contact endpoint
**File**: `src/main.rs` or `src/handlers/internal.rs`

Add protected route (auth token required):
```rust
#[post("/api/v1/internal/email-contact")]
async fn register_email_contact(
    State(state): State<AppState>,
    Json(payload): Json<EmailContactPayload>,
) -> Result<StatusCode> {
    // Validate app_id matches authenticated tenant
    // Call Bridge client to register
    state.bridge_client.register_email_contact(payload).await?;
    Ok(StatusCode::OK)
}
```

**Verify**: HiHa integration test; confirm emails registered successfully in Bridge.

---

**Reconfirmed status**: Not reconfirmed from this Bridge repo. Requires inspection of the HiHa repo.

### Finding
HiHa's `webhook_callbacks` table and handler only persist the event name and premium toggle. Fields like `new_price_cents`, `previous_status`, `google_deferred_until`, `google_pause_scheduled_at`, `revocation_reason`, and `reconciliation` metadata are dropped. HiHa cannot reconstruct old-style subscription cache behavior from callbacks alone.

### Impact
- Frontend cannot show granular lifecycle UX (deferred date, pause scheduled date, price step-up amount).
- HiHa cannot correct local state when `reconciliation.drift_detected` arrives.
- Premium fallback to Bridge polling is necessary, defeating split architecture intent.

### Code Implementation Strategy

#### Step 1: HiHa - Create subscription_cache table
**File**: Create migration `04_create_subscription_cache.sql`

```sql
CREATE TABLE IF NOT EXISTS subscription_cache (
    id SERIAL PRIMARY KEY,
    user_id UUID NOT NULL,
    subscription_id TEXT NOT NULL,
    app_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    status TEXT NOT NULL,
    
    -- Cache fields from callback
    current_period_end_epoch_ms BIGINT,
    auto_renewing BOOLEAN,
    payment_failure_notification BOOLEAN,
    revoked_at_epoch_ms BIGINT,
    revocation_reason TEXT,
    
    -- Google-specific lifecycle state
    google_requires_price_step_up_consent BOOLEAN,
    google_price_step_up_consent_deadline_epoch_ms BIGINT,
    google_new_price_cents INTEGER,
    google_pause_scheduled_at_epoch_ms BIGINT,
    google_deferred_until_epoch_ms BIGINT,
    
    -- Reconciliation metadata
    last_reconciliation_source TEXT,
    last_reconciliation_timestamp_epoch_ms BIGINT,
    last_reconciliation_previous_status TEXT,
    
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    
    FOREIGN KEY (user_id) REFERENCES users(id),
    UNIQUE(app_id, subscription_id)
);
CREATE INDEX idx_cache_user_app ON subscription_cache(user_id, app_id);
```

**Verify**: Run migration; confirm table exists with all columns.

#### Step 2: HiHa - Expand webhook_callbacks table
**File**: Extend migration `03_create_webhook_callbacks_table.sql`

Add columns:
```sql
ALTER TABLE webhook_callbacks ADD COLUMN IF NOT EXISTS (
    new_price_cents INTEGER,
    previous_status TEXT,
    corrected_status TEXT,
    reconciliation_source TEXT,
    revocation_reason TEXT,
    google_deferred_until_epoch_ms BIGINT,
    google_pause_scheduled_at_epoch_ms BIGINT,
    google_price_step_up_consent_deadline_epoch_ms BIGINT
);
```

**Verify**: Confirm columns added.

#### Step 3: HiHa - Expand BridgeWebhookPayload struct
**File**: `src/services/bridge_client.rs`

Add deserialization for all fields Bridge now sends:
```rust
#[derive(Deserialize)]
pub struct BridgeWebhookPayload {
    // ... existing ...
    pub new_price_cents: Option<i32>,
    pub previous_status: Option<String>,
    pub corrected_status: Option<String>,
    pub reconciliation_source: Option<String>,
    pub revocation_reason: Option<String>,
    pub google_deferred_until: Option<i64>,
    pub google_pause_scheduled_at: Option<i64>,
    pub google_price_step_up_consent_deadline: Option<i64>,
}
```

**Verify**: Parse mock Bridge payload; confirm all fields deserialize.

#### Step 4: HiHa - Enhance webhook handler to populate cache
**File**: `src/handlers/webhooks.rs`

Expand the callback handler to:
1. Persist all callback fields into `webhook_callbacks`.
2. Update `subscription_cache` based on event type.

```rust
async fn handle_webhook_callback(
    state: &AppState,
    payload: BridgeWebhookPayload,
) -> Result<()> {
    // 1. Log callback
    sqlx::query!(
        "INSERT INTO webhook_callbacks (..., new_price_cents, previous_status, ...)
         VALUES (...)",
        payload.new_price_cents,
        payload.previous_status,
        // ...
    )
    .execute(&state.db)
    .await?;
    
    // 2. Update premium
    update_premium_from_callback(&state.db, &payload).await?;
    
    // 3. UPDATE cache (new)
    match payload.event_type.as_str() {
        "subscription.price_step_up" => {
            sqlx::query!(
                "UPDATE subscription_cache 
                 SET google_requires_price_step_up_consent = true,
                     google_new_price_cents = $1,
                     google_price_step_up_consent_deadline_epoch_ms = $2
                 WHERE subscription_id = $3",
                payload.new_price_cents,
                payload.google_price_step_up_consent_deadline,
                payload.subscription_id,
            )
            .execute(&state.db)
            .await?;
        }
        "subscription.deferred" => {
            sqlx::query!(
                "UPDATE subscription_cache 
                 SET google_deferred_until_epoch_ms = $1
                 WHERE subscription_id = $2",
                payload.google_deferred_until,
                payload.subscription_id,
            )
            .execute(&state.db)
            .await?;
        }
        "subscription.pause_scheduled" => {
            sqlx::query!(
                "UPDATE subscription_cache 
                 SET google_pause_scheduled_at_epoch_ms = $1
                 WHERE subscription_id = $2",
                payload.google_pause_scheduled_at,
                payload.subscription_id,
            )
            .execute(&state.db)
            .await?;
        }
        "reconciliation.drift_detected" => {
            sqlx::query!(
                "UPDATE subscription_cache 
                 SET status = $1,
                     last_reconciliation_source = $2,
                     last_reconciliation_timestamp_epoch_ms = $3,
                     last_reconciliation_previous_status = $4
                 WHERE subscription_id = $5",
                payload.corrected_status,
                payload.reconciliation_source,
                chrono::Utc::now().timestamp_millis(),
                payload.previous_status,
                payload.subscription_id,
            )
            .execute(&state.db)
            .await?;
        }
        _ => {}
    }
    
    Ok(())
}
```

**Verify**: Integration test with sample payloads; confirm cache rows update correctly.

#### Step 5: HiHa - Switch subscription status to read local cache
**File**: `src/handlers/content.rs`

Replace Bridge list call with local cache read:
```rust
#[get("/api/v1/subscription-status")]
async fn get_subscription_status(
    user_id: UserId,
    State(state): State<AppState>,
) -> Result<SubscriptionStatusResponse> {
    // Read from local cache instead of Bridge
    let cache = sqlx::query_as::<_, SubscriptionCache>(
        "SELECT * FROM subscription_cache WHERE user_id = $1"
    )
    .fetch_optional(&state.db, user_id.0)
    .await?;
    
    if let Some(cache) = cache {
        return Ok(SubscriptionStatusResponse {
            is_premium: cache.status == "active" || cache.status == "trial",
            subscription_id: Some(cache.subscription_id),
            status: Some(cache.status),
            current_period_end: cache.current_period_end_epoch_ms,
            google_requires_price_step_up_consent: cache.google_requires_price_step_up_consent,
            google_new_price_cents: cache.google_new_price_cents,
            google_pause_scheduled_at: cache.google_pause_scheduled_at_epoch_ms,
            google_deferred_until: cache.google_deferred_until_epoch_ms,
            payment_failure_notification: cache.payment_failure_notification,
            revoked_at: cache.revoked_at_epoch_ms,
        });
    }
    
    Ok(SubscriptionStatusResponse::default())
}
```

**Verify**: Unit test with mock cache; confirm status response reads from local state.

---

**Reconfirmed status**: Confirmed.

### Finding
Bridge internally stores `google_price_step_up_consent_deadline`, `google_pause_scheduled_at`, and `google_deferred_until` in the subscription row, but these fields are **not included in the canonical callback payload** sent to HiHa. Without payload expansion, even a stronger HiHa callback handler cannot reconstruct full old-style UX state.

Current `CanonicalWebhookPayload` includes `new_price_cents`, `previous_status`, `corrected_status`, `reconciliation_source`, `revocation_reason`, and `cancellation_mode`, but still lacks:
- `google_price_step_up_consent_deadline`
- `google_pause_scheduled_at`
- `google_deferred_until`

Scheduler-built callbacks also omit these fields.

### Impact
- HiHa cannot show full deferred date, pause scheduled date, or price-step-up deadline in UX.
- Third-party systems receiving Bridge callbacks also lack complete lifecycle context.
- Callback payload is incomplete by design, not by ingestion miss.

### Code Implementation Strategy

#### Step 1: Bridge - Expand CanonicalWebhookPayload struct
**File**: `src/webhooks/models.rs` or equivalent models file

```rust
#[derive(Serialize, Deserialize)]
pub struct CanonicalWebhookPayload {
    // ... existing fields ...
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_price_cents: Option<i32>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_status: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corrected_status: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconciliation_source: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_reason: Option<String>,
    
    // NEW: Google lifecycle fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_price_step_up_consent_deadline: Option<i64>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_pause_scheduled_at: Option<i64>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_deferred_until: Option<i64>,
}
```

**Verify**: Compile; confirm no breaking changes to existing payload consumers.

#### Step 2: Bridge - Populate fields when building canonical payload
**File**: `src/webhooks/processor/event_handlers.rs`

When building `CanonicalWebhookPayload` from a subscription transition, fetch and include the Google fields:

```rust
async fn build_canonical_payload(
    pool: &PgPool,
    subscription_id: &str,
    event_type: &str,
) -> Result<CanonicalWebhookPayload> {
    let sub = sqlx::query_as::<_, Subscription>(
        "SELECT * FROM subscriptions WHERE id = $1"
    )
    .fetch_one(pool, subscription_id)
    .await?;
    
    CanonicalWebhookPayload {
        event_type: event_type.to_string(),
        subscription_id: subscription_id.to_string(),
        // ... other fields ...
        
        // Include Google fields if present
        google_price_step_up_consent_deadline: sub.google_price_step_up_consent_deadline,
        google_pause_scheduled_at: sub.google_pause_scheduled_at,
        google_deferred_until: sub.google_deferred_until,
        
        ..Default::default()
    }
}
```

**Verify**: Unit test with mock subscription; confirm payload includes Google fields when set.

#### Step 3: HiHa - Consume expanded fields
Payload expansion makes callback ingestion complete.

**Verify**: End-to-end test; confirm HiHa receives and stores all Google fields.

---

## Gap 5: Missing Lifecycle Email Consolidation for Granular Events

**Reconfirmed status**: Partially stale. Bridge has some email dispatch and action routes, but coverage is still incomplete.

### Finding
Six event types have partial or missing email behavior:
- `payment.failed`: DB mutation and Bridge email dispatch exist; Bridge subscription acknowledge route exists.
- `subscription.price_step_up`: Scheduler and Bridge email dispatch exist; Bridge accept/decline routes exist.
- `subscription.deferred`: DB mutation and Bridge email dispatch exist.
- `subscription.paused`: DB mutation exists; Bridge email dispatch was not found.
- `subscription.resumed`: DB mutation exists; Bridge email dispatch was not found.
- `payment.refunded` (OTP): Revoke handled; Bridge email dispatch was not found.

HiHa acknowledge/accept/decline UX routes were not reconfirmed from this repo.

### Impact
- Users unaware of payment issues, consent deadlines, deferrals, or cancellations.
- Old monolith's user communication coverage lost.
- Premium lifecycle lacks user-visible actions in both Bridge and HiHa.

### Code Implementation Strategy

#### Step 1: Bridge - Activate notifications for remaining lifecycle events
**File**: `src/webhooks/processor/event_handlers.rs`

For remaining uncovered event handlers, add conditional email dispatch after DB mutation. Current Bridge already dispatches `payment.failed`, `subscription.price_step_up`, and `subscription.deferred`.

```rust
// Example pattern
async fn handle_payment_failed(state: &AppState, event: &CanonicalWebhookPayload) -> Result<Effects> {
    // Existing DB mutation
    sqlx::query!("UPDATE subscriptions SET ... WHERE id = $1", &event.subscription_id)
        .execute(&state.db)
        .await?;
    
    // NEW: Send email if state changed to failure
    if let Some(email) = resolve_user_email(&state.db, &event.app_id, &event.external_user_id).await? {
        send_email_payment_failed(&state.email_client, &email, event).await?;
    }
    
    Ok(Effects { /* ... */ })
}

// Apply similar coverage for:
// - handle_subscription_paused
// - handle_subscription_resumed
// - handle_payment_refunded (OTP context)
```

Use webhook idempotency to suppress duplicates:
```rust
// Query webhook_log to detect state change
let previous_event = sqlx::query!(
    "SELECT status FROM webhook_log WHERE subscription_id = $1 AND event_type = $2 
     ORDER BY received_at DESC LIMIT 1",
    &event.subscription_id,
    "payment.failed"
)
.fetch_optional(&state.db)
.await?;

if previous_event.is_none() {
    // Only send email on first occurrence
    send_email_payment_failed(...).await?;
}
```

**Verify**: Test each handler; confirm email fires once per transition.

#### Step 2: HiHa - Add payment-failure acknowledge endpoint
**Status**: Not reconfirmed. Bridge already has `POST /api/v1/subscriptions/{subscription_id}/acknowledge`.

**File**: `src/handlers/webhooks.rs` or `src/handlers/payments.rs`

```rust
#[post("/api/v1/notifications/payment-failure/acknowledge")]
async fn acknowledge_payment_failure(
    user_id: UserId,
    State(state): State<AppState>,
) -> Result<StatusCode> {
    // Clear flag in local cache
    sqlx::query!(
        "UPDATE subscription_cache SET payment_failure_notification = false WHERE user_id = $1",
        user_id.0
    )
    .execute(&state.db)
    .await?;
    
    Ok(StatusCode::OK)
}
```

**Verify**: Test endpoint; confirm cache flag clears.

#### Step 3: HiHa - Add price-step-up accept/decline routes
**Status**: Not reconfirmed. Bridge already has `POST /api/v1/subscriptions/{subscription_id}/price-step-up/accept` and `/decline`.

**File**: `src/handlers/payments.rs`

```rust
#[post("/api/v1/subscriptions/{subscription_id}/price-step-up/accept")]
async fn accept_price_step_up(
    user_id: UserId,
    Path(subscription_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode> {
    // Call Bridge to confirm consent
    state.bridge_client.confirm_price_step_up(&subscription_id).await?;
    
    // Clear local cache flag
    sqlx::query!(
        "UPDATE subscription_cache 
         SET google_requires_price_step_up_consent = false 
         WHERE subscription_id = $1 AND user_id = $2",
        subscription_id,
        user_id.0
    )
    .execute(&state.db)
    .await?;
    
    Ok(StatusCode::OK)
}

#[post("/api/v1/subscriptions/{subscription_id}/price-step-up/decline")]
async fn decline_price_step_up(
    user_id: UserId,
    Path(subscription_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode> {
    // Call Bridge to cancel subscription due to declined consent
    state.bridge_client.cancel_subscription(&subscription_id, "declined_consent").await?;
    
    Ok(StatusCode::OK)
}
```

**Verify**: Integration test; confirm Bridge is called and local cache updates.

#### Step 4: Email templates & client setup
**File**: `src/services/email_client.rs` or extend `google_play/notifications.rs`

Ensure all email functions exist and take correct parameters:
```rust
pub async fn send_email_payment_failed(
    client: &EmailClient,
    recipient: &str,
    event: &CanonicalWebhookPayload,
) -> Result<()> {
    let subject = "Your payment failed";
    let body = format!("We couldn't process your payment for {}. ...", event.app_name);
    client.send(recipient, &subject, &body).await
}

pub async fn send_email_price_step_up(
    client: &EmailClient,
    recipient: &str,
    event: &CanonicalWebhookPayload,
) -> Result<()> {
    let new_price = event.new_price_cents.unwrap_or(0) as f64 / 100.0;
    let subject = "Subscription price update";
    let body = format!("Your subscription price is increasing to ${}. ...", new_price);
    client.send(recipient, &subject, &body).await
}

pub async fn send_email_deferred(
    client: &EmailClient,
    recipient: &str,
    event: &CanonicalWebhookPayload,
) -> Result<()> {
    let subject = "Your subscription renewal is deferred";
    let body = "Your subscription renewal has been postponed. ...";
    client.send(recipient, &subject, &body).await
}

// etc.
```

**Verify**: Unit test with mock client; confirm all templates are called with correct arguments.

---

## Summary Table

| Gap | Bridge | HiHa | Status |
|---|---|---|---|
| **1. API Mismatch / Thin List Schema** | Expand list response | Parse array, update handler | Confirmed Bridge issue |
| **2. Email Delivery** | Partially wired via callback email lookup | Ensure lookup endpoint exists | Partially stale |
| **3. Lifecycle Email Gaps** | Add remaining paused/resumed/refunded email coverage | Add/verify UX routes | Partially stale |

---

## Implementation Order

2. **Gap 1** (API schema / thin list schema) - Expand Bridge list response or update HiHa to consume the list/detail APIs correctly. This unblocks subscription status UX while cache work is in progress.
4. **Gap 5** (Remaining lifecycle emails) - Add Bridge email coverage for `subscription.paused`, `subscription.resumed`, and `payment.refunded`; verify HiHa UX routes.
5. **Gap 2** (Email contact policy) - Decide whether callback-based email lookup is sufficient or whether Bridge should own contact storage. This is no longer a prerequisite for Gap 5 unless product chooses Bridge-owned email contacts.

---

## Risk Mitigation

- **Backward compat**: All payload additions use `#[serde(skip_serializing_if = "Option::is_none")]`; existing consumers unaffected.
- **Email spam**: Use webhook idempotency to suppress duplicate sends.
- **Schema migration**: Separate DB migrations per table; can roll back independently.
- **Fallback**: HiHa can poll Bridge during cache bootstrap; eventually reads from local state only.
