# Bridge Documentation Index

Navigate Bridge's architecture, setup, and integration guides.

## Quick Start Path

**New to Bridge?** Follow this order:

1. **[README.md](../README.md)** - Project overview, tech stack, quick-start commands
2. **[CONFIGURATION.md](./CONFIGURATION.md)** - Runtime env vars, DB-backed app/provider config
3. **[DB-ONBOARDING.md](./DB-ONBOARDING.md)** - Database setup and migrations
4. **[DESIGN.md](../DESIGN.md)** - Architectural decisions and component interactions
5. **[WEBHOOK_ARCHITECTURE.md](./WEBHOOK_ARCHITECTURE.md)** - Webhook ingress, processing, and delivery
6. **Payment Provider Guides** - Provider-specific details (Google Play, Creem)

---

## Full Documentation Map

### Core Concepts

| Document | Purpose | Audience |
|----------|---------|----------|
| [README.md](../README.md) | Project overview, API snapshot, prerequisites | Everyone |
| [DESIGN.md](../DESIGN.md) | Architectural principles, component design, database schema | Developers, Architects |
| [INVARIANTS.md](../INVARIANTS.md) | Behavioral guarantees, constraints, invariant rules | Developers implementing features |

### Setup & Operations

| Document | Purpose | Audience |
|----------|---------|----------|
| [CONFIGURATION.md](./CONFIGURATION.md) | Runtime environment variables, DB-backed app/provider configuration | DevOps, Local setup, Integrators |
| [DB-ONBOARDING.md](./DB-ONBOARDING.md) | PostgreSQL setup, roles, RLS, migrations | DevOps, Local setup |
| [db-install-roles-rls.sql](./db-install-roles-rls.sql) | SQL script for database roles and RLS policies | DevOps, Database admins |

### Webhooks & Integration

| Document | Purpose | Audience |
|----------|---------|----------|
| [WEBHOOK_ARCHITECTURE.md](./WEBHOOK_ARCHITECTURE.md) | Webhook ingress flow, deduplication, processor logic, callback delivery | Developers, Integration engineers |

### Payment Providers

| Folder | Purpose | Status |
|--------|---------|--------|
| [google/](./google/) | Google Play integration, signature verification, event mapping | Active provider |
| [creem/](./creem/) | Creem integration, HMAC validation, state normalization | Active provider |

---

## Common Tasks

### "How do I...?"

- **Set up Bridge locally?** -> [CONFIGURATION.md](./CONFIGURATION.md) + [DB-ONBOARDING.md](./DB-ONBOARDING.md) + [README.md](../README.md#quickstart)
- **Understand how webhooks work?** -> [WEBHOOK_ARCHITECTURE.md](./WEBHOOK_ARCHITECTURE.md)
- **Add a new payment provider?** -> [DESIGN.md (Provider Abstraction)](../DESIGN.md#provider-abstraction) + provider folder
- **Verify a webhook signature?** -> [google/](./google/) or [creem/](./creem/) docs
- **Understand subscription state?** -> [DESIGN.md (Subscription Lifecycle)](../DESIGN.md)
- **Know what Bridge guarantees?** -> [INVARIANTS.md](../INVARIANTS.md)

---

## Document Relationships

```text
README.md (overview)
    |
CONFIGURATION.md (runtime/app config)
    |
DB-ONBOARDING.md (database setup)
    |
DESIGN.md (architecture)
    |-- WEBHOOK_ARCHITECTURE.md (webhook details)
    |-- google/ (provider specifics)
    `-- creem/ (provider specifics)

INVARIANTS.md (cross-cutting constraints)
```

---

## Reference Files

- **Cargo.toml** - Dependencies, feature flags
- **migrations/** - PostgreSQL schema changes (ordered by timestamp)
- **AGENTS.md** - Developer guidelines, code style, constraints
