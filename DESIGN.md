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
- Every webhook provider event logged before state mutation (deduplication via `webhook_provider`)
- Payments UPSERTed atomically to prevent duplicates
- Subscriptions versioned for optimistic concurrency control
- Stale event suppression via high-water mark (`last_event_time`)
- Checkout idempotency via `checkout_idempotency` table

### Multi-App Design
- Bridge serves N apps (currently hiha.app, future apps)
- Provider credentials stored per-app in `provider_configs` table
- Webhook paths obfuscated per-app: `webhooks/{token}/:provider`
- API keys scoped per app

### Provider Abstraction
- Provider interface across Google Play (primary) and Creem.
- State normalization (each provider's "active" maps to Bridge's `active`).
- Signature verification on all provider webhooks (configurable per-app via `verify_webhook_signature`).
- Dynamic provider loading per-app (not startup-based env vars).

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
- `POST /api/v1/payment/checkout` — create payment session
- `POST /api/v1/verify-purchase` — verify mobile purchase token
- `POST /api/v1/purchase/register` — register external purchase attempt
- `GET /api/v1/subscriptions` — list subscriptions
- `GET /api/v1/subscriptions/:subscription_id` — fetch subscription state
- `POST /api/v1/subscriptions/:subscription_id/cancel` — cancel subscription
- `POST /api/v1/subscriptions/:subscription_id/resume` — resume paused subscription
- `POST /api/v1/subscriptions/:subscription_id/acknowledge` — acknowledge purchase (Google Play 3-day rule)
- `POST /api/v1/subscriptions/:subscription_id/portal` — redirect to provider billing portal
- `POST /api/v1/subscriptions/:subscription_id/price-step-up/accept` — accept price increase
- `POST /api/v1/subscriptions/:subscription_id/price-step-up/decline` — decline price increase
- `GET /api/v1/payments` — list payment history
- `POST /api/v1/purchase/register` — register external purchase attempt
- `POST /api/v1/users/:external_user_id/anonymize` — GDPR account deletion/anonymization
- `GET /api/v1/users/:external_user_id/data-export` — GDPR data export

**Admin Endpoints** (secured by admin auth middleware):
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
2. Check webhook uniqueness via `webhook_provider` deduplication
3. All processing is idempotent (UPSERT semantics)

**Processing flow**:
1. Validate signature
2. Insert/check `webhook_provider` (app-scoped uniqueness: `app_id`, `provider`, `provider_webhook_id`)
3. Queue a durable `webhook_delivery` work item
4. Return 204 to provider only after the event is durable
5. (Async) Resolve `external_user_id` via lookup cascade
6. (Async) Update subscription/payment state, or mark the event as suppressed/no-forward
7. (Async) Forward queued callback to app's `webhook_callback_url` with HMAC signature

**Callback forwarding**:
- HMAC signature via `X-Pay-Signature` header using `apps.webhook_callback_secret`
- 3-strike retry strategy (exponential backoff)
- Dead letter queue: webhooks that exhaust retries are marked `dead_lettered`
- Stale event suppression: compare `webhook_provider.timestamp_epoch_ms` against `subscriptions.last_event_time`
- The webhook retry worker also recovers provider inbox rows that were stored before durable callback delivery existed, using a short DB lease to avoid concurrent recovery.

### 3.3 Provider Integration

**Supported providers**:
- **Google Play**: Primary support for subscriptions and one-time purchases.
- **Creem**: Support for standard web payment checkouts.

**Provider credentials**:
- Stored in `provider_configs` table (JSONB `config` column)
- Per-app configuration (app can enable/disable providers independently)

**State normalization**:
- Provider events mapped to canonical states: `active`, `trial`, `past_due`, `paused`, `cancelled`, `expired`, `revoked`
- Provider events mapped to callback events: `subscription.activated`, `subscription.updated`, `subscription.expired`, etc.

**Google Play specifics**:
- Purchase acknowledgment required within 3 days to prevent auto-refund.
- Price step-up handling for Korea-specific consent.
- Account linking support via `obfuscated_account_id`.

### 3.4 Database Layer

**Schema organization**:

| Table | Purpose |
|---|---|
| `apps` | App registry, callback URLs, rate limits |
| `api_keys` | API key auth (hashed) |
| `provider_configs` | Per-app provider credentials |
| `subscriptions` | Subscription lifecycle source of truth |
| `payments` | Immutable payment records |
| `webhook_provider` | Webhook audit + deduplication (ingress) |
| `webhook_delivery` | Callback forwarding state + dead letter queue |

**Key design decisions**:

- **Subscriptions**: One row per (app_id, external_user_id, subscription_id, provider). Unique constraint on this tuple. `purchase_token` also unique (fraud prevention). Includes `payment_failure_notification` flag for tracking payment failure events.
- **Payments**: One row per provider transaction. Atomic UPSERT with fraud detection (mismatched `external_user_id` returns 409).
- **Webhook_provider**: Unique constraint on `(app_id, provider, provider_webhook_id)` prevents exact provider delivery duplicates. Purchase token + event type is not a valid deduplication key for renewable subscriptions because providers such as Google Play reuse purchase tokens across renewal events.
- **Webhook_delivery**: Tracks callback forwarding state with dead letter queue support. `dead_lettered` flag marks exhausted retries.
- **Checkout_idempotency**: Prevents duplicate checkout requests via unique constraint on idempotency key.
- **Concurrency**: Subscriptions use optimistic locking via `version` field. `last_event_time` guards against out-of-order events.

### 3.5 Background Workers

**Reconciliation Engine** (Daily):
- Verifies subscription states against provider APIs (drift detection).
- Self-heals inconsistent states and triggers recovery callbacks.

**Cleanup Worker** (Daily):
- Deletes webhook logs older than 90 days.
- Housekeeping for temporary states and expired tokens.

**Price Step-up Worker** (Every 5 minutes):
- Manages Korea-specific price consent lifecycle.
- Handles expiry of pending price increases.

**Pause Scheduler** (Every 25 minutes):
- Process delayed subscription pauses and automated resumes.

**Webhook Retry Worker** (Continuous):
- Processes failed callback deliveries using exponential backoff.
- Manages the dead letter queue for exhausted delivery attempts.

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
- Subscription restart / resumed
- Cancellation scheduled
- Deferred renewal
- Subscription paused
- Subscription refunded

**Configuration**:
- `EMAIL_PROVIDER` — mock, clerk, or resend
- `CLERK_SECRET_KEY` — for ClerkEmailService
- `RESEND_API_KEY` — for ResendEmailService
- `APP_EMAIL_FROM` — from email address for Resend
- `EMAIL_PROVIDER_DEFAULT_RATE_LIMIT_COOLDOWN_SECONDS` — default Resend cooldown after 429
- `EMAIL_PROVIDER_MAX_RATE_LIMIT_COOLDOWN_SECONDS` — maximum Resend cooldown after 429

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
  2. Check webhook_provider uniqueness ✓
  |
  3. Insert webhook_provider (idempotent) ✓
  |
  4. Queue durable webhook_delivery ✓
  |
  5. Return 204 ✓
  |
  6. (Async) Resolve external_user_id via lookup cascade ✓
  |
  7. (Async) UPSERT subscription state or suppress/no-forward ✓
  |
  8. (Async) Forward queued callback delivery
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
| Webhook ingress + delivery state | 90 days | Operational debugging + reconciliation; `webhook_delivery` is removed with its source `webhook_provider` row |
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
| `EMAIL_PROVIDER_DEFAULT_RATE_LIMIT_COOLDOWN_SECONDS` | Default Resend cooldown after provider 429 |
| `EMAIL_PROVIDER_MAX_RATE_LIMIT_COOLDOWN_SECONDS` | Maximum Resend cooldown after provider 429 |

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

### Admin Dashboard
**Implemented**:
- Dashboard UI with app listing.
- Failed webhook counts per app.
- Webhook viewing per app.
- Manual webhook retry (continuous queue).
- background worker status monitoring.

**Deferred**:
- Subscription state deep inspection.
- API key rotation/management.
- Rate limit tuning UI.
- Provider credential management.
- Detailed reconciliation audit logs.

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

- **[BEHAVIORAL_SPEC.md](docs/BEHAVIORAL_SPEC.md)** — Step-by-step implementation flows
- **[INVARIANTS.md](INVARIANTS.md)** — Behavioral guarantees and constraints
- **[AGENTS.md](AGENTS.md)** — Code organization + style guidelines
