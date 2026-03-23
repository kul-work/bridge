# Release Notes

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
