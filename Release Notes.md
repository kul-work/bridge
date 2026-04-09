# Release Notes

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

- Multi-provider payment processing (Google Play, Creem, LemonSqueezy, Coinbase)
- Checkout flow with session management
- Purchase verification
- Subscription management
- PostgreSQL backend with migrations
- API key authentication
- Provider-specific integrations
