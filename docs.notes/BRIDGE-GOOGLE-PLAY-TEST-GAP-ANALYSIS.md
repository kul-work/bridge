# Bridge Google Play Test Gap Analysis

**Date**: 2026-05-11

## Scope

This note analyzes the audit findings in this folder against the Google Play HLD/test plan under `docs/google` and the current Bridge GPBI test scripts.

No code changes were made as part of this analysis.

## Summary

The audit findings are valid. Bridge has implementation paths for the app-facing subscription snapshot endpoint and normalized callback fields, but the Google Play shell tests still primarily validate database side effects and list-subscription responses.

The remaining risk is not that the database mutation path is completely untested. The risk is that Bridge can correctly update `pay.subscriptions` while still breaking the app-facing contracts used by HiHa and future Tyde apps.

## Findings

### 2. Callback payload fields are not end-to-end contract tested

The normalized callback contract includes fields such as:

- `new_price_cents`
- `revocation_reason`
- `cancellation_mode`
- `google_price_step_up_consent_deadline`
- `google_pause_scheduled_at`
- `google_deferred_until`

Current coverage includes a Rust serialization unit test for Google lifecycle fields, but that only proves the struct serializes those fields when manually populated.

It does not prove that real webhook scenarios populate the fields, enqueue them, and deliver them to the app callback URL.

The GPBI `NET-05` test verifies `pay.webhook_delivery.forwarded`, `last_http_status`, and `last_error`. It does not validate the callback JSON body received by the app.

### 3. Existing lifecycle tests stop at database validation

Several Google lifecycle tests verify the canonical database row but do not assert the app-facing payloads.

Examples:

- `tests/gpbi/test-sub-09.sh` verifies `status='revoked'` and `revoked_at`, but does not assert snapshot `is_premium=false`, `revocation_reason='REFUND'`, or callback `revocation_reason`.
- `tests/gpbi/test-sub-pause-01.sh` verifies `google_pause_scheduled_at` in `pay.subscriptions`, but does not assert callback or snapshot `google_pause_scheduled_at`.
- `tests/gpbi/test-sub-25.sh` verifies `google_deferred_until` in `pay.subscriptions`, but does not assert callback or snapshot `google_deferred_until`.

These tests are useful, but they do not cover the contract boundary that apps consume.


## Recommended Test Additions


### 2. Extend lifecycle tests with snapshot assertions

After existing database assertions, add snapshot checks to these tests:

- `test-sub-09.sh`: assert revoked snapshot fields.
- `test-sub-pause-01.sh`: assert `is_premium=true` and `google_pause_scheduled_at`.
- `test-sub-pause-02.sh`: assert `is_premium=false` for paused state.
- `test-sub-25.sh`: assert `is_premium=true` and `google_deferred_until`.
- price step-up test: assert `google_new_price_cents` and `google_price_step_up_consent_deadline`.

These checks should call the snapshot endpoint directly, not `/api/v1/subscriptions`.

### 3. Add callback body capture for selected scenarios

Extend the callback delivery tests to validate the actual normalized JSON body received by the app.

Recommended scenarios:

- price step-up callback includes `new_price_cents` and `google_price_step_up_consent_deadline`
- pause scheduled callback includes `status='active'` and `google_pause_scheduled_at`
- deferred callback includes `google_deferred_until`
- revoke/refund callback includes `status='revoked'` and `revocation_reason='REFUND'`
- cancellation callback includes `status='cancelled'` and `cancellation_mode` for scheduled vs immediate paths

Implementation options:

- Use a lightweight local callback receiver during GPBI tests and assert captured JSON.
- If testing through HiHa, assert the recorded callback payload in HiHa's callback/audit table.
- As a lower-value fallback, assert the outbound payload in Bridge delivery diagnostics only when a deterministic capture path is unavailable.

