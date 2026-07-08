# Bridge Webhook Architecture

## System Flow Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         PAYMENT PROVIDERS                                    │
│  (Google Play, Creem)                                                         │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ POST /webhooks/{token}/{provider}
                  │ (with signature verification)
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    BRIDGE WEBHOOK INGRESS                                    │
│                                                                               │
│  src/webhooks/ingress.rs                                                      │
│  ├─ handle_google_play()      [Extract token, verify PubSub signature]       │
│  ├─ handle_creem()             [Extract token, verify HMAC signature]        │
│  │  └─ Toggleable via `verify_webhook_signature` per app                     │
│  └─ Returns: 204 after durable provider insert + delivery enqueue            │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ durable insert + delivery enqueue
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                   DATABASE: WEBHOOK_PROVIDER TABLE                           │
│                  (Deduplication & Audit Trail)                               │
│                                                                               │
│  pay.webhook_provider {                                                      │
│    id UUID,                        ← internal unique identifier              │
│    app_id UUID,                    ← linked application                      │
│    provider TEXT,                  ← 'google_play', 'creem', etc             │
│    provider_webhook_id TEXT,       ← provider's event ID (primary dedup)     │
│    event_type TEXT,                ← provider-specific raw event type        │
│    subscription_id, purchase_token,← resolved context identifiers            │
│    payload JSONB,                  ← full raw webhook payload                │
│    timestamp_epoch_ms BIGINT,      ← provider event time (for ordering)      │
│    processed BOOLEAN,              ← mark as completion                      │
│    suppressed BOOLEAN,             ← marks stale/superseded events           │
│    created_at TIMESTAMPTZ          ← ingestion time                          │
│  }                                                                            │
│                                                                               │
│  UNIQUE (app_id, provider, provider_webhook_id)  ← Delivery dedup            │
│  Never dedupe renewable events by purchase_token + event_type                │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ If NEW webhook
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                   WEBHOOK PROCESSOR                                          │
│                                                                               │
│  src/webhooks/processor.rs                                                   │
│  └─ process_webhook()                                                        │
│     ├─ Resolution Cascade: Resolve external_user_id (Strategies 1-6)          │
│     ├─ Load subscription state                                               │
│     ├─ Stale Guard: Compare event.ts < subscription.last_event_time?         │
│     │  └─ YES: suppress as "stale" → SKIP                                    │
│     ├─ Normalization: Map provider raw status → Canonical types              │
│     ├─ Collect post-commit email effects (paused, resumed, refunded, etc.)  │
│     └─ Create Canonical Payload (serializable for apps)                      │
│        {                                                                     │
│          event_id: "google_play-msg_123",                                    │
│          event_type: "subscription.activated",                               │
│          timestamp: "2024-03-24T12:00:00Z",                                  │
│          app_slug: "hiha",                                                   │
│          subscription_id: "abc_123",                                         │
│          external_user_id: "user_456",                                       │
│          status: "active",                                                   │
│          ... (see CanonicalWebhookPayload struct)                            │
│        }                                                                     │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ Canonical payload
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                 DATABASE: WEBHOOK_DELIVERY TABLE                             │
│               (Reliable Forwarding Task Queue)                               │
│                                                                               │
│  pay.webhook_delivery {                                                      │
│    id UUID,                        ← delivery attempt task ID                │
│    app_id UUID,                    ← target application                      │
│    webhook_provider_id UUID,       ← link to source provider webhook         │
│    forward_attempts INT,           ← loop counter (0-3 retries)              │
│    forwarded BOOLEAN,              ← delivery success flag                   │
│    dead_lettered BOOLEAN,          ← failed permanently                     │
│    last_http_status INT,           ← remote app response code                │
│    created_at TIMESTAMPTZ                                                    │
│  }                                                                            │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ Delivery task
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                   WEBHOOK FORWARDER                                          │
│                                                                               │
│  src/webhooks/forwarding.rs                                                   │
│  └─ forward_webhook()                                                        │
│     ├─ Pre-flight Guard: Check if superseded_before_forward                  │
│     ├─ Load app.webhook_callback_url + secret                                │
│     ├─ Generate Signature: HMAC-SHA256(secret, payload_json)                 │
│     ├─ POST to App Callback with Headers:                                    │
│     │  ├─ X-Pay-Signature: sha256={hex}                                      │
│     │  ├─ X-Pay-Timestamp: {unix_ts}                                         │
│     │  └─ X-Pay-Event-Id: {provider-event_id}                                │
│     └─ Update delivery record (Success or Retry)                             │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ HTTP Request
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          CLIENT APP                                          │
│                  (Verified Gateway Callback)                                 │
│                                                                               │
│  Endpoint: app.webhook_callback_url                                          │
│  1. Receives signed JSON payload                                             │
│  2. Verifies HMAC-SHA256 signature using shared secret                       │
│  3. Performs business logic (activate premium, etc.)                         │
│  4. Returns 200/204 to acknowledge receipt                                   │
└─────────────────────────────────────────────────────────────────────────────┘
│  POST /admin/webhooks/:webhook_id/retry                │
│  └─ retry_webhook() → 200 OK                           │
└──────────────┬───────────────────────────────────────────┘
               │ Queries
               ▼
┌──────────────────────────────────────────────────────────┐
│          Database Queries                               │
│                                                          │
│  SELECT * FROM pay.apps                                 │
│  SELECT COUNT(*) FROM webhook_delivery                  │
│    WHERE app_id = ? AND forwarded = false               │
│                                                          │
│  SELECT * FROM webhook_delivery                         │
│    WHERE app_id = ? ORDER BY created_at DESC            │
│                                                          │
│  SELECT * FROM webhook_provider WHERE id = ?            │
└──────────────────────────────────────────────────────────┘
```

## Event Suppression Logic

```
INCOMING WEBHOOK
    │
    ├─ Check: Does webhook_provider with same ID exist?
    │  └─ YES: Return existing ID (dedup)
    │  └─ NO: Continue
    │
    ├─ Store in webhook_provider table
    │
    └─ Call process_webhook()
       │
       ├─ Check: webhook.suppressed == true?
       │  └─ YES: SKIP (already marked stale)
       │  └─ NO: Continue
       │
       ├─ Load subscription by subscription_id
       │
       ├─ Compare timestamps:
       │  webhook.timestamp_epoch_ms < subscription.last_event_time?
       │
       │  YES ───────────────────────┐
       │  │                           │
       │  ▼                           │
       │  Mark webhook as suppressed  │
       │  suppressed_reason = "stale" │
       │  Return None (SKIP)          │
       │                              │
       │  NO ───────────────────────┐│
       │  │                          ││
       │  ▼                          ││
       │  Process webhook ◄──────────┘│
       │  ├─ Normalize event type     │
       │  ├─ Create delivery task     │
       │  └─ Return Some(payload)     │
       │                              │
       └──────────────────────────────┘
```

## Key Data Flows

### 1. INGRESS (Provider → Bridge)

Assumptions at the ingress boundary:

- Provider webhook URLs are `/webhooks/{webhook_ingress_token}/{provider}`. Bridge parses `{webhook_ingress_token}` as a UUID and uses it only to resolve the target app. Invalid or unknown tokens return `404` and are not processed.
- The ingress token is not considered a provider-authentication signature. Provider signature verification must run before payload parsing, persistence, or state mutation. `verify_webhook_signature=false` is for local/mock testing only and should not be used in production.
- Google Play webhook verification uses the Pub/Sub `Authorization` JWT. Production must enable audience checking with `GOOGLE_VERIFY_AUDIENCE=true` and set `GOOGLE_PUB_SUB_AUDIENCE` to the exact webhook endpoint. Non-mock mode also verifies that the JWT `email` matches the configured Pub/Sub push service account (`pub_sub_service_account_email` or `GOOGLE_PUB_SUB_SERVICE_ACCOUNT_EMAIL`) and that `email_verified` is true; disabling `verify_pubsub_identity` is honored only for local/mock testing.
- Creem webhook verification uses HMAC-SHA256 over the raw request body and the configured `webhook_secret`; the supplied signature must match before processing. Accepted signature headers are `creem-signature`, `Webhook-Signature`, and `x-signature`.
- `X-Webhook-Verification-Mode` and other test override headers are honored only when `MOCK_EXTERNAL_APIS=true`.

```
Provider webhook
    ↓
Extract {token} from URL
    ↓
Find App by webhook_ingress_token = token
    ↓
Verify Provider Signature (per provider type)
    ↓
Store in webhook_provider (dedup on provider_webhook_id)
    ↓
Process Webhook (dedup + ordering + normalization)
    ↓
Create webhook_delivery task
    ↓
Return 204 No Content after durable enqueue
```

### 2. FORWARDING (Bridge → App)

```
pending webhook_delivery (forwarded = false, attempts < 3)
    ↓
Load app.webhook_callback_url + webhook_callback_secret
    ↓
Load webhook_provider (for canonical payload)
    ↓
Generate HMAC-SHA256(secret, payload + timestamp)
    ↓
POST to app with:
  - Body: canonical payload JSON
  - X-Pay-Signature: sha256={hex}
  - X-Pay-Timestamp: unix_ms
  - X-Pay-Event-Id: provider-event_id
    ↓
Handle Response:
  ├─ 2xx: forwarded=true, forwarded_at=NOW()
  │
  └─ 4xx/5xx or timeout:
      ├─ forward_attempts++
      ├─ Schedule retry (5min or 10min delay)
      │
      └─ If forward_attempts >= 3:
          └─ Dead-letter (stop retrying)
```

### 3. ADMIN MONITORING (Admin → Bridge DB)

```
Admin loads /admin
    ↓
GET /admin/apps
    ├─ Query: SELECT * FROM apps
    ├─ For each app:
    │  └─ count_failed_webhooks()
    │     └─ SELECT COUNT(*) FROM webhook_delivery
    │        WHERE app_id = ? AND (forwarded=false OR status >= 400)
    │
    └─ Render: Apps table with failed counts

Admin clicks "View Webhooks"
    ↓
GET /admin/apps/:app_id/webhooks
    ├─ Query: SELECT * FROM webhook_delivery WHERE app_id = ?
    ├─ For each delivery:
    │  └─ Load corresponding webhook_provider
    │
    └─ Render: Webhooks table with status/attempts

Admin clicks "Retry"
    ↓
POST /admin/webhooks/:webhook_id/retry
    └─ Atomically reset only an unforwarded dead-lettered delivery
       ├─ forward_attempts = 0
       ├─ dead_lettered = false
       ├─ forwarded/forwarded_at are not cleared
       └─ Background worker picks up eligible reset rows on next tick & forwards
          (suppressed webhooks are marked complete by the worker)
```

---

## Summary

**Webhook Pipeline**: Provider → Ingress → Processor → Delivery → Forwarder → App

Lifecycle email and dispute admin alert side effects are collected as post-commit effects during processing. Bridge commits subscription/payment state, the stored canonical payload, and `webhook_provider.processed=true` before it schedules email lookup or provider email sends. Without a durable email outbox, those notifications are best-effort: a process crash after commit but before effect execution can drop the email, but email failures do not roll back payment or subscription state.

**Deduplication**: Unique constraint on (app_id, provider, provider_webhook_id)

**Event Ordering**: Compare timestamp_epoch_ms < subscription.last_event_time

**Forwarding**: HMAC-SHA256 signed, 3-strike retry, exponential backoff

**Admin**: HTML dashboard with real-time API queries to DB
