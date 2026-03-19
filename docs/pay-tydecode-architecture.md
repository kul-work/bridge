# Bridge — Multi-App Payment Gateway Architecture

> Status: **Proposal / Under Review**  
> Owner: Tyde

---

## 1. Overview

**Bridge** (`pay.tydecode.com`) is a private payment processing microservice owned by Tyde. It acts as a centralized payment gateway for all Tyde apps (hiha.app, future apps). It will act like a bank service for Tyde. It is NOT a public service — it serves only Tyde's own applications.

**Core principle**: Bridge processes and records payments. It does not know about app users, app products, or app business logic. It stores opaque identifiers from apps and forwards payment events back to them.

### Entities

- **1 company**: Tyde
- **1 payment gateway**: Bridge (`pay.tydecode.com`)
- **N apps**: hiha.app, future apps — each registered in Bridge
- Each app has its **own backend, own database, own Clerk instance, own users table**

---

## 2. Responsibilities

### Bridge owns

| Concern | Details |
|---|---|
| App registry | Which apps are registered, their provider credentials, callback URLs |
| API key auth | Apps authenticate to pay via API keys (not Clerk — Clerk is app-side) |
| Administration | A separate Tyde internal Admin UI. **Note**: This is secured by Tyde's internal, separate Clerk instance/organization, keeping Tyde admin accounts strictly isolated from public app users. |
| Subscription lifecycle | Source of truth for subscription state (active, expired, revoked, paused, etc.) |
| Transaction records | Payment records with opaque `external_user_id` + `product_id` from app |
| Agent Ledger | tracks agent credits, balances, and scoped payment tokens for micropayments |
| Webhook ingress | Receives webhooks at obfuscated `pay.tydecode.com/webhooks/{token}/:provider` URLs. **Security Note**: This obfuscated token prevents blind bulk scans, but is NOT enough on its own. It MUST be explicitly paired with provider signature cryptographic verification to prevent spoofing if the token leaks. |
| Webhook forwarding | After processing, forwards normalized events to each app's `webhook_callback_url` |
| Reconciliation | Background jobs polling Google Play / Apple per-app, detecting drift, updating subscriptions, triggering callbacks |
| Webhook dedup & audit | Idempotent processing, full payload logging |
| Provider abstraction | Google Play, Apple IAP, Creem, LemonSqueezy, Coinbase — all normalized behind a common interface |

### Each app (e.g. hiha.app) owns

| Concern | Details |
|---|---|
| Users | Own `users` table with own Clerk auth. App decides what `is_premium` means |
| Products & pricing | App knows "premium_monthly costs 299 cents" — tells pay at checkout time |
| Premium status | Updated by the app when it receives callbacks from pay.tydecode.com |
| Rate limiting | Per-user, per-endpoint throttling — app-specific business logic |
| 402 agent flow | Decides pricing (e.g., joke costs 5 cents); calls Bridge to charge/reserve |
| Notifications | Email notifications — app decides when/what to email users |
| Business logic | Content generation, AI features, etc. — fully app-side |

---

## 3. Database Schema — Bridge

> **Important Architecture Principle**: Bridge has its own, isolated PostgreSQL database natively separate from the `apps` user database described in Section 4.

### 3.1 `apps`

Top-level entity. Each registered application.

> **Security Note**: All sensitive credentials in this table (e.g., Google service account JSON, provider API keys, webhook secrets) are encrypted at rest at the application level (via AES-GCM) using a master `ENCRYPTION_KEY` environment variable.

```sql
CREATE TABLE apps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug TEXT UNIQUE NOT NULL,              -- 'hiha', 'future-app'
    display_name TEXT NOT NULL,
    -- Mobile store identifiers
    google_package_name TEXT,               -- 'com.hiha.fe'
    apple_bundle_id TEXT,                   -- 'com.hiha.fe'
    -- Google Play credentials
    google_service_account_json TEXT,        -- JSON content (encrypted at rest)
    google_verify_webhook_signature BOOLEAN DEFAULT true,
    google_verify_audience BOOLEAN DEFAULT false,
    google_pub_sub_audience TEXT,
    -- Creem credentials
    creem_api_key TEXT,
    creem_product_id TEXT,
    creem_offer_id TEXT,
    creem_otp_id TEXT,
    creem_webhook_secret TEXT,
    creem_api_url TEXT DEFAULT 'https://api.creem.io/v1',
    -- LemonSqueezy credentials
    lemonsqueezy_api_key TEXT,
    lemonsqueezy_product_id TEXT,
    lemonsqueezy_webhook_secret TEXT,
    -- Apple credentials (future)
    apple_shared_secret TEXT,
    apple_key_id TEXT,
    apple_issuer_id TEXT,
    apple_private_key TEXT,
    -- Coinbase Commerce
    coinbase_api_key TEXT,
    coinbase_webhook_secret TEXT,
    -- App callback
    webhook_callback_url TEXT NOT NULL,     -- where Bridge forwards events to the app
    webhook_callback_secret TEXT NOT NULL,  -- HMAC secret for callback verification
    -- Security & Rate Limiting
    webhook_ingress_token UUID UNIQUE NOT NULL DEFAULT gen_random_uuid(),  -- secret path component for webhook URLs
    api_rate_limit_per_minute INT DEFAULT 120,  -- global fallback rate limit per-API-key
    api_rate_limit_rules JSONB,             -- per-endpoint dynamic overrides (e.g., {"checkout": 20, "status": 100})
    -- Settings
    app_url TEXT,                           -- app's public URL (used in checkout redirects)
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
```

### 3.2 `api_keys`

App-to-Bridge authentication. Each app gets one or more API keys.

```sql
CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    key_prefix TEXT NOT NULL,               -- first 8 chars for identification (e.g. 'sk_hiha_')
    key_hash TEXT NOT NULL,                 -- bcrypt/argon2 hash of full key
    label TEXT,                             -- 'production', 'staging', 'ci'
    permissions TEXT[] DEFAULT '{}',        -- future: granular permissions
    last_used_at TIMESTAMPTZ,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_api_keys_app_id ON api_keys(app_id);
CREATE INDEX idx_api_keys_prefix ON api_keys(key_prefix);
```

### 3.3 `subscriptions`

Source of truth for subscription lifecycle. `external_user_id` is opaque — Bridge does not interpret it.

```sql
CREATE TABLE subscriptions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    external_user_id TEXT NOT NULL,         -- opaque, from the app (e.g. clerk_id)
    subscription_id TEXT NOT NULL,
    provider TEXT NOT NULL,                 -- 'google_play', 'creem', 'apple', 'lemonsqueezy'
    purchase_token TEXT,
    status TEXT NOT NULL DEFAULT 'pending', -- pending, active, trial, past_due, cancelled, expired, revoked, paused, on_hold
    current_period_end TIMESTAMPTZ,
    auto_renewing BOOLEAN,
    payment_state INT,
    cancel_reason INT,
    provider_customer_id TEXT,
    -- Cancellation / Revocation
    cancellation_initiated_at TIMESTAMPTZ,
    revocation_reason TEXT,
    revoked_at TIMESTAMPTZ,
    -- Google Play specific (prefixed)
    google_subscription_state INT,
    google_obfuscated_account_id TEXT,
    google_obfuscated_profile_id TEXT,
    google_linked_purchase_token TEXT,
    google_previous_subscription_id TEXT,
    google_grace_period_start TIMESTAMPTZ,
    google_grace_period_end TIMESTAMPTZ,
    google_cancellation_context TEXT,
    google_cancellation_feedback TEXT,
    google_initial_committed_payments INT,
    google_remaining_committed_payments INT,
    google_pending_cancellation BOOLEAN DEFAULT false,
    google_pending_cancellation_at TIMESTAMPTZ,
    google_deferred_until TIMESTAMPTZ,
    google_prepaid_ack_deadline TIMESTAMPTZ,
    google_prepaid_allow_extend_after TIMESTAMPTZ,
    google_prepaid_linked_purchase_token TEXT,
    google_requires_price_step_up_consent BOOLEAN DEFAULT false,
    google_price_step_up_consent_status TEXT,
    google_price_step_up_consent_deadline TIMESTAMPTZ,
    google_new_price_cents INT,
    google_pause_scheduled_at TIMESTAMPTZ,
    google_pause_scheduled_reason TEXT,
    google_paused_at TIMESTAMPTZ,
    google_is_manual_resume BOOLEAN,
    -- Apple specific (future, prefixed)
    apple_original_transaction_id TEXT,
    apple_web_order_line_item_id TEXT,
    apple_environment TEXT,
    -- Concurrency
    version INT NOT NULL DEFAULT 1,
    last_event_time BIGINT NOT NULL DEFAULT 0, -- epoch milliseconds, derived from webhook callback timestamp for event ordering and deduplication
    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    -- Constraints
    CONSTRAINT uq_sub_app_user_sub_provider UNIQUE (app_id, external_user_id, subscription_id, provider)
);

CREATE INDEX idx_subs_app_id ON subscriptions(app_id);
CREATE INDEX idx_subs_app_user ON subscriptions(app_id, external_user_id);
CREATE INDEX idx_subs_purchase_token ON subscriptions(purchase_token);
CREATE INDEX idx_subs_status ON subscriptions(app_id, status) WHERE status = 'active';
CREATE INDEX idx_subs_provider_status ON subscriptions(app_id, provider, status);
CREATE INDEX idx_subs_event_time ON subscriptions(app_id, external_user_id, subscription_id, last_event_time);
```

### 3.4 `payments`

Payment records. `product_id` is opaque — Bridge does not interpret it.

```sql
CREATE TABLE payments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    external_user_id TEXT NOT NULL,         -- opaque, from app
    provider TEXT NOT NULL,
    provider_transaction_id TEXT NOT NULL,
    subscription_id TEXT,                   -- optional reference
    product_id TEXT,                        -- opaque, from app (e.g. 'premium_monthly', 'otp_lifetime')
    amount_cents INT NOT NULL,
    currency TEXT DEFAULT 'USD',
    status TEXT NOT NULL,                   -- 'pending', 'success', 'failed', 'refunded'
    acknowledged_at TIMESTAMPTZ,
    webhook_received_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    CONSTRAINT uq_pay_app_provider_txnid UNIQUE (app_id, provider, provider_transaction_id)
);

CREATE INDEX idx_pay_app_user ON payments(app_id, external_user_id);
CREATE INDEX idx_pay_provider_txn_id ON payments(provider_transaction_id);
CREATE INDEX idx_pay_subscription_id ON payments(subscription_id);

### 3.5 `agent_credits`

Tracks virtual balance for automated agents. `external_user_id` is typically an email as per human/agent bifurcation principle.

```sql
CREATE TABLE agent_credits (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    external_user_id TEXT NOT NULL,         -- opaque, from app (e.g. agent email)
    balance_cents INT NOT NULL DEFAULT 0,
    lifetime_spent_cents INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    CONSTRAINT uq_agent_app_user UNIQUE (app_id, external_user_id)
);

CREATE INDEX idx_agent_credits_user ON agent_credits(app_id, external_user_id);
```

### 3.6 `agent_transactions`

Audit log for credit top-ups and deductions.

```sql
CREATE TABLE agent_transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    external_user_id TEXT NOT NULL,
    request_type TEXT NOT NULL,              -- 'topup', 'story', 'joke' (defined by app)
    amount_cents INT NOT NULL,               -- positive=topup, negative=deduction
    charge_id TEXT,                        -- Coinbase/Creem ID for topups
    status TEXT NOT NULL DEFAULT 'pending', -- 'pending', 'completed', 'failed'
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_agent_transactions_user ON agent_transactions(app_id, external_user_id);
```

### 3.7 `agent_payment_tokens`

Scoped one-time payment tokens inside Bridge to enable atomic reservation workflows (prevents race conditions).

```sql
CREATE TABLE agent_payment_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    external_user_id TEXT NOT NULL,
    endpoint TEXT NOT NULL,                  -- 'story', 'joke'
    amount_cents INT NOT NULL,
    nonce TEXT NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX idx_agent_tokens_unique_request ON agent_payment_tokens(app_id, external_user_id, endpoint, nonce);
CREATE INDEX idx_agent_tokens_user ON agent_payment_tokens(app_id, external_user_id);
```

### 3.8 `webhook_log`
```

### 3.5 `webhook_log`

Webhook deduplication, audit trail, and forwarding status.

```sql
CREATE TABLE webhook_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    provider_webhook_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    subscription_id TEXT,
    purchase_token TEXT,
    payload JSONB NOT NULL,
    processed BOOLEAN DEFAULT false,
    -- Forwarding to app
    forwarded BOOLEAN DEFAULT false,
    forwarded_at TIMESTAMPTZ,
    forward_status_code INT,               -- HTTP status from app callback
    forward_attempts INT DEFAULT 0,
    next_forward_retry_at TIMESTAMPTZ,     -- required for the spaced/backoff 3-strike strategy
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_wh_app_provider_id ON webhook_log(app_id, provider, provider_webhook_id);
CREATE UNIQUE INDEX idx_wh_token_type ON webhook_log(app_id, provider, purchase_token, event_type)
    WHERE purchase_token IS NOT NULL;
CREATE INDEX idx_wh_forward_pending ON webhook_log(forwarded, next_forward_retry_at)
    WHERE forwarded = false;
```

---

## 4. Database Schema — hiha.app (separate DB)

hiha.app keeps its own database. These tables are NOT in pay.tydecode.com.

```sql
-- App-specific users (own Clerk instance)
CREATE TABLE users (
    clerk_id TEXT PRIMARY KEY,
    email TEXT,
    is_premium BOOLEAN DEFAULT false,
    premium_activated_at TIMESTAMPTZ,
    premium_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- App-specific rate limiting
CREATE TABLE rate_limits (
    clerk_id TEXT NOT NULL,
    endpoint TEXT NOT NULL DEFAULT 'joke',
    request_count INT DEFAULT 0,
    window_reset_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (clerk_id, endpoint)
);

-- App-specific notifications
CREATE TABLE notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    clerk_id TEXT NOT NULL,
    recipient_email TEXT NOT NULL,
    notification_type TEXT NOT NULL,
    subscription_id TEXT,
    provider TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    error_message TEXT,
    retry_count INT NOT NULL DEFAULT 0,
    sent_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
```

---

## 5. Practical Deployment: Shared Database (Initial) → Separate Databases (Growth)

### Multi-App in One Database

For cost and deployment efficiency, start with **one PostgreSQL database** containing multiple schemas:

```sql
CREATE SCHEMA pay;      -- pay.apps, pay.subscriptions, pay.payments, pay.webhook_log
CREATE SCHEMA hiha;     -- hiha.users, hiha.rate_limits, hiha.agent_credits, ...
CREATE SCHEMA app2;     -- app2.users, app2.rate_limits, ... (future apps)
```

**Why schemas instead of table prefixes:**
- **Transparent to code**: Set `search_path = hiha;` once per connection; queries use `FROM users` (not `FROM hiha_users`)
- **Easy migration**: When hiha grows, export schema to new database and update connection string — no code changes
- **Access control**: Grant entire schema to app-specific DB roles (`GRANT ALL ON SCHEMA hiha TO hiha_app_user`)

### Splitting to Separate Databases

When app grows, simply:
1. Dump the schema: `pg_dump -n hiha shared_db > hiha_backup.sql`
2. Create new database, import schema
3. Update app connection string
4. Done — pay remains on shared DB with other light apps

---

## 6. Flows

### 6.1 Checkout Flow

1. **User** → hiha.fe: "I want premium"
2. **hiha.fe** → hiha.app BE: authenticated request (Clerk JWT)
3. **hiha.app** → Bridge: `POST /api/v1/checkout` with API key, `external_user_id`, `product_id`, `amount_cents`, `provider`
4. **Bridge** → creates checkout session with provider (Google Play / Creem / etc.)
5. **Bridge** → returns checkout URL or purchase parameters to hiha.app
6. **hiha.app** → hiha.fe: redirect / initiate purchase

### 6.2 Webhook Flow (Provider → Bridge → App)

1. **Google Play / Apple / Creem** → `pay.tydecode.com/webhooks/{webhook_ingress_token}/google_play`
2. **Bridge**: verify signature, dedup, parse event, store in `webhook_log`
3. **Bridge**: normalize provider event to canonical type (see table below)
4. **Bridge**: update `subscriptions` table (status, period_end, provider-specific fields)
5. **Bridge**: record in `payments` table if payment event
6. **Bridge** → `POST hiha.app/webhook_callback_url` with normalized event:
   ```json
   {
     "event_id": "evt_uuid",
     "event_type": "subscription.grace_period",
     "external_user_id": "clerk_abc",
     "subscription_id": "premium_monthly",
     "product_id": "premium_monthly",
     "provider": "google_play",
     "status": "past_due",
     "current_period_end": "2026-04-18T00:00:00Z",
     "amount_cents": 299,
     "timestamp": "2026-03-18T10:05:00Z",
     "timestamp_epoch_ms": 1711000700000
   }
   ```
7. **hiha.app**: receives callback, verifies HMAC signature, updates own `users.is_premium` (typically keeps access during grace period)

> **Note on Callback Resilience**: Callback delivery uses a **3-strike retry system**. If the app doesn't respond with a 2xx status, a background job (running every 5 minutes) will retry delivery up to 3 times. Failed attempts increment `webhook_log.forward_attempts`. If it fails 3 times, it remains `forwarded = false`, natively acting as a dead-letter record without additional infrastructure. This job also monitors for permanently failed callbacks and pushes alerts to Discord/Slack.

#### Provider Event Normalization

Pay normalizes provider-specific events to canonical types for consistent app-side handling:

**Creem Events → Canonical**:
| App Status | Provider Event | Canonical Event | App Access |
|---|---|---|---|
| trial | `subscription.trialing` | `subscription.trial_started` | ✅ Grant |
| active | `subscription.active` | `subscription.activated` | ✅ Grant |
| active | `subscription.paid` | `subscription.activated` | ✅ Grant |
| past_due | `subscription.past_due` | `subscription.grace_period` | ✅ Retain (retry in progress) |
| cancelled | `subscription.canceled` | `subscription.cancelled` | ⏳ Retain until period_end |
| paused | `subscription.paused` | `subscription.paused` | ❌ Revoke |
| expired | `subscription.expired` | `subscription.expired` | ❌ Revoke |

**Google Play Events → Canonical**:
| App Status | Provider Event | Canonical Event | App Access |
|---|---|---|---|
| active | `SUBSCRIPTION_PURCHASED` | `subscription.activated` | ✅ Grant |
| active | `SUBSCRIPTION_RENEWED` | `subscription.activated` | ✅ Grant |
| active | `SUBSCRIPTION_RECOVERED` | `subscription.recovered` | ✅ Grant |
| active | `SUBSCRIPTION_RESTARTED` | `subscription.activated` | ✅ Grant |
| past_due | `SUBSCRIPTION_IN_GRACE_PERIOD` | `subscription.grace_period` | ✅ Retain (retry in progress) |
| on_hold | `SUBSCRIPTION_ON_HOLD` | `subscription.on_hold` | ❌ Revoke |
| cancelled | `SUBSCRIPTION_CANCELED` | `subscription.cancelled` | ⏳ Retain until period_end |
| paused | `SUBSCRIPTION_PAUSED` | `subscription.paused` | ❌ Revoke |
| expired | `SUBSCRIPTION_EXPIRED` | `subscription.expired` | ❌ Revoke |
| revoked | `SUBSCRIPTION_REVOKED` | `subscription.revoked` | ❌ Revoke |

### 6.3 Reconciliation Flow

> **Frequency Note**: This is a fallback mechanism (since webhooks handle real-time updates). To avoid hitting strict provider API rate limits (Google/Apple), this background job runs **once every 24 hours**.

1. **Bridge** (background job): iterates over all apps with Google Play / Apple configured
2. For each app: queries active subscriptions from `subscriptions` table
3. Polls Google Play API / Apple API for current state
4. If drift detected (e.g. Google says expired, Bridge says active):
    - Updates `subscriptions.status` in Bridge DB
    - Triggers callback to app's `webhook_callback_url` with normalized event
5. **hiha.app**: receives callback, updates `users.is_premium = false`

### 6.4 Purchase Verification Flow (Mobile)

1. **hiha.fe** (mobile app): user completes in-app purchase, gets purchase token
2. **hiha.fe** → hiha.app: sends purchase token for verification
3. **hiha.app** → Bridge: `POST /api/v1/verify-purchase` with token + subscription_id
4. **Bridge**: verifies token with Google Play / Apple API
5. **Bridge**: stores/updates subscription, records payment
6. **Bridge** → hiha.app: returns verification result (status, period_end)
7. **hiha.app**: updates `users.is_premium` based on result

### 6.5 Subscription Cancellation Flow

1. **User** → hiha.fe → hiha.app: "cancel my subscription"
2. **hiha.app** → Bridge: `POST /api/v1/subscriptions/:id/cancel`
3. **Bridge**: calls provider API to cancel
4. **Bridge**: updates `subscriptions.status`
5. **Bridge** → hiha.app: callback with cancellation event
6. **hiha.app**: updates user status as needed

### 6.6 Anonymization Flow (GDPR/CCPA "Right to be Forgotten")

See [Section 10.3](#103-right-to-be-forgotten-account-deletion-flow) for the full end-to-end deletion flow.

Bridge's role in this flow: receives `POST /api/v1/users/:external_user_id/anonymize`, cancels active subscriptions, scrambles identifiers, and logs subsequent provider webhooks for audit. Bridge does **NOT** emit separate callbacks to the app for anonymization-triggered cancellations — the app already knows the outcome synchronously from the `anonymize` endpoint response.

### 6.7 Agent Micropayment Flow (402)

To avoid charging an agent for execution that fails halfway through (e.g., OpenAI timeout), Bridge uses a **scoped one-time token reservation flow**:

1. **Agent Request**: Agent hits `hiha.app` endpoint (e.g., `/api/v1/story`) with `X-Agent-Email`.
2. **Balance Check**: `hiha.app` calls Bridge `GET /api/v1/agent/balance` to verify funds.
   - If balance is insufficient, `hiha.app` returns `402 Payment Required` with top-up instructions (Coinbase checkout links).
3. **Token Creation**: If balance is sufficient, `hiha.app` calls Bridge `POST /api/v1/agent/token` to create a 10-minute validity token locking the price (e.g., 300 cents).
4. **Endpoint Guard (Atomic Reservation)**: At the moment of content generation, `hiha.app` calls Bridge `POST /api/v1/agent/charge` with the token.
5. **Bridge executes single transaction**:
   - Updates `agent_payment_tokens` to `used = true`
   - Atomically deducts locked amount from `agent_credits.balance_cents`
6. **Execution**: `hiha.app` proceeds with content generation (OpenAI call).

This flow ensures concurrent requests do not oversubscribe available balance, and any state collision fails safely inside Bridge transactions without causing account drift.

---

## 7. Authentication Between Components

| Boundary | Auth method |
|---|---|
| User → hiha.fe → hiha.app | Clerk JWT (app's own Clerk instance) |
| hiha.app → Bridge Pay | API key (`Authorization: Bearer sk_hiha_xxxxx`) |
| Bridge Pay → hiha.app (callbacks) | HMAC signature on payload (`X-Pay-Signature` header) |
| Google/Apple/Creem → Bridge Pay | Provider-specific signature verification |

---

## 8. Rate Limiting & Health Endpoints

### 8.1 Bridge Pay

**API endpoints** (`/api/v1/*`) — rate limited per API key:

| Scope | Default | Configurable | Storage |
|---|---|---|---|
| Per API key | `api_rate_limit_per_minute` from `apps` table (default: 120) | Yes, per-app | In-memory (token bucket) |
| Per IP (unauthenticated / auth failures) | 10 req/min | No | In-memory |

**Webhook endpoints** (`/webhooks/{token}/:provider`) — **NOT rate limited**. Protection relies on:

1. **Obfuscated paths** — each app gets a unique `webhook_ingress_token` (UUID v4). The full webhook URL is `pay.tydecode.com/webhooks/{token}/{provider}`, unguessable by random clients. Providers (Google Play, Creem, etc.) are configured with this URL
2. **Provider signature verification** — every webhook payload is cryptographically verified before processing
3. **Deduplication** — `webhook_log` table prevents reprocessing of duplicate events

**Health endpoint**: `GET /health` — no authentication, no rate limiting. Used for uptime monitoring and load balancer health checks.

### 8.2 Each app (e.g. hiha.app)

Apps implement their own business-level rate limiting (per-user, per-endpoint). This is app-specific logic (e.g., free users: 10 jokes/min, premium: unlimited).

---

## 9. What Migrates From Current Codebase

### Code that stays in Bridge (extracted from current hiha)

| Current location | Becomes |
|---|---|
| `src/services/payment.rs` | Core payment trait + models (adapted for multi-app) |
| `src/services/creem.rs` | Creem provider (loads creds from `apps` table, not env) |
| `src/services/lemonsqueezy.rs` | LemonSqueezy provider |
| `src/services/google_play/` | Google Play provider |
| `src/services/coinbase.rs` | Coinbase provider |
| `src/webhooks/` | Webhook ingress + processing (adds forwarding to app) |
| `src/schedule.rs` (reconciliation) | Reconciliation job (loops over apps, not single config) |
| `src/handlers/payments/` | Payment API handlers (adapted for API key auth) |
| `src/db/subscriptions.rs` | Subscription DB queries (adds `app_id` to all queries) |
| `src/db/payments.rs` | Transaction DB queries (adds `app_id`) |
| `src/db/webhooks.rs` | Webhook dedup queries (adds `app_id`) |
| `src/handlers/pages.rs` | Admin pages |
| `templates/`, `static/` | App UI |

### Code that moves to hiha.app BE (new service)

| Current location | Becomes |
|---|---|
| `src/handlers/content.rs` | Joke/story endpoints (calls pay API for premium check) |
| `src/services/openai.rs` | OpenAI integration |
| `src/handlers/guards.rs` | Auth + rate limiting (calls pay API instead of direct DB) |
| `src/db/premium.rs` | Premium check (local DB, updated via pay callbacks) |
| `src/db/notifications.rs` | Email notification tracking |
| `src/services/clerk.rs` | Clerk auth (app's own Clerk) |
| `src/services/email.rs` | Email service |


### Part code that is removed (no longer needed)

| Current location | Why |
|---|---|
| `src/config.rs` (provider env vars) | Provider creds move to `apps` table in pay DB |
| `src/handlers/payments/provider_factory.rs` | Replaced by per-app provider lookup from DB |
| `src/main.rs` (provider registration from env) | Pay loads providers dynamically per app |


---

## 10. Data Retention & Privacy (GDPR/CCPA)

### 10.1 Roles & Lawful Basis

**Roles & DPA Requirement**: 
Each app using Bridge must sign a Data Processing Agreement (DPA). Under the GDPR:
- **Bridge** acts as the **Data Processor** (handling payment events, webhooks, and subscriptions on the app's behalf).
- **The client app** (e.g., `hiha.app`) acts as the **Data Controller** (owning the user relationship, email, and primary PII).
- **Sub-processors**: Bridge utilizes Google Play, Apple, Creem, LemonSqueezy, and Coinbase (with adequate safeguards, using SCCs for EU → US transfers).

**Lawful Basis**:
- **Contractual Necessity** (GDPR 6(1)(b)): Processing required to deliver payment services.
- **Legitimate Interest** (GDPR 6(1)(f)): Fraud prevention, accounting, and tax compliance.

**Transparency**: 
Apps MUST include Bridge in their Privacy Policy and inform users that payment data flows through it.

### 10.2 Data Retention Policies

To prevent unchecked database bloat, limit PII exposure windows, and comply with tax laws, Bridge enforces these lifecycle policies:

| Data Type | Storage | Retention | Rationale / Cleanup Action |
|---|---|---|---|
| **Raw Webhooks** | `webhook_log` | **30 - 90 Days** | Payloads are huge and contain raw provider JSON. Needed for short-term debugging/reconciliation. Cleaned up via cron (`DELETE FROM webhook_log WHERE created_at < NOW() - 30 days`). |
| **Mobile Obfuscated IDs** | `fraud_prevention` (pseudo) | **30 Days post-deletion** | `google_obfuscated_account_id` and similar markers are retained for 30 days *after* account deletion to prevent immediate fraudulent re-enrollment, then purged. |
| **Payments & Subscriptions** | `payments`, `subscriptions` | **7 Years / Indefinite** | Required for financial, tax, and accounting audits. Cannot be deleted, but the `external_user_id` will be scrubbed/anonymized upon user deletion. |
| **Purchase Tokens** | `subscriptions.purchase_token` | **Indefinite** | Required for "Restore Purchases" logic and ensuring the same receipt isn't fraudulently reused by a "new" user. |

### 10.3 "Right to be Forgotten" (Account Deletion Flow)

When a user deletes their account in the client app UI (e.g., `hiha.app`), the app (as Data Controller) orchestrates the global cleanup:

1. **Payment Gateway Cleanup**: 
    - App calls `POST pay.tydecode.com/api/v1/users/{clerk_id}/anonymize`.
    - **Bridge** instantly cancels any active auto-renewing subscriptions via Google Play/Apple/Provider APIs.
    - **Bridge** scrambles the `external_user_id` into an unrecoverable hash (e.g., `deleted_abc123`) on all payments/subscriptions.
    - Mobile provider IDs (`google_obfuscated_account_id` etc.) are shifted to the 30-day purge cycle.
2. **App DB Cleanup (Hard Delete)**: 
    - App runs a hard `DELETE` on all internal records (`hiha.users`, `hiha.agent_credits`, `hiha.rate_limits`, generated content).
3. **Clerk Auth Cleanup**:
    - App permanently deletes the user identity from Clerk's servers: `clerk.users.deleteUser(clerk_id)`.
4. **Third-Parties**:
    - App fires deletion requests to external marketing/email lists (Loops.so, Mailchimp, etc.).

**Restore Purchases (Edge Case)**: If a purged user later reinstalls the app and clicks "Restore Purchases", the mobile app generates a *new* Clerk ID and sends the *old* Google/Apple receipt token. Because **Bridge** kept the `purchase_token` (attached to a `deleted_*` user), it gracefully transfers/re-links that subscription to the new Clerk ID without breaking fraud constraints.

### 10.4 Data Subject Access Requests (GDPR Article 15)

- **Endpoint**: `GET /api/v1/users/:external_user_id/data-export`
- **Response**: JSON export of all data Bridge holds on the user (subscriptions, payments, webhook logs, notifications).
- **Responsibility**: The app aggregates this response with app-side data (content history, credits, etc.) and returns the full package to the end user.