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

## Required Work

### 1. Add Bridge-side user contact storage

Create a tenant-scoped table keyed by:

- `app_id`
- `external_user_id`

Suggested minimum fields:

- `email`
- `created_at`
- `updated_at`

Optional:

- `last_seen_at`
- `source` (`checkout`, `verify_purchase`, etc.)

### 2. Populate contact email from app-originated requests

At minimum:

- `checkout` already receives user email and should upsert it

Also review:

- verify/register flows that already know email

### 3. Expose DB lookup for lifecycle handlers

Add repository/DB methods to resolve email by:

- `app_id`
- `external_user_id`

### 4. Send emails in webhook/lifecycle handling

Restore old-HiHa-equivalent behavior for:

- `payment.failed`
- `subscription.price_step_up`
- `subscription.deferred`

Use the existing helpers in:

- `src/services/google_play/notifications.rs`

### 5. Keep behavior provider-aware

- Google Play: likely fully supported once user email is stored locally
- Creem: may also have payload email, but do not rely on webhook payload as the canonical long-term source

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
