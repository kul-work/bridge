# Bridge — Multi-App Payment Gateway

**Bridge** is an open-source payment middleware service. It owns subscription lifecycle, payment recording, provider webhooks, and secure callback delivery so your product backends do not have to.

Apps talk to Bridge over a versioned HTTP API. Bridge talks to billing providers (Google Play, Creem today). Normalized events flow back to each app’s callback URL.

## Why Bridge

- **Decouple payments from product logic** — apps keep users and features; Bridge keeps money state.
- **Multi-app by design** — one Bridge deployment serves many apps with per-app API keys, provider config, and webhook paths.
- **Idempotent webhooks** — provider ingress is deduplicated and ordered before state changes.
- **Least privilege** — opaque `external_user_id`s, hashed API keys, HMAC-signed app callbacks, PostgreSQL RLS for tenant isolation.

## Tech stack

| Layer | Choice |
|-------|--------|
| Runtime | Rust, Axum, Tokio |
| Database | PostgreSQL (SQLx) |
| Providers | Google Play Billing, Creem |
| Admin UI | Clerk-authenticated dashboard at `/admin` |

## Documentation

| Doc | Purpose |
|-----|---------|
| [docs/INDEX.md](docs/INDEX.md) | Full documentation map |
| [DESIGN.md](DESIGN.md) | Architecture and component design |
| [INVARIANTS.md](INVARIANTS.md) | Non-negotiable payment/webhook rules |
| [docs/API_CONTRACT.md](docs/API_CONTRACT.md) | App-facing API and callback payloads |
| [docs/CONFIGURATION.md](docs/CONFIGURATION.md) | Environment and DB-backed config |
| [docs/DB_ONBOARDING.md](docs/DB_ONBOARDING.md) | Roles, RLS, onboarding a new app |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Build, test, and PR guidelines |
| [SECURITY.md](SECURITY.md) | Vulnerability reporting |

## Prerequisites

- Rust **1.75+** (CI may pin a newer stable; see `.github/workflows`)
- PostgreSQL **16+** (17 recommended)
- Optional: [sqlx-cli](https://github.com/launchbadge/sqlx) for migrations from the shell

## Quickstart

### 1. Clone and configure

```bash
cp .env.sample .env
```

Minimum local settings:

```env
ENVIRONMENT=development
DATABASE_URL=postgresql://bridge_app:password@localhost/appgen
ADMIN_DATABASE_URL=postgresql://bridge_admin:password@localhost/appgen
PORT=3000
MOCK_EXTERNAL_APIS=true
ENABLE_BACKGROUND_JOBS=true
EMAIL_PROVIDER=mock
```

- `DATABASE_URL` — least-privilege runtime role (`bridge_app`).
- `ADMIN_DATABASE_URL` — elevated role for startup migrations (`bridge_admin`). Required in production.

Create DB roles/schema as described in [docs/DB_ONBOARDING.md](docs/DB_ONBOARDING.md) (or use the SQL helpers under `docs/db-install-roles-rls*.sql`).

### 2. Migrate

```bash
sqlx migrate run --database-url postgresql://bridge_admin:password@localhost:5432/appgen
```

Or with `DATABASE_URL` / `.env` loaded:

```bash
sqlx migrate run
```

Migrations also run automatically at process startup when `ADMIN_DATABASE_URL` (or a non-hardened `DATABASE_URL`) can apply them.

### 3. Run

```bash
cargo run
```

Default listen address: `0.0.0.0:3000` (`SERVER_ADDR` / `PORT`).

Health check:

```bash
curl -s http://127.0.0.1:3000/health
```

### 4. Register an app (DB-driven)

There is no public “create app” HTTP API yet. Onboard apps by inserting rows into `pay.apps`, `pay.provider_configs`, and `pay.api_keys`. See [docs/DB_ONBOARDING.md](docs/DB_ONBOARDING.md).

### 5. Call the API

```http
Authorization: Bearer sk_your_app_key
```

Full contract: [docs/API_CONTRACT.md](docs/API_CONTRACT.md).

## Authentication model

| Direction | Mechanism |
|-----------|-----------|
| App → Bridge | API key (`Authorization: Bearer sk_…`) |
| Bridge → App | HMAC body signature (`X-Pay-Signature`) using the app callback secret |
| Provider → Bridge | Per-app path `/webhooks/{ingress_token}/{provider}` plus provider signature / Pub/Sub JWT checks |

Production must keep provider signature verification enabled. Ingress tokens only route traffic to the correct app; they are not a substitute for cryptographic verification.

## API snapshot

### Subscriptions & payments

- `POST /api/v1/payment/checkout` — start a provider checkout session
- `POST /api/v1/verify-purchase` — verify a mobile purchase
- `GET /api/v1/subscriptions` — list subscriptions for the authenticated app
- `GET /api/v1/subscriptions/:id` — subscription detail
- `POST /api/v1/subscriptions/:id/cancel` — cancel
- `POST /api/v1/subscriptions/:id/resume` — resume
- `POST /api/v1/subscriptions/:id/acknowledge` — Google Play acknowledge
- `POST /api/v1/subscriptions/:id/portal` — billing portal URL
- `POST /api/v1/subscriptions/:id/price-step-up/accept` — accept price step-up
- `POST /api/v1/subscriptions/:id/price-step-up/decline` — decline price step-up
- `GET /api/v1/payments` — payment history
- `POST /api/v1/purchase/register` — register / pre-register a purchase

### User & privacy

- `POST /api/v1/users/:id/anonymize` — GDPR-style anonymization
- `GET /api/v1/users/:id/data-export` — GDPR-style export

### System

- `GET /health` — liveness / diagnostics
- `GET /admin/` — admin dashboard (Clerk)
- `GET /admin/apps` — list apps (auth required)
- `GET /admin/apps/:app_id/webhooks` — webhook log for an app
- `POST /admin/webhooks/:webhook_id/retry` — requeue a dead-lettered delivery

## Administration

The built-in admin UI at `/admin/` (Clerk) is for operators:

- registered apps and provider config visibility
- webhook ingress and delivery status
- manual retry for dead-lettered callbacks
- worker / health oriented monitoring

Local admin setup is documented in [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Design principles (short)

- **Opaque identifiers** — prefer `external_user_id` over storing general end-user PII in Bridge.
- **App-scoped tenancy** — API keys and RLS pin every read/write to one app.
- **Provider abstraction** — normalize provider states into Bridge’s canonical statuses.
- **Idempotency first** — log provider webhooks before mutating subscription/payment state; suppress stale events.

## Development & tests

```bash
cargo check
cargo test --lib
```

Shell integration suites live under `tests/` (admin, Creem, Google Play / GPBI, contract/isolation, security). Each suite has a README. For local provider simulation set `MOCK_EXTERNAL_APIS=true` (rejected in production).

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

Payment middleware is high risk. Please report vulnerabilities privately — see [SECURITY.md](SECURITY.md).

Do not commit real `.env` files, Google Play service-account JSON, API keys, or production webhook secrets.

## License

Bridge is released under the [MIT License](LICENSE).  
Copyright © 2026 Mihaita Nita (Kul).
