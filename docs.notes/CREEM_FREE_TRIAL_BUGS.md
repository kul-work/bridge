# Creem Free Trial Subscription Bugs

Date investigated: 2026-05-28

Scope: Bridge handling of Creem free-trial subscription webhooks for HiHa user `user_36lLgcNtpsqKzB5hpan8wYIN5ew`.

## Evidence Summary

The logs and `pay` schema show two different Creem subscription IDs for the same user:

- `sub_72m8alccxoHznvp4M5PJxW`: active, `current_period_end = 2026-06-04T11:46:27Z`
- `sub_4hfsVxvgJwBG6tT7Hs6HBY`: cancelled, `current_period_end = 2026-06-04T11:41:34Z`

Bridge also recorded two payment rows:

- `tran_5j6E7PN2Shj2kQL11PJvC3`, `amount_cents = 450`, subscription `sub_72m8alccxoHznvp4M5PJxW`
- `tran_3oSB1Zj9PZf2RAd9jqPxTc`, `amount_cents = 450`, subscription `sub_4hfsVxvgJwBG6tT7Hs6HBY`

Both source Creem `subscription.paid` payloads had `object.status = trialing` and `last_transaction.amount_paid = 0`, while `last_transaction.amount = 450`.

## Bug 1: Trial Invoice Is Recorded As Full-Price Payment

### Current Behavior

For Creem webhooks, Bridge extracts payment amount from:

1. `object.last_transaction.amount`
2. `object.order.amount`
3. `object.product.price`
4. `object.amount`

This happens in `src/webhooks/processor/fields.rs`.

For the observed `subscription.paid` trial events, Creem sent:

- `last_transaction.amount = 450`
- `last_transaction.amount_paid = 0`
- `object.status = trialing`

Bridge recorded `amount_cents = 450`.

### Problem

`last_transaction.amount` appears to be the recurring invoice/list amount, not the actual cash collected during the free trial. For trial invoices, `amount_paid = 0` is the stronger signal for persisted payment amount.

The DB therefore shows a full-price successful payment even though the Creem transaction indicates zero paid.

### Impact

- Payment history overstates revenue.
- Admin/accounting views can show successful paid transactions during free trial signup.
- Downstream analytics may treat trial starts as paid conversions.

### Fix Direction

For Creem `last_transaction` payloads, prefer actual paid/captured amount when present:

- Use `last_transaction.amount_paid` for payment rows when available.
- Treat `amount_paid = 0` as a real zero, not as missing data, and record a zero-amount payment row for trial invoices.
- Keep product/recurring price separately as recurring amount if needed.

## Bug 2: Trial Status Is Overwritten To Active

### Current Behavior

Bridge maps Creem events as:

- `subscription.trialing` -> `subscription.trial_started`
- `subscription.paid` -> `subscription.activated`
- `checkout.completed` for recurring products -> `subscription.created`

The activation handler then commits the subscription with `status: "active"` for all:

- `subscription.activated`
- `subscription.renewed`
- `subscription.recovered`
- `subscription.created`

This happens in `src/webhooks/processor/event_handlers.rs`.

For the observed Creem payloads, both `checkout.completed` and `subscription.paid` still had subscription status `trialing`, but Bridge ended with `status = active` for `sub_72m8alccxoHznvp4M5PJxW`.

### Problem

Bridge ignores the provider subscription status in the generic activation path. A Creem subscription can emit `checkout.completed` or `subscription.paid` while the subscription lifecycle status remains `trialing`.

This causes Bridge to convert a trial into an active paid subscription state too early.

### Impact

- Bridge subscription state no longer reflects Creem lifecycle state.
- HiHa receives `subscription.activated` callbacks and marks premium, with callback status `active` for some trial events.
- Cancellation handling then retains access until `current_period_end`, which may be correct for entitlement but confusing because the stored state was not trial.

### Parity Note

Old HiHa preserved normalized provider status when activating/updating the subscription. Bridge currently hard-codes the activation handler to `active`, which is a Creem trial-flow drift.

### Fix Direction

For Creem activation-like events, normalize and preserve `ctx.fields.status`:

- `trialing` should commit subscription status `Trialing` (or `trial`).
- `active`/`paid` should commit status `active`.
- The callback event to HiHa MUST use `subscription.activated` to properly grant access, but the callback payload `status` must accurately indicate `Trialing`.

## Bug 3: Duplicate Creem Subscriptions For Same User/Product Are Allowed

### Current Behavior

Bridge has two subscription rows for the same app/user/provider/product:

- Same `app_id`: `43bd7125-87eb-4136-9605-6c5e524f1ab0`
- Same `external_user_id`: `user_36lLgcNtpsqKzB5hpan8wYIN5ew`
- Same provider: `creem`
- Same product: `prod_vZ6bj0gDqKefiBW6qrZWj`
- Different Creem subscription IDs

The `checkout_idempotency` table had no row matching this user's checkout response, so checkout creation was not deduped for this flow.

Bridge's subscription uniqueness model is centered on provider `subscription_id`, which correctly prevents exact webhook replay duplicates but does not prevent a user from creating multiple live Creem subscriptions for the same product.

### Problem

Creem issued two separate subscription objects. Bridge accepted both as valid subscriptions for the same user/product. One was later cancelled, but the older one remained active.

This is not webhook idempotency failure; webhook event IDs were unique, and provider subscription IDs were different.

### Impact

- A user can accumulate multiple Creem subscriptions for the same Bridge product.
- Payment rows and subscription state become confusing for support/admin flows.
- Cancelling one subscription does not necessarily cancel the other active subscription.
- HiHa may receive multiple activation callbacks for what the user experiences as one free-trial signup attempt.

### Fix Direction

Add a product/user-level guard for Creem subscription checkout and webhook handling:

- **Block Checkout**: Prevent checkout creation if the user already has an active or trialing subscription for the same app/user/product.
- **Admin Conflict on Webhook**: If a duplicate subscription arrives via webhook (e.g., race condition), do not automatically cancel it. Accept it to preserve provider state, but surface an admin conflict for manual review.
- Preserve the existing provider webhook idempotency model; this bug is about semantic duplicate subscriptions, not duplicate provider event delivery.

## Open Questions

- Should a Creem trial callback to HiHa use event type `subscription.activated` with status `trial`, or a distinct callback event type?
  - **Answer**: It should use `subscription.activated` to grant access, but with `status` set to `Trialing` (the actual Creem status) to avoid overwriting the trial state.
- Should Bridge record a zero-amount payment row for trial invoices, or skip payment rows until actual money is collected?
  - **Answer**: Bridge should record a zero-amount payment row for trial invoices.
- What is the intended policy when Creem creates a second subscription for the same user/product: block checkout, cancel the older subscription, cancel the newer subscription, or keep both but surface an admin conflict?
  - **Answer**: The primary defense should be to **block checkout creation** if the user already has an active or trialing subscription for that product. If a duplicate still bypasses this and arrives via webhook (e.g., due to a race condition), Bridge should accept it to maintain an accurate record of provider state, but surface an admin conflict rather than automatically cancelling, allowing support to handle potential refunds appropriately.
