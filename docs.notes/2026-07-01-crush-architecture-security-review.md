# Bridge Code Review — Architecture & Security

**Reviewer:** Crush (glm-5.2)
**Date:** 2026-07-01
**Verdict:** REJECT — 4 blocking concerns require fixes; 7 medium-severity issues should be tracked.

This review covered all layers: webhook ingress/processor/forwarding, auth middleware, provider services (Google Play + Creem), DB queries, application orchestrators, and ports/traits. Findings are cited with exact `file:line` references and mapped to INVARIANTS.md / DESIGN.md.

---

## Blocking Concerns

### 1. `resume_subscription` can revive revoked/expired subscriptions

**Evidence:**
- `db/subscriptions.rs:1191-1195` — `resume_subscription` executes `SET status = 'active', auto_renewing = true` with only `WHERE app_id = $1 AND id = $2` — no status guard.
- `application/subscription_actions.rs:157-175` — calls `repo.resume_subscription(app_id, sub.id)` without checking `sub.status` is in a resumable state (`paused`, `cancelled`).

**Invariant violated:** INVARIANTS.md "Status / Lifecycle" — *"terminal states are respected"*; DESIGN.md §7 — terminal states must not be downgraded.

**Impact:** An expired or revoked subscription can be set back to `active` + `auto_renewing = true` by any app calling `POST /api/v1/subscriptions/:id/resume`. A revoked subscription (fraud, refund) silently revives.

**Required fix:** Add a status precondition — either in the DB query (`AND status IN ('paused', 'cancelled')`) or in the application layer before calling the provider API. Reject `revoked`, `expired` with a 409 or 400.

---

### 2. Google Play JWT audience verification off by default

**Evidence:**
- `services/google_play/client.rs:92` — `verify_aud: bool` defaults to `false` in `new()`.
- `webhooks/ingress.rs:254` — `parse_bool_env("GOOGLE_VERIFY_AUDIENCE", false)` — defaults to **false**.
- `client.rs:678` — `validate_aud = false` in JWT validation; audience only checked if `self.verify_aud == true` (lines 751-768).

**Invariant violated:** INVARIANTS.md "Webhook Processing" — *"All webhooks validate provider signature first."* Partial verification (signature but not audience) is incomplete.

**Impact:** Without audience validation, a valid Google-issued JWT for any audience/project could be accepted as a webhook for this app. A misconfigured or adversarial Google project could deliver forged subscription events.

**Required fix:** Default `GOOGLE_VERIFY_AUDIENCE=true` in production, or require it via `production_startup_errors` in `config.rs:81-100`.

---

### 3. `GOOGLE_SKIP_RSA_VERIFICATION` has no production guard

**Evidence:**
- `webhooks/ingress.rs:264` — reads `GOOGLE_SKIP_RSA_VERIFICATION` from env, defaults false.
- `services/google_play/client.rs:619` — when true, logs `warn!` and proceeds with **no signature verification** (only exp + iss parsed from unsigned payload).
- `config.rs:81-100` — `production_startup_errors` only guards `MOCK_EXTERNAL_APIS` and `SWAGGER_ENABLED`. No guard for `GOOGLE_SKIP_RSA_VERIFICATION`.

**Invariant violated:** INVARIANTS.md "Webhook Processing" — *"All webhooks validate provider signature first."*

**Impact:** An operator who accidentally sets `GOOGLE_SKIP_RSA_VERIFICATION=true` in production disables Google Play webhook signature verification entirely — any forged webhook with `iss=https://accounts.google.com` and a valid `exp` would be accepted. Only a `warn!` log line signals the problem.

**Required fix:** Add `GOOGLE_SKIP_RSA_VERIFICATION=true is not allowed in production` to `config.rs:88` alongside the `MOCK_EXTERNAL_APIS` guard, or make it fail-closed in `ENVIRONMENT=production`.

---

### 4. `BridgeError::DbError` leaks raw SQL errors to API clients

**Evidence:**
- `src/error.rs:102-112` — `DbError(msg)` → `format!("Database error: {}", msg)` is sent in the JSON response `"message"` field. The `msg` is `e.to_string()` from sqlx errors, which includes table names, column names, constraint names, and sometimes query fragments.
- `db/subscriptions.rs:1178,1203` — `.map_err(|e| BridgeError::DbError(e.to_string()))` throughout the DB layer.

**Invariant violated:** INVARIANTS.md "Observability / PII" — conservative error reporting. DESIGN.md §7 — 500 = `"internal_error"` with generic message.

**Impact:** Information disclosure. An attacker can provoke SQL errors to learn schema details (table/column names, constraint names), aiding further attacks. The variant `BridgeError::DatabaseError(#[from] sqlx::Error)` at `error.rs:206-217` correctly returns `"Database error occurred."` — `DbError` does not.

**Required fix:** Either route `DbError` through the sanitized `DatabaseError` path (generic message to client, full detail in server log), or sanitize the message before including it in the response.

---

## Medium-Severity Findings

### 5. `process_webhook` doesn't check `processed` before re-running event handlers

- `webhooks/processor.rs:1084-1097` — only checks `webhook.suppressed`, not `webhook.processed`.
- The scheduler's `pending_delivery_action` (`scheduler.rs:25-31`) checks `provider_processed` before calling `process_webhook_atomically`, but an ingress-spawned task and a scheduler recovery task could race for the same `webhook_provider_id`.
- Mitigated by DB row-level locking and `last_event_time` guards, but the missing `processed` check is a defensive-invariant gap. Adding `if webhook.processed { return Ok(None); }` after the `suppressed` check is cheap and closes the gap.

### 6. Stale suppression inconsistency: `<=` vs `<`

- `db/subscriptions.rs:946` — purchase_token path: `if existing_sub.last_event_time <= event_time_ms` (allows equal).
- `db/subscriptions.rs:1014` — ON CONFLICT path: `WHERE subscriptions.last_event_time < EXCLUDED.last_event_time` (strictly less).
- A re-delivery with the same `event_time_ms` would pass on the purchase_token path but be suppressed on the ON CONFLICT path. Depending on which code path executes, the same event could double-apply. Standardize on `<` (strictly less) for both.

### 7. `PaymentFailed` transition doesn't gate on staleness

- `db/subscriptions.rs:359-377` — the `PaymentFailed` transition has no `WHERE last_event_time < $1` clause. It always executes `SET payment_failure_notification = true` regardless of how old the event is.
- A stale payment-failed webhook delivered after a subscription has recovered would flip `payment_failure_notification = true` on an active subscription. Only `last_event_time` is conditionally updated (`CASE WHEN last_event_time < $1 THEN $1 ELSE last_event_time END`).
- Consider adding a staleness guard or accepting this as intentional behavior (briefly documented).

### 8. `cancel_subscription_immediate` has no terminal state guard

- `db/subscriptions.rs:1156-1183` — `SET status = 'cancelled'` with only `WHERE app_id = $1 AND id = $2`. A `revoked` subscription would be downgraded to `cancelled`, overwriting `revocation_reason = 'immediate_cancel'` and `revoked_at = NOW()`.
- Add `AND status NOT IN ('revoked', 'expired')` or check in the application layer.

### 9. Raw SQL in `ports/impls/payment.rs` bypasses the DB layer

- `ports/impls/payment.rs:228-243` — inline `UPDATE pay.subscriptions SET google_obfuscated_account_id = ..., google_linked_purchase_token = ...`
- `ports/impls/payment.rs:246-264` — inline `INSERT INTO pay.fraud_prevention ... ON CONFLICT ...`
- INVARIANTS.md "Layer Boundaries" — *"DB: Pure queries. No business decisions."* All SQL should live in `db/` modules, not in the ports impl layer.

### 10. `ConfigError` and `ProviderError` leak internal details to clients

- `error.rs:182-193` — `ConfigError(msg)` → `format!("Configuration error: {}", msg)` sent to client. Reveals which env vars are missing (`admin.rs:43-45`).
- `error.rs:131-142` — `ProviderError(msg)` → `msg.clone()` sent to client. Provider error messages may contain internal API endpoints, request IDs, or structure details.
- Both should return generic messages to the client and log the full detail server-side only.

### 11. Admin auth silently degrades without `ADMIN_CLERK_ORG_ID`

- `middleware/admin_auth.rs:111-113` — if `ADMIN_CLERK_ORG_ID` is not set, the code logs a warning and proceeds with no org membership enforcement. Any valid Clerk JWT from the configured instance gets admin access.
- Should fail-closed in production (require `ADMIN_CLERK_ORG_ID` when `ENVIRONMENT=production`), or at minimum surface this as a startup error in `config.rs`.

---

## Low-Severity / Architectural Notes

| # | Finding | Location |
|---|---------|----------|
| 12 | Subscription status stored as raw `String`, not typed enum | `db/subscriptions.rs:17`; also a spelling inconsistency: `'pending_purchase_canceled'` (one L, line 384) vs `'cancelled'` (two L, elsewhere) |
| 13 | Business logic in DB layer: terminal state hierarchy in SQL CASE, fraud detection, anonymization logic, idle retirement policy | `db/payments.rs:114-121`, `db/payments.rs:720-750`, `db/subscriptions.rs:718-760`, `db/users.rs:32-113` |
| 14 | Rate limit store is in-memory `Mutex<HashMap>`, per-process — not effective across multiple instances | `middleware/rate_limit.rs:38-40` |
| 15 | IP extraction trusts `x-forwarded-for`/`x-real-ip` without trusted-proxy enforcement | `middleware/rate_limit.rs:129-150` |
| 16 | Scheduler queries bypass `app_id` scoping + `begin_app_tx`: `list_pending_pause_subscriptions`, `mark_subscription_paused`, `delete_orphaned_pending_subscriptions` | `db/subscriptions.rs:701`, `:877`, `:901` |
| 17 | `enrich_google_play_fields` unconditionally overwrites `google_pending_price_change_*` from API response without `is_none` guard | `webhooks/processor.rs:521-527` |
| 18 | `subscription_id` logged in plaintext in email service | `services/email.rs:188`, `:238`, `:259` |
| 19 | `verify_expected_app` handler contains business logic + direct `sqlx::query!` | `handlers/api_key.rs:35-140`, esp. `:68-73` |
| 20 | `ON CONFLICT DO NOTHING` on webhook_provider doesn't specify constraint name | `db/webhooks.rs:752` |

---

## What's Working Well

- **Signature verification before mutation** — Both providers verify (Google Play JWT, Creem HMAC) before any DB write. ✅
- **Idempotency via `webhook_provider`** — Dedup key is `(app_id, provider, provider_webhook_id)`, not `purchase_token + event_type`. `ON CONFLICT DO NOTHING` checked before any state mutation. ✅
- **ACK after durable rows** — `204` returned only after `webhook_delivery` is durably stored. ✅
- **Money handling** — Integer cents only, no `f64`/`f32` anywhere. `google_money_to_cents` uses `i32::try_from` with checked arithmetic. ✅
- **Purchase token vs transaction ID separation** — `provider_transaction_id` (economic/order ID) vs `provider_purchase_token` / `purchase_token` (lifecycle handle) properly separated in DB schema and code. ✅
- **PII in logs** — Purchase tokens hashed via `diagnostic_hash`, emails via `recipient_hash`, API keys never logged, webhook bodies scrubbed. ✅
- **PII in DB** — Emails not persisted in `payments`/`subscriptions`; only as SHA-256 hash in `checkout_idempotency`. Purchase tokens in dedicated fields. ✅
- **App scoping on user-facing endpoints** — All handlers pass `auth.app_id` to every repository call; RLS via `set_local_app_id` provides defense-in-depth. ✅
- **Atomic webhook processing** — All mutations (`apply_subscription_transition`, `commit_webhook_subscription`, `record_webhook_payment`, `mark_webhook_processed`) execute within a single Postgres transaction in `atomic.rs`. ✅ *(Note: reads go through the shared pool, not the transaction — acceptable since the processor passes results in-memory.)*
- **Creem signature verification** — HMAC-SHA256 with constant-time compare, config-gated, mock-only bypass. ✅
- **Admin JWT verification** — RS256, JWKS-based, issuer/exp/nbf checked, kid-miss refresh, azp enforcement. ✅
- **Forwarding stale suppression** — `forwarding.rs:113` compares `payload.timestamp_epoch_ms < subscription.last_event_time` before forwarding. ✅

---

## Recommended Fix Priority

1. **Blocking #1** (resume terminal state) — one-line WHERE clause fix, can be hot-fixed
2. **Blocking #3** (skip_rsa production guard) — add to `config.rs:88` immediately
3. **Blocking #2** (audience default) — flip default to `true` or add production guard
4. **Blocking #4** (DbError leak) — route through sanitized path
5. **Medium #6** (`<=` vs `<`) — standardize to `<`
6. **Medium #8** (cancel terminal state) — add WHERE clause
7. **Medium #5** (processed check) — one-line guard in `process_webhook`