# Bridge Architecture and Security Review — Amp

Date: 2026-07-01

Scope: current worktree whole-app review. No git diff was active. Review focused on architecture/security compatibility with `DESIGN.md` and `INVARIANTS.md`.

## Executive Summary

Change requested. The highest-risk issues are in Google Play lifecycle identity, provider webhook verification hardening, admin authorization defaults, RLS compatibility for background jobs, and webhook worker claim fencing.

The scary one: Google Play lifecycle mutation still appears able to fall back to SKU / `subscription_id`, despite the invariant requiring purchase-token-only lifecycle identity.

## Critical Findings

### 1. Google Play lifecycle mutation still falls back to SKU identity

**Risk:** A stale or unrelated Google event for the same SKU can revoke, expire, or cancel the current subscription row for the same user if token lookup fails or token is absent.

**Evidence:**

- `INVARIANTS.md` says Google Play subscription lifecycle identity must strictly be resolved by `purchase_token`; product SKU / `subscription_id` must never be used as a lifecycle fallback.
- `src/services/google_play/subscription_lifecycle.rs` derives `subscription_id` from fields/webhook and passes it into generic mutation.
- `src/db/subscriptions.rs` generic transitions update by `(app_id, external_user_id, provider, subscription_id)`.
- `src/webhooks/processor/event_handlers.rs` handles `subscription.expired` by purchase token first, then falls back to `ctx.fields.subscription_id` / SKU when token lookup misses.
- `src/webhooks/processor/event_handlers.rs` handles `payment.refunded` by token first, then falls back to `matched_payment_subscription_id`, `fields.subscription_id`, and `webhook.subscription_id`.

**Required fix:**

- For `google_play`, lifecycle mutation should resolve a concrete row by purchase token, then mutate that row only.
- If a Google lifecycle event has no matching purchase-token row, suppress/no-forward or emit an informational callback, but do not mutate by SKU.
- Remove or guard Google SKU fallbacks in refund, expired, cancelled, revoked, paused, and resumed paths.

**Required test:**

- Create user subscription `sku=monthly, token=new_token`.
- Send old/refund/expired Google event for `old_token` or missing token with same SKU.
- Assert the current row remains active and is not revoked/expired.
- Add a positive test proving matching-token events update exactly that row.

### 2. Provider webhook verification can be disabled in production

**Risk:** A leaked or guessed webhook token plus misconfiguration can allow unsigned provider payloads to mutate payment/subscription state. For Google Pub/Sub push, signed Google JWTs without audience validation are weaker than endpoint-bound validation.

**Evidence:**

- `src/webhooks/ingress.rs` honors per-provider `verify_webhook_signature=false` outside mock mode for Google Play and Creem.
- `src/webhooks/ingress.rs` defaults Google audience verification to false.
- `src/services/google_play/client.rs` allows `GOOGLE_SKIP_RSA_VERIFICATION=true` and only logs a warning.
- `src/config.rs` production startup validation blocks `MOCK_EXTERNAL_APIS` and Swagger, but does not forbid RSA skip, missing Google audience verification, or provider signature-off config.

**Required fix:**

- In production, fail closed:
  - forbid `GOOGLE_SKIP_RSA_VERIFICATION=true`;
  - require Google audience validation and a non-empty expected audience;
  - reject or ignore `verify_webhook_signature=false` unless `MOCK_EXTERNAL_APIS=true`.

**Required test:**

- Startup/config tests for production rejection of RSA skip and missing audience.
- Ingress tests proving DB `verify_webhook_signature=false` is ignored or rejected outside mock mode.

## High Findings

### 3. Admin auth accepts any Clerk user if org enforcement is unset

**Risk:** If the configured Clerk instance includes non-admin users, an allowed-origin valid session can access admin APIs.

**Evidence:**

- `src/middleware/admin_auth.rs` logs that if `ADMIN_CLERK_ORG_ID` is missing, admin auth will accept any valid JWT from the configured Clerk instance.
- Org is checked only if configured.
- `src/config.rs` production config requires authorized parties, but not org membership or a subject/email allowlist.

**Required fix:**

- In production, require `ADMIN_CLERK_ORG_ID` or a strict admin subject/email allowlist.
- Treat missing admin authorization scope as startup failure, not an info log.

**Required test:**

- Production config test: missing `ADMIN_CLERK_ORG_ID` or allowlist fails.
- Middleware test: valid JWT without org is rejected when production policy requires org.

### 4. Several background DB helpers bypass RLS app context

**Risk:** Under the intended runtime role, jobs may see/update zero rows or fail unpredictably, breaking reconciliation, pause scheduling, and cleanup guarantees.

**Evidence:**

- Most DB paths use `begin_app_tx(..., app_id)` and `set_local_app_id`, but some background helpers query the pool directly:
  - `src/db/subscriptions.rs::list_reconciliation_subscriptions`
  - `src/db/subscriptions.rs::list_pending_pause_subscriptions`
  - `src/db/subscriptions.rs::mark_subscription_paused`
  - `src/db/subscriptions.rs::delete_orphaned_pending_subscriptions`
- The RLS design requires `bridge.current_app_id` / narrow bootstrap functions, with app context set through `set_local_app_id`.

**Required fix:**

- Make each helper app-scoped and use `begin_app_tx(app_id)`.
- For global cleanup, either iterate enabled app IDs or add a narrow `SECURITY DEFINER` function.

**Required test:**

- DB/RLS tests for worker paths proving each scheduler query sees only the intended app rows after `SET LOCAL`, and no cross-app rows.

### 5. Webhook state mutation is not fenced by the delivery claim

**Risk:** If processing/enrichment exceeds the lease, another worker can reclaim the same delivery while the first worker still mutates state. Stale guards reduce damage, but duplicate same-timestamp side effects and post-commit effects remain plausible.

**Evidence:**

- `src/webhooks/processor/atomic.rs` takes `delivery_id` but not `claim_token`.
- Atomic processing mutates webhook/subscription/payment state before any delivery claim validation in the same transaction.
- `src/db/webhooks.rs` fences delivery completion by `claim_token` later.
- `src/webhooks/forwarding.rs` refreshes the claim before outbound callback, but internal state mutation may already have happened.

**Required fix:**

- Pass `claim_token` into atomic processing.
- At transaction start, lock/refresh delivery with `WHERE id = ? AND app_id = ? AND claim_token = ? AND forwarded=false AND dead_lettered=false`.
- If the claim is lost, return without mutation.

**Required test:**

- Simulate stale worker A and current worker B. Assert A performs no payment/subscription/webhook mutation and B succeeds exactly once.

## Medium Findings

### 6. Payment rows silently default missing amount/currency

**Risk:** Missing provider price data becomes indistinguishable from a real zero-value transaction/trial. This weakens payment auditability and can hide provider extraction failures.

**Evidence:**

- `src/webhooks/processor/event_handlers.rs` uses `ctx.fields.amount_cents.unwrap_or(0)` for subscription event payment records.
- `src/services/google_play/product_lifecycle.rs` uses `fields.amount_cents.unwrap_or(0)` for one-time purchase handling.
- `src/db/payments.rs` defaults missing/empty currency to `N/A`.
- The money invariant says currency source should be explicit.

**Required fix:**

- For payment events that should carry money, require explicit amount/currency or mark the row/status as price-unknown with a separate nullable model.
- At minimum, avoid creating successful paid payment records with `0` / `N/A` unless the event type is explicitly zero-value/trial.

### 7. Operational PII is exposed in admin dispute emails

**Risk:** Plaintext PII lands in email provider logs and support inboxes.

**Evidence:**

- `src/webhooks/processor.rs` dispute alert body includes raw `External user ID` and `Customer email`.
- The observability invariant says logs/traces should stay conservative and sensitive identifiers should be hashed where possible.

**Required fix:**

- Use hashed external user ID by default.
- Include customer email only behind an explicit config flag or link to an authenticated admin view.
- Avoid forwarding raw provider/customer PII into third-party email unless necessary.

## Architecture Boundary Concern

The DB layer currently contains lifecycle/business policy. This conflicts with the invariant that DB should be pure queries and application should own status transitions.

Examples:

- `src/db/subscriptions.rs` contains transition-specific status changes and Google field cleanup.
- `src/db/payments.rs` contains Creem-specific stale payment adoption policy.

Recommended direction:

1. Do not start with a broad refactor.
2. Fix Google token-only mutation and production verification fail-closed behavior first.
3. As touched, move lifecycle policy out of DB functions while keeping SQL responsible for atomicity and constraints.

## Positive Evidence Checked

- Webhook ingress generally validates provider signature before parsing/mutation.
- Provider webhook dedupe is app-scoped by `(app_id, provider, provider_webhook_id)`.
- Webhook delivery is unique per provider inbox row.
- Many subscription updates use `last_event_time` guards.
- Outbound callbacks use HMAC signing.

## Verification

No tests were run for this review because it was read-only until this document was created. This document creation did not change application code.
