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
│  └─ Returns: 204 No Content (if successful ingestion)                        │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ (Async) tokio::spawn() 
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
│     ├─ Trigger Lifecycle Emails (paused, resumed, refunded, etc.)           │
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
Return 204 No Content (immediately, for provider acknowledgement)
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
    └─ (TODO) Queue webhook for immediate retry
       ├─ Reset forward_attempts to 0
       ├─ Set forwarded = false
       └─ Trigger forward_webhook() again
```

---

## Summary

**Webhook Pipeline**: Provider → Ingress → Processor → Delivery → Forwarder → App

**Deduplication**: Unique constraint on (app_id, provider, provider_webhook_id)

**Event Ordering**: Compare timestamp_epoch_ms < subscription.last_event_time

**Forwarding**: HMAC-SHA256 signed, 3-strike retry, exponential backoff

**Admin**: HTML dashboard with real-time API queries to DB
