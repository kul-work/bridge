# Bridge Callback and Subscription Snapshot Test Coverage

**Date**: 2026-05-09

## Purpose

Bridge recently added app-facing subscription state fields in two places:

- Normalized callback JSON sent from Bridge to application backends
- Direct subscription snapshot endpoint: `GET /api/v1/users/{external_user_id}/subscription-status`

The current test scripts partially validate subscription rows and list-subscription responses, but they do not fully assert these two app-facing contracts.

## Coverage Gap

### 1. Canonical Callback JSON Contract

Bridge callbacks should be tested as app-facing JSON contracts, not only as database side effects.

The scripts should assert that callbacks include the new normalized fields when relevant:

- `new_price_cents`
- `revocation_reason`
- `cancellation_mode`
- `google_price_step_up_consent_deadline`
- `google_pause_scheduled_at`
- `google_deferred_until`

### 2. Direct Subscription Snapshot Endpoint

The scripts should call the new endpoint directly:

```text
GET /api/v1/users/{external_user_id}/subscription-status
```

This should be tested separately from:

```text
GET /api/v1/subscriptions?external_user_id=...
```

The snapshot endpoint is the simpler app-facing entitlement contract. Tests should assert the returned snapshot fields directly, not infer premium access by scanning listed subscriptions.

## Proposed Test Scenarios

### Callback Payload Scenarios

- **Price step-up**: Assert the callback includes `new_price_cents` and `google_price_step_up_consent_deadline`.
- **Pause scheduled / deferred**: Assert the callback includes `google_pause_scheduled_at` and/or `google_deferred_until` when applicable.
- **Refund / revoke**: Assert the callback includes `status = revoked`, `revocation_reason = REFUND`, and revoke timing where applicable.
- **Cancellation**: Assert the callback includes `status = cancelled` and `cancellation_mode` for scheduled vs immediate cancellation.

### Subscription Snapshot Scenarios

For `GET /api/v1/users/{external_user_id}/subscription-status`, assert the response for these states:

- `active` -> `is_premium = true`
- `trial` -> `is_premium = true`
- `past_due` -> `is_premium = true`
- `pending` -> `is_premium = false`
- `on_hold` -> `is_premium = false`
- `paused` -> `is_premium = false`
- `expired` -> `is_premium = false`
- `revoked` -> `is_premium = false`, with `revocation_reason` and `revoked_at`
- price step-up pending -> includes consent deadline and new price
- pause scheduled / deferred -> includes pause and deferred timestamps

## Acceptance Criteria

Coverage is complete when scripts fail if:

- A required callback field is missing.
- A callback field is renamed.
- A callback field is unexpectedly `null`.
- The snapshot endpoint returns the wrong `is_premium` value.
- The snapshot endpoint omits lifecycle fields that exist in Bridge subscription state.
- The snapshot response is inconsistent with the canonical subscription row in Bridge.

## Product Impact

These tests protect the app-facing Bridge contract used by HiHa and future Tyde applications. They reduce the risk that Bridge correctly updates its own database but sends incomplete callback data or exposes an incomplete entitlement snapshot to client applications.
