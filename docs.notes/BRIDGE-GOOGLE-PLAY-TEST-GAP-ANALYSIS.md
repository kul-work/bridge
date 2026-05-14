# Bridge Google Play Test Gap Analysis

**Date**: 2026-05-11
**Updated**: 2026-05-14

## Scope

This note analyzes the audit findings in this folder against the Google Play HLD/test plan under `docs/google` and the current Bridge GPBI test scripts.

No code changes were made as part of this analysis.

## Summary

The original audit finding about missing Google lifecycle snapshot assertions is no longer current. Bridge has implementation paths for the app-facing subscription snapshot endpoint and normalized callback fields, and the relevant Google Play shell tests now assert the app-facing subscription snapshot after webhook scenarios.

The app-facing snapshot endpoint is `/api/v1/users/:external_user_id/subscription-status`. It exposes Google lifecycle fields such as `google_new_price_cents`, `google_pause_scheduled_at`, `google_deferred_until`, and `revocation_reason`; the named Google lifecycle shell tests now call this endpoint directly.

The remaining risk is callback delivery-body coverage. Bridge can correctly update `pay.subscriptions` and expose the correct subscription snapshot while still delivering an incomplete or incorrectly mapped callback JSON payload to HiHa and future Tyde apps.

## Findings

### 1. Callback payload fields are not end-to-end contract tested

The normalized callback contract includes fields such as:

- `new_price_cents`
- `revocation_reason`
- `cancellation_mode`
- `google_price_step_up_consent_deadline`
- `google_pause_scheduled_at`
- `google_deferred_until`

Current coverage includes a Rust serialization unit test for Google lifecycle fields, but that only proves the struct serializes those fields when manually populated.

It does not prove that real webhook scenarios populate the fields, enqueue them, and deliver them to the app callback URL.

The GPBI `NET-05` test verifies that a webhook was delivered by checking `pay.webhook_delivery.forwarded`, `last_http_status`, and `last_error`. It does not validate that the correct callback JSON body was delivered to the app.

### 2. Snapshot lifecycle assertions are now covered

The previously recommended snapshot assertions are now present in the GPBI lifecycle tests:

- `tests/gpbi/test-sub-09.sh`: asserts revoked snapshot fields, including `is_premium=false`, `status="revoked"`, `revocation_reason="REFUND"`, and `revoked_at`.
- `tests/gpbi/test-sub-pause-01.sh`: asserts `is_premium=true`, `status="active"`, and `google_pause_scheduled_at`.
- `tests/gpbi/test-sub-pause-02.sh`: asserts `is_premium=false` and `status="paused"`.
- `tests/gpbi/test-sub-25.sh`: asserts `is_premium=true`, `status="active"`, and `google_deferred_until`.
- `tests/gpbi/test-sub-21.sh`: asserts price step-up consent snapshot fields, including `google_requires_price_step_up_consent`, `google_new_price_cents`, and `google_price_step_up_consent_deadline`.
- `tests/gpbi/test-sub-20.sh`: asserts the post-renewal snapshot remains active and has no pending price step-up fields.

There is also a broader snapshot contract test in `tests/test-acc-google-snapshot.sh`, but that test seeds database rows directly. It validates the endpoint contract, not the webhook-to-snapshot flow.

## Recommended Test Additions

### 1. Validate delivered callback JSON bodies

Add end-to-end assertions that inspect the actual callback JSON body delivered to the app callback URL for representative Google lifecycle scenarios.

At minimum, cover callback payloads for:

- revocation/refund: `revocation_reason`.
- price step-up consent: `new_price_cents` and `google_price_step_up_consent_deadline`.
- scheduled pause: `google_pause_scheduled_at`.
- deferral: `google_deferred_until`.
- cancellation modes where applicable: `cancellation_mode`.

These checks should validate the delivered body content, not only `pay.webhook_delivery.forwarded`, `last_http_status`, or `last_error`.
