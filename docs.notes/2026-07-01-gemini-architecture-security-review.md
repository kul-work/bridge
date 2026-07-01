# Bridge — Architecture and Security Review

This document contains a structured analysis of the Bridge payment processing service, evaluated against its design goals, architectural principles, security standards, and behavioral invariants.

---

## 1. Module Boundaries & Ownership

### Findings
- **Layer Separation:** The backend strictly adheres to the stated boundaries in `INVARIANTS.md`.
  - **Handlers (`src/handlers/`):** Perform HTTP orchestration, route mapping, and authentication validation. They contain no business logic or direct database operations.
  - **Application Services (`src/application/`):** Contain business rules, owner verification, and orchestration of verification and checkout.
  - **Provider Integrations (`src/services/`):** Translate provider-specific formats (Google Play, Creem) to canonical structures without deciding policy.
  - **DB Layer (`src/db/`):** Exposes pure query interfaces.
- **Tenant Isolation (RLS):** Cross-tenant data leakage is prevented via PostgreSQL Row Level Security (RLS) on all tables.
  - Runtime request transactions execute `set_local_app_id` to bind `bridge.current_app_id` before querying, forcing isolation at the database engine level.
  - Centralized lookups (like API key authentication or webhook token resolution) bypass RLS via narrow, focused `SECURITY DEFINER` helper functions to resolve the app ID context safely.

---

## 2. Idempotency & Webhook/Subscription Flows

### Findings
- **Deduplication:** Inbound webhook events are logged in the `pay.webhook_provider` table with a unique index on `(app_id, provider, provider_webhook_id)`. Duplicates are caught atomically using `ON CONFLICT DO NOTHING`.
- **Durable Webhook Queueing:** Acknowledgement of webhooks only occurs after inserting the event into the provider table and queueing it in the delivery table, protecting against callback dropouts.
- **Race Condition Prevention in Concurrent Deliveries:** The `create_webhook_delivery` query uses a PostgreSQL `ON CONFLICT DO UPDATE` trick with `(xmax = 0) AS created` to return whether the delivery record was newly inserted. This prevents duplicate/concurrent API threads from processing the same webhook callback simultaneously.
- **Transaction Theft Protection:** During verification commits, the update statement checks `payments.external_user_id = EXCLUDED.external_user_id`. If the purchase token or transaction ID is claimed by a different user ID, no rows are updated, and a `FraudDetected` error is thrown immediately.

---

## 3. Auth, Callback Validation, & Provider Signature Handling

### Findings
- **API Key Storage:** API keys are stored hashed with Argon2 or Bcrypt. The runtime auth middleware looks up candidate keys by an 8-character prefix using a `SECURITY DEFINER` function, verifying the hash in Rust.
- **Admin Dashboard Auth:** The admin panel is protected by a Clerk JWT verifier verifying signature (using Google/Clerk JWKs), clock skew, authorized parties, and organization ID scopes.
- **Provider Callback Signature:** Outbound callbacks to the client apps are signed via HMAC-SHA256 in the `X-Pay-Signature` header using the app's `webhook_callback_secret`.

> [!WARNING]
> ### Security Vulnerability: Optional Google Pub/Sub Audience Validation
> **Location:** [src/services/google_play/client.rs](file:///c:/share/tyde/bridge/src/services/google_play/client.rs#L751-L768) and [src/webhooks/ingress.rs](file:///c:/share/tyde/bridge/src/webhooks/ingress.rs#L254)
>
> In `verify_pubsub_signature`, Google Pub/Sub JWT signature verification validates the RSA signature, but audience validation is conditionally skipped unless `GOOGLE_VERIFY_AUDIENCE` is configured to `true`.
> - **Risk:** Google Cloud Pub/Sub uses the same public JWKs to sign push requests across all Google Cloud accounts. If `GOOGLE_VERIFY_AUDIENCE` is `false` (which is the default in `.env.sample`), **any valid Google Pub/Sub message from any Google Cloud project in the world** can target Bridge's webhook endpoint, pass signature verification, and trigger fake subscription events.
> - **Production Check:** The startup validation in `src/config.rs` (`validate_startup()`) does not enforce `GOOGLE_VERIFY_AUDIENCE=true` in production mode.

---

## 4. Stale Event Suppression & Concurrency

### Findings
- **High-Water Mark (last_event_time):** Updates to the `subscriptions` table always include a `last_event_time < $new_timestamp` clause. If a stale webhook event is processed, the UPDATE statement affects 0 rows, preventing old states from overwriting newer ones.
- **Superseded Event Forwarding Prevention:** The webhook forwarding worker (`forwarding.rs`) queries the database and verifies `payload.timestamp_epoch_ms < subscription.last_event_time` before calling the client app. If it is stale, the event is marked `superseded_before_forward` and suppressed, avoiding duplicate client notifications.
- **Worker Concurrency:** Webhook retry workers use `FOR UPDATE SKIP LOCKED` on the `webhook_delivery` table, preventing concurrent workers from claiming the same batch of callbacks.

---

## 5. PII Minimization & Logging Hygiene

### Findings
- **Data Minimization:** No personal user data (emails, names) is stored in the database. Opaque user identifiers (`external_user_id`) are used instead.
- **Log Scrubbing:** Logs do not output raw purchase tokens or email addresses.
  - Path redactors in `observability.rs` scrub tokens, user IDs, and subscription IDs from HTTP access logs.
  - Reqwest and API errors in `GooglePlayClient` and `CreemClient` are run through email regex and token-hash replacement before logging.
- **GDPR Compliance:** GDPR data deletion/anonymization scrambles `external_user_id` into a secure hash (`deleted_hash_xxx`) and clears sensitive linking columns (e.g. `google_obfuscated_account_id` and `google_linked_purchase_token`).

---

## 6. Compatibility with Invariants and Design Docs

We identified two areas of divergence or operational fragility relative to `INVARIANTS.md` and `DESIGN.md`:

### 1. Money Cents Storage Type Inconsistency
- **Divergence:** `INVARIANTS.md` states: *"All amounts stored as i64 cents in DB."*
- **Actual Code:** The database migrations (`03_create_payments.sql` and `02_create_subscriptions.sql`) define `payments.amount_cents` and `subscriptions.google_new_price_cents` as `INT` (32-bit integer). The Rust struct definitions in `src/db/payments.rs` use `i32` for `amount_cents`.
- **Note:** Only `google_pending_price_change_new_price_cents` uses `BIGINT` (`i64` in Rust).

### 2. Operational Fragility in RLS & Security Definership
- **Divergence:** Tables use `FORCE ROW LEVEL SECURITY`. RLS policies are defined `TO bridge_app`. Pre-auth and admin functions are defined as `SECURITY DEFINER` (executing as the owner, typically the `bridge_admin` user that runs migrations).
- **Actual Code:** Because `FORCE ROW LEVEL SECURITY` is enabled, the table owner (`bridge_admin`) is subject to RLS. Since there are no RLS policies defined `TO bridge_admin` (or `TO public` allowing admin access), if `bridge_admin` is not a PostgreSQL superuser and does not have the `BYPASSRLS` attribute, RLS will block all queries made inside `SECURITY DEFINER` functions, breaking the admin dashboard entirely.
- **Note:** The system assumes `bridge_admin` connects with superuser privileges or `BYPASSRLS`. This dependency should be documented or solved by explicitly defining policies or roles.
