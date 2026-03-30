# Bridge Behavioral Gaps (vs BEHAVIORAL_SPEC.md)

Based on a review of the current Bridge codebase against the `BEHAVIORAL_SPEC.md`, most of the core logic, background jobs, and webhook mappings have been successfully implemented. However, the following behavioral gaps and discrepancies were identified:

## 1. Purchase Registration Endpoint Route (§6)
- **Spec Requirement:** Defines the endpoint as `POST /api/v1/purchase/register` (singular).
- **Current Implementation:** In `src/main.rs`, the route is implemented as `/purchases/register` (plural: `axum::routing::post(handlers::payments::register_purchase)`).

## 2. Missing User Lookup Strategy for Google Play (§53)
- **Spec Requirement:** `Strategy 3` defines resolving users via `obfuscated_account_id` for Google Play (`Google API verify_token → extract obfuscated_id → SELECT external_user_id FROM subscriptions...`).
- **Current Implementation:** In `src/webhooks/processor.rs::resolve_user`, this step is entirely missing. The resolution cascade skips directly from `Strategy 2` (`purchase_token` lookup) to `Strategy 4` (Creem `metadata.user_id` extraction).

## 3. Missing Creem Stale Fix on Activation (§13)
- **Spec Requirement:** Under Subscription Activation, Step 5 specifies: *"Creem stale fix: if Creem AND no provider_transaction_id → call adopt_stale_payment() to merge old records with mismatched transaction IDs."*
- **Current Implementation:** In `src/webhooks/processor.rs`, the handler for `"subscription.activated"` simply calls `record_payment_tx` via `fields.provider_transaction_id`. There is no logic or function call to `adopt_stale_payment()` to recover orphaned or mismatched Creem records.

## 4. Missing Google Play Post-Commit Resubscribe/Upgrade Linking (§13)
- **Spec Requirement:** Under Subscription Activation, Step 8 specifies: *"Post-commit: Resubscribe/Upgrade linking (Google Play only): If purchase_token present → detect upgrades/downgrades by checking existing active subscriptions for this user. If old active subscription found with different subscription_id → mark it as replaced."*
- **Current Implementation:** The handler in `src/webhooks/processor.rs` performs the database upsert and commits the transaction, but exits immediately after. It does not apply upgrade/downgrade linking markers (such as setting the old subscription to `"replaced"`).
