# Bridge Documentation Index

Navigate Bridge's architecture, setup, integration guides, provider references, and test plans.

## Quick Start Path

**New to Bridge?** Follow this order:

1. **[README.md](../README.md)** - Project overview, tech stack, quick-start commands
2. **[CONFIGURATION.md](./CONFIGURATION.md)** - Runtime env vars, DB-backed app/provider config
3. **[DB_ONBOARDING.md](./DB_ONBOARDING.md)** - Database setup and migrations
4. **[DESIGN.md](../DESIGN.md)** - Architectural decisions and component interactions
5. **[API_CONTRACT.md](./API_CONTRACT.md)** - App-facing API endpoints, payloads, callbacks, and errors
6. **[WEBHOOK_ARCHITECTURE.md](./WEBHOOK_ARCHITECTURE.md)** - Webhook ingress, processing, and delivery
7. Payment Provider Guides - [Google Play](./google/) and [Creem](./creem/) specifics
8. **[BEHAVIORAL_SPEC.md](./BEHAVIORAL_SPEC.md)** - Detailed procedural flows for every Bridge action
9. **[TROUBLESHOOTING.md](./TROUBLESHOOTING.md)** - Operational debugging and common failure modes
10. **[CONTRIBUTING.md](../CONTRIBUTING.md)** - Build, test, and PR guidelines for contributors
11. **[SECURITY.md](../SECURITY.md)** - Vulnerability reporting and operator hardening notes
---

## Full Documentation Map

### Core Concepts

| Document | Purpose | Audience |
|----------|---------|----------|
| [README.md](../README.md) | Project overview, API snapshot, prerequisites | Everyone |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Local development, tests, PR expectations | Contributors |
| [SECURITY.md](../SECURITY.md) | Private vulnerability reporting, production hardening | Operators, security researchers |
| [DESIGN.md](../DESIGN.md) | Architectural principles, component design, database schema | Developers, Architects |
| [INVARIANTS.md](../INVARIANTS.md) | Behavioral guarantees, constraints, invariant rules | Developers implementing features |
| [BEHAVIORAL_SPEC.md](./BEHAVIORAL_SPEC.md) | Detailed procedural flows for every Bridge action | Developers, Auditors |
| [API_CONTRACT.md](./API_CONTRACT.md) | Implemented app-facing API contract, callback payloads, error format, rate limits | App integrators, Backend developers |
| [LICENSE](../LICENSE) | MIT license | Everyone |

### Setup & Operations

| Document | Purpose | Audience |
|----------|---------|----------|
| [CONFIGURATION.md](./CONFIGURATION.md) | Runtime environment variables, DB-backed app/provider configuration | DevOps, Local setup, Integrators |
| [DB_ONBOARDING.md](./DB_ONBOARDING.md) | PostgreSQL setup, roles, RLS, migrations | DevOps, Local setup |
| [db-install-roles-rls.sql](./db-install-roles-rls.sql) | SQL script for database roles and RLS policies | DevOps, Database admins |
| [db-install-roles-rls.demo.sql](./db-install-roles-rls.demo.sql) | Demo/sample SQL for database roles and RLS setup | DevOps, Local demos |
| [TROUBLESHOOTING.md](./TROUBLESHOOTING.md) | Common debugging paths and operational fixes | Developers, Operators |

### Webhooks & Integration

| Document | Purpose | Audience |
|----------|---------|----------|
| [WEBHOOK_ARCHITECTURE.md](./WEBHOOK_ARCHITECTURE.md) | Webhook ingress flow, deduplication, processor logic, callback delivery | Developers, Integration engineers |

### Payment Providers

#### Google Play

| Document | Purpose | Status |
|----------|---------|--------|
| [google/GOOGLE_PLAY_BILLING_TESTPLAN.md](./google/GOOGLE_PLAY_BILLING_TESTPLAN.md) | Google Play billing acceptance scenarios | Active provider |
| [google/GOOGLE_PLAY_BILLING_DEFERRALS.md](./google/GOOGLE_PLAY_BILLING_DEFERRALS.md) | Deferred Google Play features and rationale | Planning reference |
| [google/GOOGLE_PLAY_ONE-TIME_LIFECYCLE-v1.1.md](./google/GOOGLE_PLAY_ONE-TIME_LIFECYCLE-v1.1.md) | Google Play one-time purchase lifecycle reference | Provider reference |
| [google/GOOGLE_PLAY_SUBSCRIPTION_LIFECYCLE-v1.1.md](./google/GOOGLE_PLAY_SUBSCRIPTION_LIFECYCLE-v1.1.md) | Google Play subscription lifecycle reference | Provider reference |

#### Creem

| Document | Purpose | Status |
|----------|---------|--------|
| [creem/CREEM_BILLING_TESTPLAN.md](./creem/CREEM_BILLING_TESTPLAN.md) | Creem billing acceptance scenarios | Active provider |
| [creem/CREEM_BILLING_TEST_FLOWS.md](./creem/CREEM_BILLING_TEST_FLOWS.md) | Condensed Creem end-to-end test flow narratives | Active provider |
| [creem/CREEM_ONE-TIME_LIFECYCLE-v1.0.md](./creem/CREEM_ONE-TIME_LIFECYCLE-v1.0.md) | Creem one-time payment lifecycle reference | Provider reference |
| [creem/CREEM_SUBSCRIPTION_LIFECYCLE-v1.0.md](./creem/CREEM_SUBSCRIPTION_LIFECYCLE-v1.0.md) | Creem subscription lifecycle reference | Provider reference |

### Testing & Acceptance

| Document | Purpose | Audience |
|----------|---------|----------|
| [testing/BRIDGE_ADMIN_TESTPLAN.md](./testing/BRIDGE_ADMIN_TESTPLAN.md) | Admin retry, scheduler trigger, admin auth, CSP, and audit acceptance scenarios | Developers, Operators |
| [testing/BRIDGE_CONTRACT_TESTPLAN.md](./testing/BRIDGE_CONTRACT_TESTPLAN.md) | Cross-app isolation and Bridge contract acceptance scenarios | Developers, App integrators |

---

## Common Tasks

### "How do I...?"

- **Set up Bridge locally?** -> [CONFIGURATION.md](./CONFIGURATION.md) + [DB_ONBOARDING.md](./DB_ONBOARDING.md) + [README.md](../README.md#quickstart)
- **Integrate an app with Bridge?** -> [API_CONTRACT.md](./API_CONTRACT.md)
- **Understand how webhooks work?** -> [WEBHOOK_ARCHITECTURE.md](./WEBHOOK_ARCHITECTURE.md)
- **Add a new payment provider?** -> [DESIGN.md (Provider Abstraction)](../DESIGN.md#provider-abstraction) + provider folder
- **Verify a webhook signature?** -> [google/](./google/) or [creem/](./creem/) provider docs
- **Understand subscription state?** -> [DESIGN.md (Subscription Lifecycle)](../DESIGN.md)
- **Know what Bridge guarantees?** -> [INVARIANTS.md](../INVARIANTS.md)
- **Need step-by-step logic for a flow?** -> [BEHAVIORAL_SPEC.md](./BEHAVIORAL_SPEC.md)
- **Run acceptance checks?** -> [testing/](./testing/) + provider test plans
- **Debug local or production issues?** -> [TROUBLESHOOTING.md](./TROUBLESHOOTING.md)

---

## Document Relationships

```text
README.md (overview)
    |
CONFIGURATION.md (runtime/app config)
    |
DB_ONBOARDING.md (database setup)
    |
DESIGN.md (architecture)
    |-- API_CONTRACT.md (app-facing API)
    |-- WEBHOOK_ARCHITECTURE.md (webhook details)
    |-- google/ (provider specifics)
    |-- creem/ (provider specifics)
    `-- testing/ (cross-cutting acceptance plans)

INVARIANTS.md (cross-cutting constraints)
BEHAVIORAL_SPEC.md (detailed procedural flows)
TROUBLESHOOTING.md (debugging and operations)
```

---

## Reference Files

- **Cargo.toml** - Dependencies, feature flags
- **migrations/** - PostgreSQL schema changes (ordered by timestamp)
- **AGENTS.md** - Developer guidelines, code style, constraints
