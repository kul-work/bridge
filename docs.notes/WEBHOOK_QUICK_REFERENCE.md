# Bridge Webhook Implementation - Quick Reference

## Routes

### Webhook Ingress (Unprotected, Signature-Verified)

```
POST /webhooks/{ingress_token}/google_play
POST /webhooks/{ingress_token}/creem
POST /webhooks/{ingress_token}/lemonsqueezy
POST /webhooks/{ingress_token}/coinbase
```

**Response**: `200 OK` (always, to prevent provider retries)

**Flow**:
1. Parse `{ingress_token}` UUID → find app
2. Verify provider signature (TODO: implement per provider)
3. Call appropriate handler → `handle_google_play()`, etc.
4. Return `200 OK` immediately

### Admin API (Protected, Tyde Clerk Auth - TODO)

```
GET  /admin                           → HTML dashboard
GET  /admin/apps                      → JSON: apps list with failed webhook counts
GET  /admin/apps/:app_id/webhooks    → JSON: paginated webhooks for app
POST /admin/webhooks/:webhook_id/retry → Trigger manual retry
```

## Database Layer

All functions in `src/db/webhooks.rs`:

```rust
// Store incoming webhook (with dedup)
store_webhook_provider(
    pool, app_id, provider, provider_webhook_id, 
    event_type, subscription_id, purchase_token, 
    payload, timestamp_epoch_ms
) → Result<Uuid>

// Suppress stale webhook
suppress_webhook(pool, webhook_id, reason) → Result<()>

// Create delivery task
create_webhook_delivery(pool, app_id, webhook_provider_id) → Result<Uuid>

// Get webhook delivery record
get_webhook_delivery(pool, delivery_id) → Result<WebhookDelivery>

// Update delivery after attempt
update_webhook_delivery_attempt(
    pool, delivery_id, http_status, error, forwarded
) → Result<()>

// List pending webhooks (not forwarded, attempts < 3)
list_pending_webhooks(pool, limit) → Result<Vec<WebhookDelivery>>

// List app's recent webhooks (admin)
list_app_webhooks(pool, app_id, limit, offset) 
    → Result<Vec<(WebhookDelivery, WebhookProvider)>>

// Count failed deliveries
count_failed_webhooks(pool, app_id) → Result<i64>
```

## Webhook Processing

In `src/webhooks/processor.rs`:

```rust
// Main webhook processor
process_webhook(pool, webhook_provider_id, app_id) 
    → Result<Option<CanonicalWebhookPayload>>

// Returns None if:
// - Webhook already suppressed
// - Event is stale (timestamp < subscription.last_event_time)

// Returns Some(payload) with normalized fields:
pub struct CanonicalWebhookPayload {
    event_id: String,              // "provider-provider_webhook_id"
    event_type: String,            // "subscription.renewed", etc.
    timestamp: i64,                // epoch ms
    app_id: String,
    subscription_id: Option<String>,
    external_user_id: Option<String>,
    status: Option<String>,
    provider: String,
    provider_event_id: String,
}
```

### Event Type Normalization

All providers → canonical types:

| Provider | Event | → Canonical |
|----------|-------|-----------|
| Google Play | SUBSCRIPTION_PURCHASED | subscription.renewed |
| Google Play | SUBSCRIPTION_RENEWED | subscription.renewed |
| Google Play | SUBSCRIPTION_CANCELED | subscription.cancelled |
| Google Play | SUBSCRIPTION_EXPIRED | subscription.expired |
| Creem | subscription.created | subscription.renewed |
| Creem | subscription.renewed | subscription.renewed |
| Creem | subscription.cancelled | subscription.cancelled |
| LemonSqueezy | subscription_created | subscription.renewed |
| LemonSqueezy | subscription_updated | subscription.renewed |
| Coinbase | charge:confirmed | payment.succeeded |
| Coinbase | charge:failed | payment.failed |

## Webhook Forwarding

In `src/webhooks/forwarding.rs`:

```rust
forward_webhook(pool, app_id, webhook_delivery_id, payload)
    → Result<()>

// Process:
// 1. Load app callback_url + webhook_callback_secret
// 2. Serialize payload
// 3. Create HMAC-SHA256 signature
// 4. POST to app with headers:
//    - X-Pay-Signature: sha256={hex_digest}
//    - X-Pay-Timestamp: unix_ms
//    - X-Pay-Event-Id: {event_id}
// 5. Retry logic:
//    - Attempt 0: immediate
//    - Attempt 1: 5 min delay
//    - Attempt 2: 10 min delay
//    - Attempt 3+: dead-letter
// 6. Update webhook_delivery with status/error
```

### HMAC Signature

```rust
create_signature(payload, timestamp, secret) → Result<String>

// Format: sha256={hex_encoded_digest}
// Message: "{payload}.{timestamp}"
// Secret: app.webhook_callback_secret
```

## Admin Dashboard

`templates/admin.html` - Standalone HTML with embedded Bootstrap 5 + JavaScript

**Features**:
- Load apps list from `/admin/apps`
- Show failed webhook count per app
- Load webhooks for selected app from `/admin/apps/:id/webhooks`
- Display webhook status (Delivered / Pending)
- Show attempt count and last error
- Manual retry button (POST to `/admin/webhooks/:id/retry`)

## Implementation Checklist

- [x] Database schema (migrations 05, 06)
- [x] Database queries (src/db/webhooks.rs)
- [x] Webhook ingress handlers (src/webhooks/ingress.rs)
- [x] Webhook processing (src/webhooks/processor.rs)
- [x] Webhook forwarding (src/webhooks/forwarding.rs)
- [x] Admin handlers (src/handlers/admin.rs)
- [x] Admin UI (templates/admin.html)
- [x] Route registration (src/main.rs)
- [x] Compilation (cargo check/build pass)
- [ ] Provider signature verification (per provider)
- [ ] Background job queue (Tokio spawn or Redis)
- [ ] Webhook forwarding execution (background task)
- [ ] Admin route protection (Tyde Clerk)
- [ ] Subscription state updates (after forwarding)
- [ ] Monitoring & metrics

## Testing

Manual webhook ingress:

```bash
# Google Play webhook (requires valid JWT)
curl -X POST http://localhost:3000/webhooks/{token}/google_play \
  -H "Content-Type: application/json" \
  -d '{"message": {"data": "..."}}'

# Check admin dashboard
curl http://localhost:3000/admin
curl http://localhost:3000/admin/apps
curl http://localhost:3000/admin/apps/{app_id}/webhooks
```

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Invalid webhook token | 404 App not found |
| Signature verification failed | 400 Webhook verification failed |
| Duplicate webhook (dedup) | 200 OK (logged as duplicate) |
| Stale event | Suppressed, not forwarded |
| Webhook forwarding fails | Retry up to 3 times, then dead-letter |
| Admin app not found | 400 Validation error |

## Security

- ✅ Webhook ingress: Provider signature verification (TODO: implement)
- ✅ App callbacks: HMAC-SHA256 signing
- ✅ Admin routes: Should be protected by Tyde Clerk (TODO: implement)
- ✅ Rate limiting: API key rate limits (existing)
- ✅ Webhook dedup: Prevents duplicate processing
- ✅ Event ordering: Rejects stale events

---

**See**: `WEBHOOK_IMPLEMENTATION_SUMMARY.md` for full details
