# Bridge — Design Document

Architectural overview of Bridge payment gateway system, design decisions, and component interactions.

## 1. Core Purpose

Bridge (`pay.tydecode.com`) is a private payment processing microservice for Tyde. It acts as a centralized payment gateway decoupling payment logic from app business logic.

**Single responsibility**: Process, record, and relay payments. Never owns app users, products, or business logic.

---

## 2. Architectural Principles

### Decoupling
- Bridge processes payments, apps own users + business logic
- Bridge stores opaque identifiers (`external_user_id`, `product_id`)
- Apps pull subscription state via API or receive events via callbacks
- Email addresses are pass-through (never persisted in Bridge, but used for notifications)

### Idempotency First
- Every webhook logged before state mutation (deduplication via `webhook_log`)
- Payments UPSERTed atomically to prevent duplicates
- Subscriptions versioned for optimistic concurrency control
- Stale event suppression via high-water mark (`last_event_time`)
- Checkout idempotency via `checkout_idempotency` table
- Agent topup idempotency via unique index on charge_id

### Multi-App Design
- Bridge serves N apps (currently hiha.app, future apps)
- Provider credentials stored per-app in `provider_configs` table
- Webhook paths obfuscated per-app: `webhooks/{token}/:provider`
- API keys scoped per app

### Provider Abstraction
- Unified interface across Google Play (primary), Apple (checkout only)
- Creem, LemonSqueezy, Coinbase: Archived (not currently instantiated)
- State normalization (each provider's "active" maps to Bridge's `active`)
- Signature verification required on all provider webhooks
- Dynamic provider loading per-app (not startup-based env vars)

---

## 3. System Components

### 3.1 HTTP API Layer

**Authentication**: API key from `Authorization: Bearer sk_hiha_xxxxx` header
- Lookup by `key_prefix` (first 8 chars)
- Verify hash against `api_keys.key_hash`
- Check `api_keys.enabled` and `apps.enabled`
- Update `api_keys.last_used_at`

**Rate Limiting**: Per-API-key token bucket
- Default: 120 req/min per API key
- Endpoint-specific overrides via `apps.api_rate_limit_rules` (JSONB)
- Failed auth attempts: 10 req/min per IP

**Endpoints** (all scoped to authenticated `app_id`):
- `POST /api/v1/checkout` — create payment session
- `POST /api/v1/verify-purchase` — verify mobile purchase token
- `POST /api/v1/purchase/register` — pre-register purchase attempt
- `GET /api/v1/subscriptions` — list subscriptions
- `GET /api/v1/subscriptions/:subscription_id` — fetch subscription state
- `POST /api/v1/subscriptions/:subscription_id/cancel` — cancel subscription
- `POST /api/v1/subscriptions/:subscription_id/resume` — resume paused subscription
- `POST /api/v1/subscriptions/:subscription_id/acknowledge` — acknowledge purchase (Google Play 3-day rule)
- `POST /api/v1/subscriptions/:subscription_id/portal` — redirect to provider billing portal
- `POST /api/v1/subscriptions/:subscription_id/price-step-up/accept` — accept price increase
- `POST /api/v1/subscriptions/:subscription_id/price-step-up/decline` — decline price increase
- `GET /api/v1/payments` — list payment history
- `GET /api/v1/agent/balance` — check agent credit balance
- `POST /api/v1/agent/token` — create scoped charge token
- `POST /api/v1/agent/charge` — atomically deduct credits
- `POST /api/v1/agent/topup` — manually top up agent balance
- `POST /api/v1/users/:external_user_id/anonymize` — GDPR account deletion
- `GET /api/v1/users/:external_user_id/data-export` — GDPR data export

**Admin Endpoints** (secured by Clerk organization auth):
- `GET /admin/` — admin dashboard UI
- `GET /admin/apps` — list apps with failed webhook counts
- `GET /admin/apps/:app_id/webhooks` — list webhooks for an app
- `POST /admin/webhooks/:webhook_id/retry` — manually retry failed webhook

### 3.2 Webhook Ingress

**Inbound path**: `POST /webhooks/{token}/:provider`
- `token` = `apps.webhook_ingress_token` (UUID, obfuscated per-app)
- `:provider` = payment provider identifier

**Security**:
1. Verify provider signature (provider-specific algorithm)
2. Check webhook uniqueness via `webhook_log` deduplication
3. All processing is idempotent (UPSERT semantics)

**Processing flow**:
1. Validate signature
2. Insert/check `webhook_log` (primary key: `provider_webhook_id`)
3. Resolve `external_user_id` via lookup cascade (see §53 of behavioral spec)
4. Update subscription/payment state
5. Queue callback delivery to app
6. Return 204 to provider immediately
7. (Async) Forward callback to app's `webhook_callback_url` with HMAC signature

**Callback forwarding**:
- HMAC signature via `X-Pay-Signature` header using `apps.webhook_callback_secret`
- 3-strike retry strategy (configurable)
- Dead letter queue: webhooks that exhaust retries are marked `dead_lettered` with timestamp and reason
- Stale event suppression: compare `webhook_log.timestamp_epoch_ms` against `subscriptions.last_event_time`

### 3.3 Provider Integration

**Supported providers**:
- Google Play (subscriptions + one-time purchases) — **Primary, fully implemented**
- Apple IAP (checkout metadata only) — **Partial support for app_store checkout**
- Creem — **Archived** (code exists but not instantiated)
- LemonSqueezy — **Archived** (code exists but not instantiated)
- Coinbase — **Archived** (code exists but not instantiated)

**Provider credentials**:
- Stored in `provider_configs` table (JSONB `config` column)
- Per-app configuration (app can enable/disable providers independently)

**State normalization**:
- Provider events mapped to canonical states: `pending`, `active`, `trial`, `past_due`, `grace_period`, `on_hold`, `paused`, `cancelled`, `expired`, `revoked`
- Provider events mapped to callback events: `subscription.activated`, `subscription.expired`, etc.

**Google Play specifics**:
- Purchase acknowledgment required within 3 days (5 min for testers) to prevent auto-refund
- Price step-up handling: users can accept/decline price increases
- Subscription lifecycle notifications: revocation, cancellation, price changes, payment failures

### 3.4 Database Layer

**Schema organization** (in Bridge's own PostgreSQL DB):

| Table | Purpose |
|---|---|
| `apps` | App registry, callback URLs, rate limits, provider identifiers (google_package_name, apple_bundle_id) |
| `api_keys` | API key auth (hashed) |
| `provider_configs` | Per-app provider credentials + settings |
| `subscriptions` | Subscription lifecycle (source of truth), includes payment_failure_notification flag |
| `payments` | Payment records (immutable once created) |
| `webhook_provider` | Webhook audit + deduplication (provider ingress) |
| `webhook_delivery` | Callback forwarding state, includes dead_lettered tracking |
| `checkout_idempotency` | Checkout request deduplication |
| `fraud_prevention` | Fraud detection tracking |
| `agent_credits` | Agent micropayment balances |
| `agent_transactions` | Agent ledger audit log (includes topup transactions) |
| `agent_payment_tokens` | Scoped one-time charge tokens |

**Key design decisions**:

- **Subscriptions**: One row per (app_id, external_user_id, subscription_id, provider). Unique constraint on this tuple. `purchase_token` also unique (fraud prevention). Includes `payment_failure_notification` flag for tracking payment failure events.
- **Payments**: One row per provider transaction. Atomic UPSERT with fraud detection (mismatched `external_user_id` returns 409).
- **Webhook_provider**: Two unique constraints:
  - `(provider, provider_webhook_id)` — prevents exact duplicates
  - `(provider, purchase_token, event_type)` — catches token+type duplicates from multi-app scenarios
- **Webhook_delivery**: Tracks callback forwarding state with dead letter queue support. `dead_lettered` flag marks exhausted retries.
- **Checkout_idempotency**: Prevents duplicate checkout requests via unique constraint on idempotency key.
- **Agent topup idempotency**: Unique index on `(app_id, charge_id)` where request_type='topup' prevents duplicate charge-based topups.
- **Concurrency**: Subscriptions use optimistic locking via `version` field. `last_event_time` guards against out-of-order events.

### 3.5 Background Jobs

**Reconciliation** (every 24 hours, per app):
- Polls Google Play for all active subscriptions
- Detects drift (e.g., Google says expired, Bridge says active)
- Updates subscription state and triggers callback to app
- Logs audit trail in `webhook_provider`

**Price Step-Up Expiry** (every 5 minutes):
- Auto-cancel subscriptions where Google price consent deadline passed
- Forward cancellation callback to app

**Pause Scheduler** (every 25 minutes):
- Transition subscriptions from `pause_scheduled` to `paused`
- Forward pause callback to app
- Clean up orphaned pending subscriptions (>30 min old)

**Webhook Retry Worker** (continuous):
- Processes failed webhook deliveries from `webhook_delivery` table
- Retries with exponential backoff (3-strike strategy)
- Marks exhausted retries as `dead_lettered` with timestamp and reason

**Webhook Cleanup Worker** (daily):
- Delete webhook records older than 90 days (data retention policy)

### 3.6 Email Service

**Purpose**: Send subscription lifecycle notifications to users (pass-through emails, not stored in Bridge)

**Supported providers**:
- MockEmailService (development/testing)
- ClerkEmailService (via Clerk API)
- ResendEmailService (via Resend API)

**Notification types** (Google Play):
- Subscription revocation
- Subscription cancellation
- Price step-up (increase/decline)
- Payment failure with actionable links
- Subscription restart
- Cancellation scheduled
- Deferred renewal

**Configuration**:
- `EMAIL_PROVIDER` — mock, clerk, or resend
- `CLERK_SECRET_KEY` — for ClerkEmailService
- `RESEND_API_KEY` — for ResendEmailService
- `APP_EMAIL_FROM` — from email address for Resend

---

## 4. Data Flow Examples

### 4.1 Checkout Flow

```
App                Bridge             Google Play
 |                  |                      |
 +--checkout------->|                      |
 |  (user, product) |                      |
 |                  +---create session--->|
 |                  |  (metadata, email)   |
 |                  |<---checkout_url-----+
 |<--checkout_url--+
 |
 User clicks -> Google Play payment -> Google webhook
                                          |
                                          v
                                        Bridge
                                          |
                                          v
                              Update subscription
                              Record payment
                                          |
                                          v
                              Forward callback
                                          |
                                          v
                                        App
                                          |
                                          v
                                   Update users table
```

### 4.2 Webhook Ingress + Callback

```
Google Play (webhook)
  |
  v
  Bridge /webhooks/{token}/google_play
  |
  1. Verify Google's signature ✓
  |
  2. Check webhook_log uniqueness ✓
  |
  3. Insert webhook_log (idempotent) ✓
  |
  4. Resolve external_user_id via lookup cascade ✓
  |
  5. UPSERT subscription state ✓
  |
  6. Return 204 ✓
  |
  7. (Async) Queue callback delivery
     |
     v
     App webhook_callback_url
     |
     v
     HMAC signature check
     |
     v
     Update users.is_premium
```

### 4.3 Agent Micropayment (402 Flow)

```
App                          Bridge
 |                             |
 +--GET /agent/balance-------->|
 |                             |
 |<---balance_cents (300)------+
 |
 (balance sufficient)
 |
 +--POST /agent/token-------->|
 |  (300 cents, 10 min TTL)   |
 |                             |
 |<---token_id, token_secret--+
 |
 (execute content generation)
 |
 +--POST /agent/charge------->|
 |  (token_id)                 |
 |                             |
 |  [Atomic transaction]       |
 |  - Mark token.used = true   |
 |  - Deduct balance           |
 |  - Return 200               |
 |<---balance_remaining-------+
 |
 (proceed with generation)
```

---

## 5. Authentication & Security

### API Keys
- `sk_hiha_` prefix + random suffix
- Hashed with argon2/bcrypt in DB
- Rotation: new key → old key → disable → delete
- Last-used tracking for audit

### Webhook Signatures
- **Inbound**: Provider signature verification (provider-specific algorithms)
- **Outbound**: HMAC-SHA256 on payload using app's `webhook_callback_secret`

### Rate Limiting
- Per-API-key: configurable via `apps.api_rate_limit_per_minute` + per-endpoint overrides
- Per-IP (failed auth): 10 req/min fixed
- Webhook paths: exempt from rate limiting (protected by obfuscation + signature)

### Provider Credentials
- Stored in `provider_configs.config`
- Loaded on demand when provider API calls are made

---

## 6. Data Retention & Privacy

### GDPR Compliance

**Roles**:
- Bridge = Data Processor (handles payment data on app's behalf)
- App = Data Controller (owns user relationship)

**Lawful basis**: Contractual necessity + legitimate interest (fraud prevention)

### Retention Windows

| Data | Retention | Rationale |
|---|---|---|
| Raw webhooks | 90 days | Debugging + reconciliation |
| Payments + subscriptions | 7 years / indefinite | Tax + audit compliance |
| Purchase tokens | Indefinite | Fraud prevention + "restore purchases" |
| Obfuscated IDs (deleted users) | 90 days post-deletion | Prevent re-enrollment fraud |

### Account Deletion (Right to be Forgotten)

1. App calls `POST /api/v1/users/:external_user_id/anonymize`
2. Bridge:
   - Cancels active subscriptions via provider APIs
   - Scrambles `external_user_id` → `deleted_hash_xxx` on all records
   - Preserves `purchase_token` for restore purchases (linked to new account)
3. App deletes its own user records + Clerk identity

---

## 7. Error Handling

### HTTP Status Codes

| Status | Code | When |
|---|---|---|
| 200 | OK | Success |
| 204 | No Content | Webhook processed (no response body) |
| 400 | bad_request | Invalid request |
| 400 | provider_not_configured | Provider not set up for app |
| 400 | invalid_purchase_token | Token verification failed |
| 401 | unauthorized | Missing/invalid API key |
| 403 | app_disabled | App disabled in Bridge |
| 404 | subscription_not_found | Subscription not found |
| 409 | fraud_detected | Purchase token already claimed by different user |
| 429 | rate_limit_exceeded | Rate limit hit |
| 500 | internal_error | Unexpected error |
| 502 | provider_error | Provider API error |

### Fraud Detection

- **Payment fraud**: UPSERT guards on `(app_id, provider, provider_transaction_id)` with `external_user_id` check
- **Token reuse**: Unique constraint on `subscriptions.purchase_token` prevents one token claimed by multiple users
- **Stale events**: `last_event_time` comparison prevents old webhooks overwriting fresh state

---

## 8. Configuration

### Environment Variables

| Variable | Purpose |
|---|---|
| `PORT` | HTTP server port (default 3000) |
| `DATABASE_URL` | PostgreSQL connection string |
| `ADMIN_DATABASE_URL` | Admin database connection string (optional) |
| `ENABLE_BACKGROUND_JOBS` | Enable/disable reconciliation, pause scheduler, etc. |
| `RECONCILIATION_INTERVAL_MINUTES` | How often to reconcile with providers (default 1440 = 24h) |
| `MOCK_EXTERNAL_APIS` | Set to `false` in production (panic if true) |
| `LOGGING_LEVEL` | `debug` (dev) or `info` (prod) |
| `ENVIRONMENT` | Environment name (development, production) |
| `EMAIL_PROVIDER` | Email service provider (mock, clerk, resend) |
| `CLERK_SECRET_KEY` | Clerk API key for email service |
| `RESEND_API_KEY` | Resend API key for email service |
| `APP_EMAIL_FROM` | From email address for Resend |

### Per-App Configuration

Stored in `apps` table:
- `webhook_callback_url` — where Bridge sends events
- `webhook_callback_secret` — HMAC secret for signing callbacks
- `webhook_ingress_token` — obfuscated webhook path token
- `api_rate_limit_per_minute` — default rate limit
- `api_rate_limit_rules` — endpoint-specific limits (JSONB)
- `app_url` — app's public URL (used in checkout redirects)
- `google_package_name` — Google Play package name for app
- `apple_bundle_id` — Apple App Store bundle ID for app

Per-app provider config in `provider_configs`:
- Credentials (API keys, service accounts, webhook secrets)
- Provider-specific settings (endpoints, product IDs, verification flags)

---

## 9. Future Enhancements (Deferred)

### Admin Dashboard (Partially Implemented)
**Implemented**:
- Dashboard UI with app listing
- Failed webhook counts per app
- Webhook viewing per app
- Manual webhook retry endpoint (TODO: queue implementation)

**Deferred**:
- Subscription state inspection
- API key rotation
- Rate limit tuning
- Provider credential management
- Reconciliation job monitoring

### Analytics
- Payment volume, success rates
- Provider health metrics
- Webhook delivery success rates
- Callback forwarding performance

---

## 10. Testing Strategy

### Unit Tests
- Provider implementations (mocked HTTP calls)
- Rate limiting logic
- Deduplication logic
- Fraud detection

### Integration Tests
- Full checkout flow (real provider APIs in staging)
- Webhook ingress + callback delivery
- Subscription state transitions
- Reconciliation job

### Load Testing
- Webhook burst handling
- API rate limiting under load
- Database connection pooling

---

## 11. Deployment & Operations

### Scaling

- **Stateless API**: Autoscaling via load balancer (no session state)
- **Database**: Single PostgreSQL instance (can scale with read replicas for webhooks)
- **Background jobs**: Can run on separate instance or same instance with feature flag

### Health Checks
- `GET /health` — no auth, returns `{ "status": "healthy", "version": "..." }`
- Database connectivity check
- Provider API connectivity (optional, background job only)

### Monitoring

Track:
- Request latency per endpoint
- API key usage + last-used timestamps
- Webhook delivery success rates
- Dead letter queue: count of dead-lettered webhooks per app
- Provider signature verification failures
- Rate limit hits per app
- Database connection pool usage
- Background job execution times
- Email delivery success/failure rates

---

## 12. Related Documents

- **[BEHAVIORAL_SPEC.md](docs.notes/BEHAVIORAL_SPEC.md)** — Step-by-step implementation flows
- **[DECISIONS.md](DECISIONS.md)** — Why architectural choices exist
- **[AGENTS.md](AGENTS.md)** — Code organization + style guidelines
