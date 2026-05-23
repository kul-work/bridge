# Google Play Price Change Parity Notes

Date: 2026-05-22

Rechecked: 2026-05-23

## Recheck Summary

The original parity finding still holds: ordinary Google Play price-change state is not durably
modeled by Bridge, so HiHa cannot expose a useful pending-price-change UI until Bridge adds those
fields. The recheck added three important corrections:

- Google Play API v2 exposes ordinary price-change data at
  `lineItems[].autoRenewingPlan.priceChangeDetails`, not the legacy/top-level
  `price_change_summary` shape currently modeled in Bridge.
- Current Google v2 `priceChangeState` enum values are `OUTSTANDING`, `CONFIRMED`, `APPLIED`, and
  `CANCELED`.
- RTDN `SUBSCRIPTION_PRICE_CHANGE_CONFIRMED` (`notificationType: 8`) is deprecated. Do not rely on
  it as the primary signal that the new price was actually charged; Google documents renewal after
  a price change as `SUBSCRIPTION_RENEWED`, while all price-change status updates use
  `SUBSCRIPTION_PRICE_CHANGE_UPDATED`.

## Context

These notes capture the parity investigation across:

- Old HiHa backend: `C:\share\hiha`
- Old HiHa frontend: `C:\share\hiha.fe`
- Current Bridge: `C:\share\tyde\bridge`
- Current HiHa backend: `C:\share\tyde\hiha`

The concrete event investigated was Google Play RTDN `SUBSCRIPTION_PRICE_CHANGE_UPDATED`
(`notificationType: 19`), normalized by Bridge as `subscription.price_change_updated`.

## Observed Live Flow

When a Google Play subscription price was changed in Play Console:

1. Bridge verified the Google Pub/Sub JWT.
2. Bridge received RTDN `notificationType: 19`.
3. Bridge fetched the Google Play subscription via Android Publisher API v2.
4. Google returned:
   - `subscriptionState = SUBSCRIPTION_STATE_ACTIVE`
   - `recurringPrice = RON 5.49`
   - `priceChangeDetails.newPrice = RON 7.49`
   - `priceChangeDetails.priceChangeMode = PRICE_INCREASE`
   - `priceChangeDetails.priceChangeState = OUTSTANDING`
   - `priceChangeDetails.expectedNewPriceChargeTime = 2026-05-22T16:58:21.621Z`
5. Bridge treated the event as informational and forwarded `subscription.price_change_updated`
   to HiHa.
6. Current HiHa accepted the callback but previously logged it as unknown.

The current recurring renewal amount remained `549` cents until Google actually charges the new
price. That part is correct.

## Old HiHa Backend Behavior

Old HiHa was the behavioral oracle for this investigation.

Relevant files:

- `C:\share\hiha\src\services\google_play\provider.rs`
- `C:\share\hiha\src\webhooks\processor.rs`
- `C:\share\hiha\src\webhooks\events\subscription\google_play.rs`
- `C:\share\hiha\docs.notes\pay-tydecode-behavioral-spec.md`

Old HiHa mapped:

- `notificationType: 8` -> `subscription.price_changed`
- `notificationType: 19` -> `subscription.price_change_updated`
- `notificationType: 22` -> `subscription.price_step_up_consent_updated`

Old HiHa explicitly dispatched `subscription.price_change_updated` to
`handle_subscription_price_change_updated`.

Behavior for `subscription.price_change_updated`:

- Lookup user by subscription ID.
- Log that a pending price change exists.
- Optionally notify user.
- Do not change premium access.
- Do not record a successful payment yet.

Behavior for `subscription.price_changed`:

- Record an audit payment row with `status = "price_changed"`.
- Notify user about the new price.
- Do not change subscription access.

## Current Bridge Behavior

Relevant files:

- `src\webhooks\processor\normalize.rs`
- `src\webhooks\processor\event_handlers.rs`
- `src\webhooks\processor.rs`
- `src\services\google_play\provider.rs`
- `src\services\google_play\models.rs`

Current Bridge already maps:

- `SUBSCRIPTION_PRICE_CHANGE_CONFIRMED` -> `subscription.price_changed` (deprecated Google RTDN)
- `SUBSCRIPTION_PRICE_CHANGE_UPDATED` -> `subscription.price_change_updated`
- `SUBSCRIPTION_PRICE_STEP_UP_CONSENT_UPDATED` -> `subscription.price_step_up`

Current Bridge behavior:

- `subscription.price_changed` records a webhook payment with `status = "price_changed"`.
- `subscription.price_change_updated` is handled as informational only.
- Bridge forwards the callback to apps.
- Bridge callback payload supports `new_price_cents`, but ordinary price-change details from
  Google `priceChangeDetails` are not currently persisted to subscription status.
- Bridge's current Google model only reads legacy/top-level `price_change_summary`; it does not
  deserialize `lineItems[].autoRenewingPlan.priceChangeDetails`.

Important distinction:

- `google_requires_price_step_up_consent`, `google_new_price_cents`, and
  `google_price_step_up_consent_deadline` currently model Korea price step-up consent.
- They should not be reused blindly for ordinary developer-initiated Google price increases.

## Current HiHa Backend Behavior

Relevant files:

- `C:\share\tyde\hiha\src\handlers\webhooks.rs`
- `C:\share\tyde\hiha\src\handlers\content.rs`
- `C:\share\tyde\hiha\src\services\bridge_client.rs`

Fix applied on 2026-05-22:

- `BridgeWebhookPayload` now deserializes `new_price_cents`.
- `subscription.price_changed` is handled as a known no-premium-access-change callback.
- `subscription.price_change_updated` is handled as a known pending price-change callback.
- Both map to active status via `resolve_status`.

Current HiHa subscription status already passes through the Korea step-up fields from Bridge:

- `price_change_consent_required`
- `google_requires_price_step_up_consent`
- `google_new_price_cents`
- `new_price`
- `google_price_step_up_consent_deadline`
- `consent_deadline`

But ordinary pending Google price-change fields are not available unless Bridge exposes them first.

## Old HiHa Frontend Behavior

Relevant files:

- `C:\share\hiha.fe\src\types\subscription.ts`
- `C:\share\hiha.fe\src\components\Billing.tsx`
- `C:\share\hiha.fe\src\components\PriceStepUpConsent.tsx`
- `C:\share\hiha.fe\src\services\subscriptionService.ts`
- `C:\share\hiha.fe\docs\GPFI\GOOGLE_PLAY_FRONTEND_INTEGRATION_PLAN.md`

Old FE polled subscription status. It did not receive webhook push directly.

Subscription status type supported:

- `price_change_consent_required`
- `google_requires_price_step_up_consent`
- `new_price`
- `price_change_effective_date`
- `consent_deadline`

Billing UI behavior:

- If `price_change_consent_required` became true, show a price-change warning card/modal.
- Show the new price and effective date when present.
- Show a countdown if `consent_deadline` exists.
- For Korea price step-up, use `PriceStepUpConsent` modal with accept/decline actions.

Service methods:

- `POST /subscription/price-step-up/accept`
- `POST /subscription/price-step-up/decline`

Frontend distinction:

- Ordinary developer price increase should guide the user to Google Play for review/acceptance.
- Korea price step-up can use the custom accept/decline backend endpoints.

## Where Ordinary Pending Price-Change Fields Should Be Added

Add this in Bridge first, then pass it through HiHa.

### 1. Bridge Google Model

File:

- `src\services\google_play\models.rs`

Add `price_change_details` under `AutoRenewingPlan`, matching Google v2:

- `lineItems[].autoRenewingPlan.priceChangeDetails.newPrice`
- `lineItems[].autoRenewingPlan.priceChangeDetails.priceChangeMode`
- `lineItems[].autoRenewingPlan.priceChangeDetails.priceChangeState`
- `lineItems[].autoRenewingPlan.priceChangeDetails.expectedNewPriceChargeTime`

The live Google response showed `priceChangeDetails` inside `autoRenewingPlan`, not in the
top-level `price_change_summary`.

Use current Google v2 enum names for `priceChangeState`:

- `OUTSTANDING`
- `CONFIRMED`
- `APPLIED`
- `CANCELED`

Do not use the older `PRICE_CHANGE_STATE_*` names for this v2 field.

### 2. Bridge Subscription Persistence

File:

- `src\db\subscriptions.rs`

Add separate fields for ordinary pending price change, for example:

- `google_pending_price_change_new_price_cents`
- `google_pending_price_change_currency`
- `google_pending_price_change_mode`
- `google_pending_price_change_state`
- `google_pending_price_change_expected_at`

Do not reuse `google_requires_price_step_up_consent` for this ordinary flow.

### 3. Bridge Webhook Handler

File:

- `src\webhooks\processor\event_handlers.rs`

For `subscription.price_change_updated`:

- Persist the pending price-change fields extracted from the enriched Google subscription.
- Treat `OUTSTANDING` as a user-review-required pending state for ordinary developer price changes.
- Treat `CONFIRMED`, `APPLIED`, and `CANCELED` as state transitions that should update or clear the
  pending fields as appropriate.
- Keep premium access unchanged.
- Continue forwarding the callback to the app.

For `subscription.price_changed`:

- Keep existing audit payment behavior.
- Remember this comes from deprecated Google RTDN `SUBSCRIPTION_PRICE_CHANGE_CONFIRMED`.
- Do not rely on this event alone to decide that the new price was charged/applied.

For `subscription.paid` / Google `SUBSCRIPTION_RENEWED` after a price change:

- Re-read Google subscription state and compare the current `recurringPrice` / latest order details
  with any pending price-change fields.
- Clear pending fields after the new recurring price is actually applied, or when Google reports the
  price change as `APPLIED`/`CANCELED`.

### 4. Bridge Subscription Status API

Files:

- `src\application\subscription_status.rs`
- `src\handlers\subscriptions.rs`

Expose ordinary pending price-change fields in the status snapshot, separately from Korea step-up:

- `google_pending_price_change_new_price_cents`
- `google_pending_price_change_currency`
- `google_pending_price_change_mode`
- `google_pending_price_change_state`
- `google_pending_price_change_expected_at`

### 5. Current HiHa Backend Pass-Through

Files:

- `C:\share\tyde\hiha\src\services\bridge_client.rs`
- `C:\share\tyde\hiha\src\handlers\types.rs`
- `C:\share\tyde\hiha\src\handlers\content.rs`

Deserialize the new Bridge fields and map them to frontend-friendly fields:

- `price_change_consent_required`
- `new_price`
- `price_change_effective_date`

Only map ordinary pending price changes to these fields when the pending state is outstanding.
Keep Korea step-up fields available separately.

Use `price_change_effective_date` from Bridge's
`google_pending_price_change_expected_at` when Google provides it.

## Decision

Classification: PARITY, adapted for Bridge split.

Bridge owns provider-specific payment state and durable payment audit rows. Current HiHa should not
recreate old HiHa's local provider/payment tables. It should consume Bridge subscription status and
Bridge callbacks.

Ordinary Google price changes and Korea price step-up consent must remain separate flows:

- Ordinary `subscription.price_change_updated`: pending Google-managed price increase review.
- Korea `subscription.price_step_up`: app/backend-managed consent flow with accept/decline actions.
