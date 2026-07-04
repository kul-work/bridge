# Release Notes

## [0.6.0] - 2026-07-02
### Features
- **API Keys**: Add an API key handler path for app credential management flows.
- **Scanner Routes**: Add an internal Swagger route index and OpenAPI spec for scanner/manual route discovery, guarded behind explicit opt-in.
- **Scanner Routes**: Add a plain text route listing at `/routes/plain` for quick manual importing into Burp Suite or other security tools.
- **Creem Tests**: Add automated Creem bridge subscription access scripts for staging/regression checks.

### Fixes
- **Configuration**: Add production startup validation for Google Play RSA bypass, clerk authentication, and audience verification.
- **Configuration**: Preserve Clerk issuer fallback during startup validation.
- **Error Handling**: Sanitize API error responses to prevent database and provider detail exposure.
- **Lifecycle Guards**: Reject resume/cancel actions on terminal subscriptions, enforce strict monotonic transitions, and deduplicate processed webhooks.
- **Google Play Identity**: Resolve Google Play subscription lifecycles strictly by purchase token to eliminate SKU fallbacks.
- **Google Play Security**: Encode outbound API path segments and harden Google Play mock URL handling.
- **Security**: Force webhook signature verification outside mock mode, and redact PII in dispute emails by hashing user IDs and hiding customer emails.
- **Payments**: Explicitly handle unknown amount and currency defaults as -1 and UNKNOWN.
- **Payments**: COALESCE nullable ACK placeholder amount/currency reads to avoid NULL decode crashes.
- **Webhook Atomicity**: Provider ACK now waits for durable enqueue, callback payloads persist before forwarding, worker side effects are claim-fenced, and lifecycle emails send only after webhook commit.
- **Webhook Adapter**: Introduce the provider webhook adapter boundary for normalized provider ingress handling.
- **Subscriptions**: Constrain canonical subscription status writes and fence Google price step-up decline actions.
- **Row Level Security**: Enforce RLS context app-scoping across all background jobs and implement claim-token fencing in the atomic processor.
- **Database & Schema**: Migration of `payments.amount_cents` and `subscriptions.google_new_price_cents` to `bigint` to satisfy Money design invariants.
- **Database & Schema**: Isolate business-logic schema migrations from RLS policy migrations.
- **Docker**: Add Docker ignore handling and certificate/runtime improvements.
- **Documentation**: Clarify `bridge_admin` `BYPASSRLS` database role requirements for Admin Dashboard data visibility.

### Tests and docs
- **Google Play Regression**: Add same-SKU cross-user identity regression coverage and OTP-06 missing-price ACK row coverage.
- **Webhook Regression**: Add retry-action coverage for unprocessed delivery rows and stabilize subscription webhook suite checks.
- **Documentation**: Update Google Play billing test plan, DB onboarding, provider adapter notes, webhook architecture, and behavioral specs for the new hardening.

## [0.5.2] - 2026-06-25
### Fixes
- **Webhook Tokens**: Add expiry handling for generated webhook tokens.

### Improvements
- **Documentation**: Add architectural review notes.

## [0.5.1] - 2026-06-24
### Features
- **Admin Alerts**: Add an admin dashboard surface for alert monitoring.

### Fixes
- **Webhooks**: Persist delivery attempt updates before emitting retry/dead-letter alert signals so alert logs match stored delivery state.
- **Webhooks**: Structure forwarding outcome logs for terminal skips, stale suppression, duplicate queues, and resumed forwarding while preserving PII-safe identifiers.

### Improvements
- **Observability**: Add structured request, provider, webhook, subscription action, admin job, email, and checkout diagnostics with hashed external user and recipient identifiers.
- **Alerts**: Add alert signal logging and follow-up alert documentation cleanup.
- **Documentation**: Reorganize and refresh documentation indexes.

## [0.5.0] - 2026-06-23
### Features
- **Observability**: Add PII-safe request, provider, webhook, and background job diagnostics with structured correlation fields.
- **App Verification**: Add protected app identity verification for API key slug checks and callback-secret HMAC proof validation.
- **Admin Webhooks**: Add webhook pagination for the admin dashboard.

### Fixes
- **Readiness**: Make provider readiness checks RLS-safe through a security-definer bootstrap count.
- **Admin Testing**: Require explicit local admin test env loading and keep production/staging from reading test env files.
- **Observability**: Scrub PII from provider/email/log diagnostics and clarify hashed webhook receipt log fields.
- **Google Pub/Sub**: Use provider DB audience configuration when the environment fallback is unset.

### Improvements
- **Testing**: Add ADMIN, CTI, GPBI, Creem, and dead-letter/admin retry coverage.
- **Documentation**: Refresh observability, admin test env, and Bridge check guidance.

## [0.4.0] - 2026-06-22
### Features
- **Admin Dashboard**: Add a minimal Clerk-protected admin interface with notes, webhook actions, and manual job triggers.
- **Webhooks**: Add manual retry support for dead-lettered deliveries through the admin flow.

### Fixes
- **Admin Security**: Harden Clerk admin auth, CSP, security headers, authorized-party checks, token handling, audit logging, and admin rate limits.
- **Webhooks**: Prevent manual retry from reopening already-forwarded deliveries and make dead-letter retry reset atomic.
- **Security Testing**: Add cross-app tenant isolation coverage and tighten RLS/security regression tests.

### Improvements
- **Documentation**: Refresh admin, webhook retry, and security audit notes for the remediated findings.

## [0.3.4] - 2026-06-19
### Fixes
- **Security**: Harden request deserialization and Creem API URL validation, including mock-only Google Play test price header handling.
- **RLS/Auth**: Tighten bootstrap reads, runtime privileges, API-key/webhook-token lookup paths, and app-scoped admin/scheduler access.
- **Testing**: Align GPBI purchase/register scripts with strict request payload validation.

### Improvements
- **Database**: Consolidate baseline migrations and add internal app notes support.
- **Deployment**: Add Docker image support and environment-based Google Play service account credential loading.
- **Documentation**: Refresh hardened deployment, RLS audit, API contract, and security notes.

## [0.3.3] - 2026-06-02
### Fixes
- **Creem**: Block duplicate subscription checkout, resolve checkout product selectors, and avoid sending unsupported checkout cancel URLs.
- **Creem**: Correct trial invoice amount/currency extraction and treat subscription lifecycle object IDs as state-only unless a real transaction ID is provided.
- **Subscriptions**: Revoke Creem subscriptions when refunded payments arrive and pass cancellation `onExecute` through to Creem.

### Improvements
- **Agent Checks**: Add Bridge payment review checkers and release-loop skills for safer provider changes.
- **Documentation**: Refresh release-loop, checker, and Creem fix notes.

## [0.3.2] - 2026-05-24
### Features
- **Google Play**: Track ordinary pending price changes from v2 `priceChangeDetails` and expose them in callbacks and subscription APIs.
- **Fraud Prevention**: Record Google Play obfuscated account IDs during verified purchases.

### Fixes
- **Google Play**: Reject invalid mock subscription verifications before committing state.
- **Google Play**: Record free trial subscriptions as zero-value phases.
- **Google Play**: Persist payment and OTP currencies from provider data, including refunds and price step-up rows.
- **Google Play**: Preserve renewal webhook/payment records and separate purchase tokens from transaction IDs.
- **Google Play**: Align OTP refund handling with purchase-token invariants and deduplicate OTP verify callbacks.
- **Webhooks**: Make delivery enqueue idempotent and improve Google Pub/Sub audience/test notification handling.
- **Subscriptions**: Clear pending Google price-change fields on terminal states.

### Improvements
- **Logging**: Improve webhook and provider diagnostics.
- **Documentation**: Refresh Google Play price-change, OTP, and parity notes.
- **Testing**: Expand Google Play renewal, OTP, currency, and price-change coverage.

## [0.3.1] - 2026-05-14
### Features
- **Google Play**: Handle pending one-time purchase verification with 202 responses and OTP-04 replay/polling coverage.

### Fixes
- **Google Play**: Enforce acknowledgement lifecycle for subscriptions and one-time products, including mock-mode-safe retry handling.
- **Subscriptions**: Preserve provider period end during reconciliation and avoid stale lifecycle downgrades.
- **Payments**: Prevent approved OTP payments from being downgraded by later pending verification retries.

### Improvements
- **Testing**: Expand provider callback, Google lifecycle snapshot, and provider status change coverage.
- **Documentation**: Refresh API contract, configuration, billing test plan, and architecture notes.

## [0.3.0] - 2026-05-09
### Features
- **Lifecycle Emails**: Wire email dispatch for paused, resumed, and refunded events with retry logic and transient lookup recovery.
- **Subscriptions**: Expand list API with full lifecycle fields; expose lifecycle status snapshots.
- **Webhooks**: Improve callback forwarding diagnostics; send lifecycle emails from Bridge; support verify_webhook_signature config with test-mode bypass for Creem.
- **Subscriptions**: Add Google Play renewal period extension logic.
- **Outbound Config**: Configurable HTTP timeouts for provider outbound calls.

### Fixes
- **Google Play**: Fix price step-up consent flow; align RTDN webhook event mappings; deduplicate subscription lifecycle outcomes; simplify JWT verification.
- **Creem**: Honor requested checkout product IDs; map refund.created to payment.refunded; handle payment.failed and payment.partially_refunded for OTP; make stale payment window configurable per-app.
- **Webhooks**: Scope subscription/payment lookups and transitions by provider; make transitions user-aware; handle Creem webhook token edge cases; resume stranded duplicate provider events; include OTP product id on refund callbacks.
- **Subscriptions**: Validate cancellation mode before provider API call; move user ownership check into DB query layer; clean up pending subscriptions on linking_required responses.
- **Payments**: Replace f64 arithmetic with integer-cent parsing and formatting; replace silent Expired fallback with Unknown(String) variant.
- **Users**: Move provider cancellation out of DB anonymization.
- **Checkout**: Validate email format and length before forwarding to providers.

### Improvements
- **Code Quality**: Consolidate subscription lookup queries; remove LemonSqueezy provider and dead code; improve error handling in email lookups.
- **Testing**: Expand test coverage for Creem and Google Play integrations; fix test suite alignment.
- **Documentation**: Improve architecture, audit, and test plan documentation.

### Removed
- **Coinbase**: Removed Coinbase provider and legacy code.

## [0.2.0] - 2026-04-09
### Breaking Changes
- **Architecture**: Complete migration to Hexagonal Architecture (Ports and Adapters) for improved testability and provider isolation.
- **Security**: Removed `ENCRYPTION_KEY` and `MASTER_ENCRYPTION_KEY` in favor of simplified security model.
- **Environment**: Removed `APP_URL` and `CORS` configuration requirements.

### Features
- **Architecture**: Ported scheduler, Google Play lifecycle, and webhook ingress to new port-driven logic.
- **Testing**: Added initial GPBI (Google Play Billing Infrastructure) test suite.
- **RLS**: Implemented and consolidated Row-Level Security (RLS) migrations.
- **Models**: Added support for Gemini 3 Flash specifications.

### Fixes
- **Behavioral Gaps**: Resolved extensive behavioral discrepancies across all payment providers.
- **Security**: Scoped subscription action writes strictly to the active application context via RLS.
- **Google Play**: Fixed certificate reading logic and purchase verification edge cases.
- **Scheduler**: Stabilized background task processing with port-based abstractions.

## [0.1.2] - 2026-03-28
- Empty bump version

## [0.1.1] - 2026-03-28
- Several gap fixes

## [0.1.0] - 2026-03-23

Initial Bridge payment gateway implementation with:

- **Webhooks**: Full webhook ingress, processing, and forwarding pipeline with deduplication and event ordering
- **Admin Dashboard**: HTML dashboard for monitoring apps, webhooks, and failed deliveries
- **Admin API**: REST endpoints for apps list, webhooks per app, and manual retry
- **HMAC Signing**: Secure webhook callbacks to apps with SHA256 signatures
- **Event Normalization**: Canonical event types across all payment providers
- **Retry Logic**: Exponential backoff (0s, 5m, 10m) for webhook delivery failures
- **Database**: Complete webhook tracking tables with audit trail

- Multi-provider payment processing (Google Play, Creem, Coinbase)
- Checkout flow with session management
- Purchase verification
- Subscription management
- PostgreSQL backend with migrations
- API key authentication
- Provider-specific integrations
