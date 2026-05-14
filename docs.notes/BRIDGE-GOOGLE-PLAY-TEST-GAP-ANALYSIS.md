# Bridge Google Play Test Gap Analysis

**Date**: 2026-05-14

## Scope

This note tracks the remaining Google Play test gap after rechecking the current Bridge GPBI test scripts.

## Summary

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
