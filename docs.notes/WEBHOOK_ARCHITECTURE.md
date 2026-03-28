# Bridge Webhook Architecture

## System Flow Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         PAYMENT PROVIDERS                                    │
│  (Google Play, Creem, LemonSqueezy, Coinbase)                               │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ POST /webhooks/{token}/{provider}
                  │ (with signature: HMAC or JWT)
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    BRIDGE WEBHOOK INGRESS                                    │
│                                                                               │
│  src/webhooks/ingress.rs                                                     │
│  ├─ handle_google_play()      [Extract token, verify JWT signature]         │
│  ├─ handle_creem()             [Extract token, verify HMAC signature]       │
│  ├─ handle_lemonsqueezy()      [Extract token, verify signature]            │
│  └─ handle_coinbase()          [Extract token, verify signature]            │
│                                                                               │
│  Returns: 200 OK (always, to prevent provider retries)                      │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ Incoming webhook data
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                   DATABASE: WEBHOOK PROVIDER TABLE                           │
│                  (Deduplication & Audit Trail)                              │
│                                                                               │
│  webhook_provider {                                                          │
│    id UUID,                        ← unique webhook identifier              │
│    app_id UUID,                    ← which app this is for                  │
│    provider TEXT,                  ← 'google_play', 'creem', etc            │
│    provider_webhook_id TEXT,       ← provider's event ID (dedup key)        │
│    event_type TEXT,               ← provider-specific event type            │
│    subscription_id, purchase_token,← context                                │
│    payload JSONB,                  ← full raw webhook payload               │
│    timestamp_epoch_ms BIGINT,      ← event timestamp (for ordering)         │
│    suppressed BOOLEAN,             ← marks stale/old events                 │
│    suppressed_reason TEXT,         ← why suppressed                         │
│    created_at TIMESTAMPTZ          ← ingress time                           │
│  }                                                                            │
│                                                                               │
│  UNIQUE (app_id, provider, provider_webhook_id)  ← Dedup key               │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ If NEW webhook
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                   WEBHOOK PROCESSOR                                          │
│                                                                               │
│  src/webhooks/processor.rs                                                   │
│  └─ process_webhook()                                                        │
│     ├─ Check if already suppressed → SKIP                                   │
│     ├─ Load subscription from DB                                             │
│     ├─ Compare event.timestamp < subscription.last_event_time?              │
│     │  └─ YES: suppress as STALE → SKIP                                     │
│     │  └─ NO: continue to forwarding                                        │
│     ├─ Normalize event type (provider-specific → canonical)                 │
│     │  │  Google Play:    SUBSCRIPTION_PURCHASED → subscription.renewed    │
│     │  │  Creem:          subscription.created → subscription.renewed       │
│     │  │  LemonSqueezy:   subscription_created → subscription.renewed       │
│     │  │  Coinbase:       charge:confirmed → payment.succeeded              │
│     │  └─ etc.                                                               │
│     └─ Create canonical webhook payload                                      │
│        {                                                                      │
│          event_id: "creem-evt_123456",                                      │
│          event_type: "subscription.renewed",                                │
│          timestamp: 1711270000000,                                           │
│          app_id: "uuid",                                                     │
│          subscription_id: "sub_123",                                        │
│          provider: "creem",                                                  │
│          provider_event_id: "evt_123456"                                    │
│        }                                                                      │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ Canonical payload
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                 DATABASE: WEBHOOK DELIVERY TABLE                             │
│               (Task Queue for App Callbacks)                                │
│                                                                               │
│  webhook_delivery {                                                          │
│    id UUID,                        ← delivery task ID                        │
│    app_id UUID,                    ← which app to send to                   │
│    webhook_provider_id UUID,       ← link to provider webhook               │
│    forward_attempts INT,           ← 0-3 (auto-fail after 3)                │
│    forwarded BOOLEAN,              ← true if successful                     │
│    forwarded_at TIMESTAMPTZ,       ← when delivered                         │
│    last_http_status INT,           ← HTTP response code                     │
│    last_error TEXT,                ← error message if failed                │
│    created_at TIMESTAMPTZ          ← task creation time                     │
│  }                                                                            │
│                                                                               │
│  INDEX: (app_id, forwarded) WHERE forwarded = false  ← pending tasks       │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ Delivery task
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                   WEBHOOK FORWARDER                                          │
│          (Background Job / Async Task Processor)                            │
│                                                                               │
│  src/webhooks/forwarding.rs                                                  │
│  └─ forward_webhook()                                                        │
│     ├─ Load app.webhook_callback_url from DB                                │
│     ├─ Load app.webhook_callback_secret from DB                             │
│     ├─ Serialize canonical payload to JSON                                  │
│     ├─ Generate HMAC-SHA256 signature                                       │
│     │  └─ message = "{payload}.{timestamp}"                                │
│     │  └─ signature = "sha256=" + hex(HMAC(secret, message))                │
│     ├─ POST payload to app with headers:                                    │
│     │  ├─ X-Pay-Signature: sha256=...                                       │
│     │  ├─ X-Pay-Timestamp: 1711270000000                                    │
│     │  ├─ X-Pay-Event-Id: creem-evt_123456                                 │
│     │  └─ Content-Type: application/json                                    │
│     ├─ Handle response:                                                      │
│     │  ├─ 2xx: Mark as forwarded=true in DB                                │
│     │  └─ 3xx/4xx/5xx: Retry (if attempts < 3)                             │
│     └─ Retry logic:                                                          │
│        ├─ Attempt 0: immediately                                            │
│        ├─ Attempt 1: wait 5 minutes, retry                                  │
│        ├─ Attempt 2: wait 10 minutes, retry                                 │
│        └─ Attempt 3: dead-letter (give up)                                 │
└─────────────────┬───────────────────────────────────────────────────────────┘
                  │ Forward status
                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          CLIENT APP                                          │
│                  (e.g., HiHa, other Bridge consumers)                       │
│                                                                               │
│  Receives webhook at: app.webhook_callback_url                              │
│  {                                                                            │
│    "event_id": "creem-evt_123456",                                          │
│    "event_type": "subscription.renewed",                                    │
│    "timestamp": 1711270000000,                                               │
│    "app_id": "hiha-app-uuid",                                               │
│    "subscription_id": "sub_123",                                            │
│    ...                                                                       │
│  }                                                                            │
│                                                                               │
│  Headers:                                                                    │
│  ├─ X-Pay-Signature: sha256=abc123...                                       │
│  ├─ X-Pay-Timestamp: 1711270000000                                          │
│  └─ X-Pay-Event-Id: creem-evt_123456                                        │
│                                                                               │
│  App verifies signature: HMAC(secret, payload.timestamp) == signature       │
│  App updates user subscription status based on event_type                   │
└─────────────────────────────────────────────────────────────────────────────┘
                  
                  │
                  ├─ (Optional) Retry on 5xx
                  │
                  └─ Update subscription.last_event_time in app DB
```

## Admin Dashboard Flow

```
┌──────────────────────────────────────────────────────────┐
│              ADMIN USER (Browser)                         │
└──────────────┬───────────────────────────────────────────┘
               │ GET /admin
               ▼
┌──────────────────────────────────────────────────────────┐
│        Admin Dashboard Handler                           │
│  src/handlers/admin.rs::admin_dashboard()               │
│  Returns: templates/admin.html (static HTML + JS)       │
└──────────────┬───────────────────────────────────────────┘
               │ HTML + embedded JavaScript
               ▼
┌──────────────────────────────────────────────────────────┐
│         Admin Dashboard UI                               │
│      (Bootstrap 5 + Fetch API)                          │
│                                                          │
│  ┌─────────────────────────────────────────────────┐   │
│  │ APPS TABLE                                       │   │
│  ├─────────────────────────────────────────────────┤   │
│  │ App Name │ Slug │ URL │ Failed Webhooks │ Action│   │
│  │ ─────────┼──────┼─────┼─────────────────┼───────│   │
│  │ HiHa     │ hiha │ ... │ 2 ❌ (badge)    │ View  │   │
│  └─────────────────────────────────────────────────┘   │
│            │ Click "View Webhooks"                      │
│            ▼                                             │
│  ┌─────────────────────────────────────────────────┐   │
│  │ RECENT WEBHOOKS FOR APP                          │   │
│  ├─────────────────────────────────────────────────┤   │
│  │ Event │ Provider │ Type │ Status │ Att │ Action │   │
│  │ ─────┼──────────┼──────┼────────┼─────┼────────│   │
│  │ evt.. │ creem    │ subs │ ✅ Del │ 1   │        │   │
│  │ evt.. │ creem    │ subs │ ⏳ Pend│ 2   │ Retry  │   │
│  └─────────────────────────────────────────────────┘   │
│            │ Click "Retry"                              │
│            ▼                                             │
│        POST /admin/webhooks/:id/retry                   │
└──────────────┬───────────────────────────────────────────┘
               │ Async fetch calls
               ▼
┌──────────────────────────────────────────────────────────┐
│           Admin API Endpoints                            │
│                                                          │
│  GET /admin/apps                                        │
│  └─ list_apps() → JSON array                           │
│     [                                                    │
│       {                                                  │
│         "id": "uuid",                                   │
│         "slug": "hiha",                                 │
│         "display_name": "HiHa",                         │
│         "failed_webhooks": 2                            │
│       },                                                 │
│       ...                                                │
│     ]                                                    │
│                                                          │
│  GET /admin/apps/:app_id/webhooks                       │
│  └─ get_app_webhooks() → JSON array                    │
│     [                                                    │
│       {                                                  │
│         "id": "uuid",                                   │
│         "provider_webhook_id": "evt_123",              │
│         "event_type": "subscription.renewed",          │
│         "provider": "creem",                            │
│         "forwarded": true,                              │
│         "forward_attempts": 1,                          │
│         "created_at": "2026-03-23T21:00:00Z"           │
│       },                                                 │
│       ...                                                │
│     ]                                                    │
│                                                          │
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
Return 200 OK (immediately, for provider acknowledgement)
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
