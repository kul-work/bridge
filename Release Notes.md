# Release Notes

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
