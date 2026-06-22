# Bridge Admin Clerk Security Audit

Audit date: 2026-06-22  
Baseline commit: `05000c90245cbb4457135e6fc5a820bb57e2b992`  
Scope: admin area and Clerk-backed admin API changes introduced after the baseline commit, with emphasis on login, authorization, and admin auditability.

## Executive Summary

The admin page being public is not the main issue; a Clerk publishable key is public by design, and the HTML can be served without authentication if all sensitive data/actions are protected server-side.

This audit assumes the intended deployment model is a **dedicated Bridge-admin Clerk instance with exactly one admin user**. Under that model, user-vs-admin authorization and Clerk organization membership are not the primary risks right now.

The admin API had one remaining correctness gap worth fixing before relying on it for production payment operations. This is now addressed in the working tree:

1. Manual webhook retry no longer re-opens an already forwarded delivery.

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

### P1 — Manual webhook retry can race and re-open an already forwarded delivery

Status: fixed in the working tree.

Original finding: the retry handler read the delivery, checked `forwarded`, and then later reset the delivery. The reset SQL unconditionally set `forwarded = false`.

Original evidence:

- `src/handlers/admin.rs:151-164` checks whether the delivery is already forwarded.
- `src/db/webhooks.rs:187-203` resets `forwarded`, `forwarded_at`, attempt counts, and dead-letter fields without `forwarded = false` in the `WHERE` clause.

Original impact:

If the background worker forwards the delivery between the handler's read and reset, the admin retry can mark an already-forwarded delivery as pending again, creating duplicate delivery risk.

Recommended fix:

- Make the reset conditional and atomic:
  - `UPDATE ... WHERE id = $1 AND forwarded = false`, or
  - transaction with `SELECT ... FOR UPDATE`, then check `forwarded` under lock.
- Return whether a row was actually reset.
- Audit both successful and skipped retry attempts.
- Consider allowing manual reset only for dead-lettered deliveries, not all pending deliveries.

Implemented fix:

- `reset_webhook_delivery` now performs one conditional `UPDATE` with `WHERE` predicates for the delivery `id`, resolved `app_id`, `forwarded = false`, and `dead_lettered = true`.
- The reset no longer writes `forwarded = false` or clears `forwarded_at`.
- The reset returns whether a row was actually queued, and the admin handler logs both queued and skipped attempts.
- The admin UI only exposes manual retry for dead-lettered, unforwarded deliveries.
