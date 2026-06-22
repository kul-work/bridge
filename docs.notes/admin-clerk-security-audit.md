# Bridge Admin Clerk Security Audit

Audit date: 2026-06-22  
Baseline commit: `05000c90245cbb4457135e6fc5a820bb57e2b992`  
Scope: admin area and Clerk-backed admin API changes introduced after the baseline commit, with emphasis on login, authorization, and admin auditability.

## Executive Summary

The admin page being public is not the main issue; a Clerk publishable key is public by design, and the HTML can be served without authentication if all sensitive data/actions are protected server-side.

This audit assumes the intended deployment model is a **dedicated Bridge-admin Clerk instance with exactly one admin user**. Under that model, user-vs-admin authorization and Clerk organization membership are not the primary risks right now.

The current admin API still has a small set of security and correctness gaps worth fixing before relying on it for production payment operations:

1. `ADMIN_CLERK_AUTHORIZED_PARTIES`, if configured, is bypassable when JWTs omit `azp`.
2. Manual webhook retry can race and re-open an already forwarded delivery.
3. Admin UI browser hardening has a few low-priority gaps.

## Scope Reviewed

- `src/middleware/admin_auth.rs`
- `src/handlers/admin.rs`
- `src/main.rs`
- `templates/admin.html`
- `src/db/webhooks.rs`
- `.env.sample`
- admin-related database changes in the diff from the baseline commit

## Deployment Assumption

The findings below are scoped to the current plan: Bridge admin uses a dedicated Clerk instance with one admin user. If Bridge later shares a Clerk instance with app users, enables public signup for the admin Clerk instance, or adds multiple admins/operators, the discarded org/role concerns should be reopened and treated as higher severity.

## Findings

### P1 — `ADMIN_CLERK_AUTHORIZED_PARTIES` is bypassable when `azp` is missing

If `ADMIN_CLERK_AUTHORIZED_PARTIES` is configured, the verifier checks `azp` only when the claim exists. A token with no `azp` claim is accepted.

Evidence:

- `src/middleware/admin_auth.rs:96-99` loads allowed authorized parties from env.
- `src/middleware/admin_auth.rs:203-210` validates `azp` only inside `if let Some(azp)`.

Impact:

The authorized-party allowlist is not fail-closed. If a token from the same issuer omits `azp`, it bypasses this restriction.

Recommended fix:

- If `ADMIN_CLERK_AUTHORIZED_PARTIES` is non-empty, require the `azp` claim.
- Reject tokens whose normalized `azp` is not in the allowlist.
- Trim/filter CSV parts before URL normalization; blank entries should not normalize into `https://`.
- Consider adding and verifying a dedicated `aud` claim for the Bridge admin API.

### P1 — Manual webhook retry can race and re-open an already forwarded delivery

The retry handler reads the delivery, checks `forwarded`, and then later resets the delivery. The reset SQL unconditionally sets `forwarded = false`.

Evidence:

- `src/handlers/admin.rs:151-164` checks whether the delivery is already forwarded.
- `src/db/webhooks.rs:187-203` resets `forwarded`, `forwarded_at`, attempt counts, and dead-letter fields without `forwarded = false` in the `WHERE` clause.

Impact:

If the background worker forwards the delivery between the handler's read and reset, the admin retry can mark an already-forwarded delivery as pending again, creating duplicate delivery risk.

Recommended fix:

- Make the reset conditional and atomic:
  - `UPDATE ... WHERE id = $1 AND forwarded = false`, or
  - transaction with `SELECT ... FOR UPDATE`, then check `forwarded` under lock.
- Return whether a row was actually reset.
- Audit both successful and skipped retry attempts.
- Consider allowing manual reset only for dead-lettered deliveries, not all pending deliveries.

