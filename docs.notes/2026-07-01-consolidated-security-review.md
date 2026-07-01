# Bridge — Consolidated Architecture & Security Review

**Date:** 2026-07-01  
**Sources:** Amp (whole-app read-only), Crush (glm-5.2, full-layer), Gemini (architecture/invariant alignment)  
**Overall Verdict:** ⛔ REJECT — 5 blocking issues, ~10 tracked medium/low issues

All three reviewers independently converged on the same core vulnerabilities. Where findings overlap, the strongest evidence and fix guidance is merged. Where a reviewer raised a unique concern, it is clearly flagged.

---

## 🔴 Blocking / Critical Findings

### B-1. Google Play lifecycle falls back to SKU identity — INVARIANT VIOLATION

**Reviewers:** Amp, (implied by Crush B-1-adjacent)

**Risk:** A stale or unrelated Google event for the same SKU can revoke, expire, or cancel the current subscription row even when the purchase token doesn't match. This violates the core invariant that Google lifecycle identity must be resolved **by `purchase_token` only**.

**Evidence:**
- `src/webhooks/processor/event_handlers.rs` — `subscription.expired` resolves by token first, then falls back to `ctx.fields.subscription_id` / SKU.
- Same file — `payment.refunded` chains through `matched_payment_subscription_id`, `fields.subscription_id`, and `webhook.subscription_id`.
- `src/db/subscriptions.rs` — generic transitions update by `(app_id, external_user_id, provider, subscription_id)` (SKU-scoped, not token-scoped).
- `src/services/google_play/subscription_lifecycle.rs` — derives `subscription_id` and passes it into generic mutation.

**Required fix:**
- For `google_play`, resolve the subscription row by `purchase_token`. If no row is found, suppress/no-op — never fall back to SKU.
- Remove Google SKU fallbacks from all lifecycle paths: refund, expired, cancelled, revoked, paused, resumed.

**Required tests:**
- Create subscription `sku=monthly, token=new_token`. Send old/expired Google event for `old_token` or missing token with same SKU. Assert current row stays `active`.
- Positive test: matching-token event updates exactly that row.

---

### B-2. `resume_subscription` can revive terminal subscriptions (revoked/expired)

**Reviewer:** Crush (unique finding)

**Risk:** Any app can call `POST /api/v1/subscriptions/:id/resume` on a `revoked` or `expired` subscription and set it back to `active + auto_renewing = true`. A fraud-revoked subscription silently revives.

**Evidence:**
- `db/subscriptions.rs:1191-1195` — `SET status = 'active', auto_renewing = true WHERE app_id = $1 AND id = $2` — **no status guard**.
- `application/subscription_actions.rs:157-175` — calls `resume_subscription` without checking `sub.status`.

**Invariant violated:** INVARIANTS.md "Status / Lifecycle" — terminal states must not be downgraded.

**Required fix:**
- Add `AND status IN ('paused', 'cancelled')` to the DB query, **or** add an application-layer guard that returns 409/400 for `revoked`/`expired`.

---

### B-3. Google Play webhook signature verification can be fully disabled in production

**Reviewers:** Amp, Crush, Gemini (all three flagged independently — highest consensus)

This is the highest-consensus finding across all reviewers.

**Risk A — `GOOGLE_SKIP_RSA_VERIFICATION`:** When `true`, RSA signature verification is entirely skipped. Only `exp` + `iss` are parsed from an **unsigned** payload. Any forged JWT with `iss=https://accounts.google.com` and valid `exp` is accepted.

**Risk B — `GOOGLE_VERIFY_AUDIENCE` defaults to false:** Google Pub/Sub uses the **same public JWKs across all Google Cloud projects**. Without audience validation, a valid JWT from **any Google Cloud project in the world** passes signature verification and can trigger fake subscription events.

**Evidence:**
- `src/services/google_play/client.rs:92` — `verify_aud: bool` defaults `false`.
- `src/webhooks/ingress.rs:254` — `parse_bool_env("GOOGLE_VERIFY_AUDIENCE", false)`.
- `src/webhooks/ingress.rs:264` — reads `GOOGLE_SKIP_RSA_VERIFICATION`, no production guard.
- `src/services/google_play/client.rs:619` — skip path logs `warn!` and proceeds.
- `src/config.rs:81-100` — `production_startup_errors` guards only `MOCK_EXTERNAL_APIS` and `SWAGGER_ENABLED`.

**Required fix:**
- Add to `config.rs` production startup errors:
  - `GOOGLE_SKIP_RSA_VERIFICATION=true` → startup failure.
  - `GOOGLE_VERIFY_AUDIENCE=false` (or unset) → startup failure.
  - Require a non-empty expected audience string.

**Required tests:**
- Production config test: `GOOGLE_SKIP_RSA_VERIFICATION=true` fails startup.
- Production config test: missing or false `GOOGLE_VERIFY_AUDIENCE` fails startup.
- Ingress test: `verify_webhook_signature=false` from DB is ignored/rejected outside mock mode.

---

### B-5. `BridgeError::DbError` leaks raw SQL errors to API clients

**Reviewer:** Crush (unique finding)

**Risk:** Information disclosure. Provoking SQL errors reveals table names, column names, constraint names, and query fragments — schema reconnaissance aid.

**Evidence:**
- `src/error.rs:102-112` — `DbError(msg)` → `format!("Database error: {}", msg)` sent in JSON `"message"` field. The `msg` is `e.to_string()` from sqlx, which includes internal schema details.
- `db/subscriptions.rs:1178,1203` — `.map_err(|e| BridgeError::DbError(e.to_string()))` throughout DB layer.
- Contrast: `BridgeError::DatabaseError(#[from] sqlx::Error)` at `error.rs:206-217` correctly returns `"Database error occurred."` — `DbError` does not.

**Required fix:**
- Route `DbError` through the sanitized `DatabaseError` path (generic message to client, full detail in server logs only).

---

## 🟠 Medium Findings

### M-1. Several background job DB helpers bypass RLS app context

**Reviewers:** Amp, Crush

- `src/db/subscriptions.rs` — `list_reconciliation_subscriptions`, `list_pending_pause_subscriptions`, `mark_subscription_paused`, `delete_orphaned_pending_subscriptions` query the pool directly without `begin_app_tx(app_id)` / `set_local_app_id`.
- Under the intended runtime role with `FORCE ROW LEVEL SECURITY`, these queries may see zero rows or fail unpredictably, breaking reconciliation and pause/cleanup guarantees.

**Fix:** Make each helper app-scoped, or iterate enabled app IDs with a narrow `SECURITY DEFINER` for global cleanup.

---

### M-2. Webhook state mutation not fenced by delivery claim token

**Reviewer:** Amp

- `src/webhooks/processor/atomic.rs` takes `delivery_id` but not `claim_token`.
- State mutation (webhook/subscription/payment) happens before any claim validation in the same transaction.
- If a lease expires and another worker reclaims the delivery mid-processing, the first worker may still commit mutations.

**Fix:** Pass `claim_token` into atomic processing; at transaction start, lock delivery with `WHERE id = ? AND claim_token = ? AND forwarded=false AND dead_lettered=false`. Return without mutation if claim is lost.

---

### M-3. Payment rows silently default missing amount/currency

**Reviewer:** Amp

- `event_handlers.rs` — `ctx.fields.amount_cents.unwrap_or(0)` for subscription event payment records.
- `product_lifecycle.rs` — `fields.amount_cents.unwrap_or(0)` for one-time purchases.
- `db/payments.rs` — missing/empty currency defaults to `"N/A"`.
- Creates `successful paid` payment records indistinguishable from real zero-value/trial events.

**Fix:** For payment events that should carry money, require explicit amount/currency or mark as `price-unknown` with a separate nullable model. Never create a paid record with `0`/`N/A` unless the event type is explicitly zero-value.

---

### M-4. Stale suppression inconsistency: `<=` vs `<`

**Reviewer:** Crush

- `db/subscriptions.rs:946` — purchase_token path uses `<=` (allows equal timestamps).
- `db/subscriptions.rs:1014` — ON CONFLICT path uses `<` (strictly less).
- Same event re-delivered with the same `event_time_ms` can double-apply on the token path but be suppressed on the conflict path.

**Fix:** Standardize to `<` (strictly less) for both paths.

---

### M-5. `PaymentFailed` transition has no staleness guard

**Reviewer:** Crush

- `db/subscriptions.rs:359-377` — `PaymentFailed` always sets `payment_failure_notification = true` regardless of event age. No `WHERE last_event_time < $1` clause.
- A stale payment-failed webhook delivered after recovery would flip the flag on an active subscription.

**Fix:** Add staleness guard, or explicitly document the decision to skip it.

---

### M-6. `cancel_subscription_immediate` has no terminal state guard

**Reviewer:** Crush

- `db/subscriptions.rs:1156-1183` — `SET status = 'cancelled'` with no guard; a `revoked` subscription would be downgraded, overwriting `revocation_reason` and `revoked_at`.

**Fix:** Add `AND status NOT IN ('revoked', 'expired')` to the query.

---

### M-7. `process_webhook` doesn't check `processed` before re-running handlers

**Reviewer:** Crush

- `webhooks/processor.rs:1084-1097` — only checks `webhook.suppressed`, not `webhook.processed`.
- An ingress-spawned task and a scheduler recovery task could race for the same `webhook_provider_id`.
- Mitigated by DB locking and `last_event_time`, but a one-line `if webhook.processed { return Ok(None); }` guard closes the gap cheaply.

---

### M-8. `ConfigError` and `ProviderError` leak internal details to clients

**Reviewer:** Crush

- `error.rs:182-193` — `ConfigError(msg)` → `format!("Configuration error: {}", msg)` sent to client. Reveals which env vars are missing.
- `error.rs:131-142` — `ProviderError(msg)` → `msg.clone()` sent to client. May contain internal API endpoints, request IDs.

**Fix:** Return generic messages to clients; log full detail server-side only.

---

### M-9. Operational PII in admin dispute emails

**Reviewer:** Amp

- `src/webhooks/processor.rs` — dispute alert body includes raw `External user ID` and `Customer email` in plaintext, landing in email provider logs.

**Fix:** Use hashed external user ID. Include customer email only behind an explicit config flag or link to an authenticated admin view.

---

## 🟡 Low / Architectural Notes

| # | Finding | Location | Source |
|---|---------|----------|--------|
| L-1 | Subscription status stored as raw `String` (not typed enum); spelling inconsistency `pending_purchase_canceled` (1 L) vs `cancelled` (2 L) | `db/subscriptions.rs:17`, `:384` | Crush |
| L-2 | Business logic in DB layer: terminal state hierarchy in SQL CASE, fraud detection, anonymization, idle retirement | `db/payments.rs:114-121,720-750`, `db/subscriptions.rs:718-760`, `db/users.rs:32-113` | Crush |
| L-3 | Rate limit store is in-memory `Mutex<HashMap>` — not effective across multiple instances | `middleware/rate_limit.rs:38-40` | Crush |
| L-4 | IP extraction trusts `x-forwarded-for`/`x-real-ip` without trusted-proxy enforcement | `middleware/rate_limit.rs:129-150` | Crush |
| L-5 | `enrich_google_play_fields` unconditionally overwrites `google_pending_price_change_*` from API without `is_none` guard | `webhooks/processor.rs:521-527` | Crush |
| L-6 | `subscription_id` logged in plaintext in email service | `services/email.rs:188,238,259` | Crush |
| L-7 | `verify_expected_app` handler contains business logic + direct `sqlx::query!` | `handlers/api_key.rs:35-140` | Crush |
| L-8 | Raw SQL for Google obfuscated account ID and fraud prevention in ports impl layer (should be in `db/`) | `ports/impls/payment.rs:228-264` | Crush |
| L-9 | `ON CONFLICT DO NOTHING` on webhook_provider doesn't specify constraint name | `db/webhooks.rs:752` | Crush |
| L-10 | `payments.amount_cents` stored as `INT`/`i32` in schema + Rust; INVARIANTS.md says `i64`. Only `google_pending_price_change_new_price_cents` uses `BIGINT` | migrations, `db/payments.rs` | Gemini |
| L-11 | `FORCE ROW LEVEL SECURITY` + `SECURITY DEFINER` with `bridge_admin` requires `BYPASSRLS` or superuser. If not, admin dashboard breaks entirely. Undocumented assumption | migrations, `DESIGN.md` | Gemini |

---

## ✅ What All Reviewers Agree Is Working Well

- **Signature verification before mutation** — Both providers (Google Play JWT, Creem HMAC) verify before any DB write.
- **Idempotency via `webhook_provider`** — Dedup key is `(app_id, provider, provider_webhook_id)`. `ON CONFLICT DO NOTHING` before state mutation.
- **ACK after durable rows** — `204` only after `webhook_delivery` is durably stored.
- **Money as integer cents** — No `f64`/`f32` anywhere. `google_money_to_cents` uses checked arithmetic.
- **Purchase token vs transaction ID separation** — Properly separated in schema and code.
- **PII in logs** — Tokens hashed via `diagnostic_hash`, emails via `recipient_hash`, API keys never logged, webhook bodies scrubbed.
- **PII in DB** — Emails not persisted in `payments`/`subscriptions`; only as SHA-256 hash in `checkout_idempotency`.
- **App scoping on user-facing endpoints** — All handlers pass `auth.app_id`; RLS via `set_local_app_id` provides defense-in-depth.
- **Atomic webhook processing** — All mutations execute in a single Postgres transaction in `atomic.rs`.
- **Forwarding stale suppression** — `forwarding.rs` compares `payload.timestamp_epoch_ms < subscription.last_event_time` before forwarding.
- **Admin JWT verification** — RS256, JWKS-based, issuer/exp/nbf checked, kid-miss refresh, azp enforcement.
- **GDPR anonymization** — Scrambles `external_user_id` and clears sensitive linking columns on delete.

---

## Recommended Fix Priority

| Priority | ID | Fix | Effort |
|---|---|---|---|
| 🔴 P0 | B-3 | Fail-closed: `GOOGLE_SKIP_RSA_VERIFICATION` + `GOOGLE_VERIFY_AUDIENCE` startup guards | Small — `config.rs` |
| 🔴 P0 | B-2 | `resume_subscription` terminal state guard | 1-line WHERE clause |
| 🔴 P0 | B-5 | Route `DbError` through sanitized `DatabaseError` path | Small — `error.rs` |
| 🔴 P1 | B-1 | Google Play token-only lifecycle identity, remove SKU fallbacks | Medium — multiple files |
| 🔴 P1 | B-4 | Admin auth: require `ADMIN_CLERK_ORG_ID` in production | Small — `config.rs` |
| 🟠 P2 | M-4 | Standardize stale suppression to `<` | Trivial |
| 🟠 P2 | M-6 | `cancel_subscription_immediate` terminal state guard | Trivial |
| 🟠 P2 | M-7 | Add `processed` check in `process_webhook` | 1-line guard |
| 🟠 P3 | M-1 | Background job RLS app-context scoping | Medium |
| 🟠 P3 | M-2 | Claim-token fencing in atomic processor | Medium |
| 🟠 P3 | M-3 | Explicit amount/currency or price-unknown model | Medium |
| 🟡 P4 | M-8 | Sanitize `ConfigError`/`ProviderError` to clients | Small |
| 🟡 P4 | M-5 | `PaymentFailed` staleness guard | Small |
| 🟡 P4 | L-10 | `amount_cents` schema type → `BIGINT` / Rust `i64` | Migration required |
| 🟡 P5 | L-11 | Document or fix `bridge_admin` BYPASSRLS assumption | Doc or migration |
