# Security Audit: Creem Payment Flow

**Date:** 2026-04-22
**Auditor:** Agent Cascade
**Scope:** Bridge backend — Creem checkout, webhooks, callbacks, HMAC validation, subscription lifecycle, and reconciliation.

---

## Executive Summary

The Creem flow in Bridge has **strong foundational security** (HMAC verification, idempotency, RLS, constant-time comparison) but contains **two medium-risk gaps** and **several low-risk hardening opportunities**. No critical vulnerabilities were found.

---

## Findings

### 🔶 Medium: Webhook signature verification lacks test-mode bypass for Creem

**Location:** `c:/share/tyde/bridge/src/webhooks/ingress.rs:221-260`

Unlike Google Play (`handle_google_play`), the Creem handler **does not support an `X-Webhook-Verification-Mode: off` header override** for testing. While this is stricter, it means local/integration testing against real Creem sandbox accounts requires the actual secret — increasing secret exposure in CI/test environments.

**Recommendation:** Document this restriction or add a scoped test-mode toggle consistent with Google Play's pattern.

---

### 🔶 Medium: Metadata-based user resolution is last-resort and trust-dependent

**Location:** `c:/share/tyde/bridge/src/webhooks/processor.rs:419-422`

For Creem webhooks, `external_user_id` is ultimately resolved from provider metadata (`/metadata/user_id`, `/object/metadata/user_id`, etc.) after all DB lookups fail. This means **if an attacker can forge a signed webhook with tampered metadata, they can potentially reassign subscription events to a different user**. The HMAC signature prevents this *if* the secret is uncompromised, but the trust boundary is worth noting.

The "Creem orphan guard" at line 424-431 correctly suppresses the webhook if metadata resolution fails, which limits blast radius.

**Recommendation:** Consider adding a `verify_metadata_signature` or checksum if Creem supports signing nested metadata separately.

---

### 🟡 Low: `adopt_stale_payment` time window is generous

**Location:** `c:/share/tyde/bridge/src/db/payments.rs:322-349`

The `adopt_stale_payment` query matches records up to **24 hours old**:

```sql
AND created_at > NOW() - INTERVAL '24 hours'
```

This is a business-logic workaround for Creem's `order.created` -> `subscription.active` latency. A malicious or replayed webhook within that window could potentially adopt stale payments for a different subscription if other controls fail.

**Recommendation:** Tighten the interval to the observed maximum Creem latency (e.g., 5 minutes) or make it configurable per app.

---

### 🟡 Low: `checkout.completed` normalizes to `subscription.created` via heuristics

**Location:** `c:/share/tyde/bridge/src/webhooks/processor/normalize.rs:6-44`

The `checkout.completed` event type is mapped based on payload heuristics (`billing_type`, presence of `/subscription/id`, etc.). If Creem ever sends a malformed or unexpected payload, the wrong canonical event could fire, causing incorrect status updates.

**Recommendation:** Add explicit validation/assertions for `billing_type` presence and log a warning when falling back to structural heuristics.

---

### 🟡 Low: `constant_time_compare` has a minor early-exit on length mismatch

**Location:** `c:/share/tyde/bridge/src/webhooks/ingress.rs:496-505`

The current implementation:

```rust
if a.len() != b.len() {
    return false;
}
```

This leaks **length information** via timing. While the header is hex-encoded (predictable length), a fully constant-time comparison should avoid this branch.

**Recommendation:** Use a fixed-length comparison or a crate like `subtle` that handles this securely.

---

### 🟡 Low: Creem checkout does not validate `email` format before forwarding

**Location:** `c:/share/tyde/bridge/src/application/checkout.rs:22-23`

`email` is normalized (trimmed) but not validated against an RFC-compliant format or length limit before being passed to Creem. Malformed emails could cause provider-side failures or be used for header-injection if Creem's API is ever vulnerable.

**Recommendation:** Add a lightweight email format/length validation step.

---

## Positive Controls Observed

- **HMAC-SHA256 signature verification** with constant-time comparison (ingress).
- **Idempotency** via unique DB index on `(app_id, provider, provider_webhook_id)` plus secondary dedup on `(app_id, provider, purchase_token, event_type)`.
- **Stale event suppression** via `timestamp_epoch_ms` high-water mark.
- **Row-Level Security (RLS)** on `webhook_provider`, `webhook_delivery`, and `payments`.
- **Callback HMAC signing** (`sha256=<sig>`) for outbound webhooks to HiHa.
- **App-scoped transactions** (`begin_app_tx`) ensuring tenant isolation at the DB level.
- **Creem orphan guard** drops webhooks that cannot resolve a user after DB lookups, preventing unscoped event processing.

---

## Actionable Summary

| Priority | Fix | Location |
|----------|-----|----------|
| Medium | Add `X-Webhook-Verification-Mode` handling for Creem (or document why it's absent) | `ingress.rs` |
| Medium | Validate metadata `user_id` against a known whitelist/history if possible | `processor.rs` |
| Low | Tighten `adopt_stale_payment` window from 24h to ~5m or make configurable | `db/payments.rs` |
| Low | Harden `constant_time_compare` to avoid length early-exit | `ingress.rs` |
| Low | Validate email format before Creem checkout creation | `checkout.rs` |
| Low | Add heuristic fallback logging for `checkout.completed` normalization | `normalize.rs` |
