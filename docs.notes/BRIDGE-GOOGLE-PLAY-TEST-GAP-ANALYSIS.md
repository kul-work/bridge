# Bridge Google Play Test Gap Analysis

**Date**: 2026-05-11

## Scope

This note analyzes the audit findings in this folder against the Google Play HLD/test plan under `docs/google` and the current Bridge GPBI test scripts.

No code changes were made as part of this analysis.

## Summary

The audit findings are valid. Bridge has implementation paths for the app-facing subscription snapshot endpoint and normalized callback fields, but the Google Play shell tests still primarily validate database side effects and list-subscription responses.

The app-facing snapshot endpoint is `/api/v1/users/:external_user_id/subscription-status`. It already exposes Google lifecycle fields such as `google_new_price_cents`, `google_pause_scheduled_at`, `google_deferred_until`, and `revocation_reason`; the gap is that the Google lifecycle shell tests do not assert this endpoint after webhook scenarios.

The remaining risk is not that the database mutation path is completely untested. The risk is that Bridge can correctly update `pay.subscriptions` while still breaking the app-facing contracts used by HiHa and future Tyde apps.

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

## Recommended Test Additions

### 1. Extend lifecycle tests with snapshot assertions

After existing database assertions, add snapshot checks to these tests:

- `test-sub-09.sh`: assert revoked snapshot fields.
- `test-sub-pause-01.sh`: assert `is_premium=true` and `google_pause_scheduled_at`.
- `test-sub-pause-02.sh`: assert `is_premium=false` for paused state.
- `test-sub-25.sh`: assert `is_premium=true` and `google_deferred_until`.
- `test-sub-21.sh`: assert price step-up consent snapshot fields, including `google_new_price_cents` and `google_price_step_up_consent_deadline`.
- `test-sub-20.sh`, if used for new-price renewal behavior: assert the snapshot reflects the expected Google new-price fields after the renewal scenario.

These checks should call `/api/v1/users/:external_user_id/subscription-status` directly, not `/api/v1/subscriptions`.

