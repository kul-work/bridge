# Leftover Ticket: Restore Bridge-Owned Lifecycle Emails

## Context

Old `C:\share\hiha` sent user-facing emails for payment/subscription lifecycle events such as:

- `payment.failed`
- `subscription.price_step_up`
- `subscription.deferred`

In Tyde, that responsibility should live in `C:\share\tyde\bridge`, because Bridge now owns payment lifecycle processing and canonical subscription/payment state.

## Problem

Bridge already has:

- email infrastructure in `src/services/email.rs`
- event-specific email helpers in `src/services/google_play/notifications.rs`

But the live webhook processing path does **not** currently send those user-facing emails for these events.

The deeper blocker is that Bridge does not currently have a reliable, durable mapping of:

- `app_id + external_user_id -> user email`

This means adding `send_email_*()` calls alone is not enough, especially for Google Play webhook flows where email is not reliably present in the provider payload.

## What Was Confirmed

### In old HiHa

- `payment.failed` set local notification state and sent an email.
- `subscription.price_step_up` stored consent-required state and sent an email.
- `subscription.deferred` stored deferred state and sent an email.

### In Bridge today

- `payment.failed` updates payment/subscription state only.
- `subscription.price_step_up` updates subscription state only.
- `subscription.deferred` updates subscription state only.
- Bridge can send emails, but these webhook handlers do not currently do so.

## Correct Ownership

- `bridge`: send lifecycle/payment emails
- `hiha`: store app-local UX flags and surface them in app responses/callback handling

## Recommended Architecture (Option 1)

Bridge **fetches user email from HiHa on-demand** via a new internal HiHa endpoint, using the **existing webhook callback HMAC-SHA256 signature mechanism** for security. Email is used in-memory only for sending; no persistent storage in Bridge.

### 1. Create HiHa Internal Email-Lookup Endpoint

**Endpoint:** `POST /internal/bridge/email-lookup`

**Request:**
```json
{
  "clerk_id": "user_123"
}
```

**Response (200 OK):**
```json
{
  "email": "user@example.com"
}
```

**Error Responses:**
- `400` — bad request / missing `clerk_id`
- `401` — signature verification failed
- `404` — user not found
- `409` — user exists but no usable email
- `5xx` — transient error (Clerk API unavailable, etc.)

**Security:**
- Verify `X-Pay-Signature: sha256=<hmac>` header using `webhook_callback_secret` (same as webhook callbacks)
- Raw request body signed with HMAC-SHA256
- No new auth scheme; reuse existing trusted mechanism

**Implementation:**
- Add handler in `hiha/src/handlers/` (similar to `handle_bridge_callback`)
- DB-first lookup: query `hiha.users.email` by `clerk_id`
- Optional Clerk fallback: if missing/null, query Clerk admin API and cache result
- Return `404` if no email available after fallback

### 2. Bridge: Call Email-Lookup Endpoint on Webhook

In lifecycle webhook handlers (`payment.failed`, `subscription.price_step_up`, `subscription.deferred`):

1. Receive webhook event with `external_user_id` (= `clerk_id`)
2. Call HiHa email-lookup endpoint: `POST https://<hiha-url>/internal/bridge/email-lookup`
   - Sign request body with `WEBHOOK_CALLBACK_SECRET`
   - Add `X-Pay-Signature: sha256=<hmac>` header
3. If successful (`200`), extract `email` from response
4. Send lifecycle email using existing `send_email_*()` helpers
5. Discard email after send attempt (no storage)

**Error Handling:**
- `404` / `409`: skip email send, log warning (non-retryable)
- `401`: skip send, log warning (re-attempt only after config fix)
- `5xx` / timeout: **optional** short retry with backoff (HiHa temporarily unavailable)
  - Decision: should this fail the webhook, queue for retry, or just skip?
  - Recommended: skip with warning (email is nice-to-have, not critical)

### 3. Remove Email from Bridge Logs

Update `src/services/email.rs` to not log recipient email:

**Before:**
```rust
info!("Sending email via Clerk to: {}", to);
info!("Sending email via Resend to: {}", to);
```

**After:**
```rust
info!("Sending lifecycle email");
```

Do not include email in tracing spans, metrics, or error messages.

### 4. Keep Behavior Provider-Aware

- Google Play: webhook payload lacks email → Bridge fetches from HiHa
- Creem: webhook payload may include email → Bridge fetches from HiHa anyway (canonical source)
- All providers now use the same lookup path (consistency)

## Acceptance Criteria

- Bridge can resolve user email for webhook-driven lifecycle events without depending on provider payload email.
- `payment.failed` sends the expected user-facing email when processed.
- `subscription.price_step_up` sends the expected user-facing email when processed.
- `subscription.deferred` sends the expected user-facing email when processed.
- Email sending respects existing `EMAIL_PROVIDER` behavior (`mock`, `clerk`, `resend`).
- No email responsibility is added back into Tyde HiHa for these lifecycle events.

## Notes

- This should be implemented in Bridge, not HiHa.
- HiHa may still keep local app-facing cache/acknowledgment state for UX.
- If a user contact table is added, it must be covered by the same tenant isolation model as the rest of Bridge.
