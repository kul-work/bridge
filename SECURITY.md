# Security Policy

Bridge handles payment state, provider webhooks, and multi-tenant app isolation. Treat security reports seriously and keep sensitive details out of public channels until a fix is available.

## Supported versions

Security fixes target the default branch (`main` / current development line). If you run a fork or an older tag, please still report issues — maintainers will indicate whether a backport is planned.

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Prefer one of:

1. **GitHub Security Advisories** — “Report a vulnerability” on this repository (private advisory), if enabled.
2. **Private contact** — open a draft security advisory or contact the repository maintainers through the project’s GitHub organization / owner profile so the report stays non-public.

Include as much as you can:

- affected version / commit SHA
- component (e.g. webhook ingress, admin auth, RLS, API key auth, callback HMAC)
- reproduction steps or a minimal PoC
- impact (auth bypass, cross-app data access, payment state corruption, secret leakage, etc.)
- whether you plan any public disclosure timeline

You should receive an acknowledgment when maintainers are able to respond. Coordination of disclosure (including CVE assignment, if any) happens after a fix is agreed.

## In scope (examples)

- Authentication / authorization bypass (API keys, admin Clerk, webhook ingress)
- Cross-app tenant isolation failures (RLS, missing `app_id` scope)
- Webhook signature / Pub/Sub identity bypass that allows forged provider events
- Callback HMAC weaknesses that allow forged Bridge → app events
- Injection into SQL or unsafe dynamic query construction
- Secrets or PII logged or returned in unexpected places
- Privilege escalation via admin routes or background workers

## Out of scope (typical)

- Denial of service without a concrete, fixable bug (e.g. “send lots of traffic”)
- Issues that require already-compromised production secrets (valid API keys, SA JSON, Clerk secrets)
- Vulnerabilities only in third-party services (Google Play, Creem, Clerk, Resend) with no Bridge misconfiguration path
- Social engineering or physical access scenarios
- Reports against deployments you do not own, without permission

## Safe harbor

We will not pursue legal action against researchers who:

- make a good-faith effort to avoid privacy violations, data destruction, and service disruption
- do not access more data than needed to demonstrate the issue
- report findings promptly and keep them confidential until coordinated disclosure
- do not exploit the issue beyond a minimal proof of concept

## Hardening notes for operators

When self-hosting Bridge in production:

- set `ENVIRONMENT=production` (or `prod`)
- never enable `MOCK_EXTERNAL_APIS`
- use least-privilege `DATABASE_URL` (`bridge_app`) and a separate `ADMIN_DATABASE_URL` for migrations
- require Clerk admin auth (`CLERK_PUBLISHABLE_KEY`, `ADMIN_CLERK_AUTHORIZED_PARTIES`, preferably `ADMIN_CLERK_ORG_ID`)
- keep provider webhook signature verification enabled in `pay.provider_configs`
- store Google Play service-account JSON and provider secrets outside the git tree
- set `ADMIN_ALERT_EMAIL` (or `BRIDGE_SUPPORT_EMAIL`) for dispute / reconciliation alerts

See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) and [INVARIANTS.md](INVARIANTS.md).

## Acknowledgments

Responsible disclosures that lead to fixes may be credited in release notes if the reporter wants attribution.
