# Bridge Admin Clerk Security Audit

Audit date: 2026-06-22  
Baseline commit: `05000c90245cbb4457135e6fc5a820bb57e2b992`  
Scope: admin area and Clerk-backed admin API changes introduced after the baseline commit, with emphasis on login, authorization, and admin auditability.

## Executive Summary

The admin page being public is not the main issue; a Clerk publishable key is public by design, and the HTML can be served without authentication if all sensitive data/actions are protected server-side.

This audit assumes the intended deployment model is a **dedicated Bridge-admin Clerk instance with exactly one admin user**. Under that model, user-vs-admin authorization and Clerk organization membership are not the primary risks right now.

The current admin API still has security and operations gaps worth fixing before relying on it for production payment operations:

1. `ADMIN_CLERK_AUTHORIZED_PARTIES`, if configured, is bypassable when JWTs omit `azp`.
2. Sensitive admin actions have weak server-side provenance if the single admin session is compromised or behaves unexpectedly.
3. Manual webhook retry can race and re-open an already forwarded delivery.
4. Webhook payload viewing and manual job triggering need tighter operational controls.

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

### P2 — Admin action provenance is weak

The middleware validates the JWT and then discards the identity. With the current dedicated one-user Clerk instance, this is not a user-vs-admin authorization problem. The risk is weaker incident/debug provenance: Bridge cannot tie sensitive admin operations to the authenticated Clerk session, IP, user-agent, target object, and result.

Evidence:

- `src/middleware/admin_auth.rs:151-227` has `verify_token` return `Result<(), String>`.
- `src/middleware/admin_auth.rs:413-418` calls `next.run(request)` without inserting a principal into request extensions.
- `src/handlers/admin.rs:61-78` updates app notes with no actor audit.
- `src/handlers/admin.rs:116-135` returns webhook payload data with no actor audit.
- `src/handlers/admin.rs:151-164` logs manual retry details but not the Clerk session/IP/user-agent.
- `src/handlers/admin.rs:207-265` logs manually triggered job names but not the Clerk session/IP/user-agent.

Impact:

Bridge cannot answer basic incident/debug questions such as:

- Was this action performed by the expected Clerk session?
- Which IP/user-agent viewed this webhook payload?
- Which session retried this webhook delivery?
- Which session triggered reconciliation or cleanup?
- What target object was touched, and did the action succeed or fail?

Recommended fix:

- Return `AdminPrincipal` from token verification.
- Insert the principal into request extensions in `admin_auth_middleware`.
- Extract the principal in all admin handlers.
- Add structured provenance/audit events for:
  - `admin.apps.notes.update`
  - `admin.webhook.payload.view`
  - `admin.webhook.retry.request`
  - `admin.jobs.trigger`
- Include at minimum:
  - Clerk `sub`
  - session ID / JWT ID if available
  - method/path
  - target app/webhook/job
  - result success/failure
  - remote IP
  - user-agent
- Structured tracing logs are acceptable as the first step for this one-user internal admin. A durable `pay.admin_audit_log` table is optional unless stronger forensics/compliance is needed.
- Never log raw bearer tokens or raw webhook payloads.

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

### P2 — Webhook payload viewer uses blacklist redaction on arbitrary provider JSON

The payload endpoint fetches stored provider payloads and returns a redacted copy. The redaction is based on sensitive-looking key names.

Evidence:

- `src/handlers/admin.rs:116-135` returns `redacted_payload` for a webhook delivery.
- `src/db/webhooks.rs:308-330` fetches the full stored provider payload.
- `src/handlers/admin.rs:331-389` performs recursive blacklist-based key redaction.

Impact:

Provider payloads can contain sensitive identifiers under keys not covered by the blacklist, for example:

- `customer_id`
- `external_user_id`
- `user_id`
- `phone`
- `address`
- `ip`
- `billing_details`
- `order_id`
- arbitrary `metadata`

The current model cannot guarantee PII minimization.

Recommended fix:

- Default endpoint should return curated metadata only.
- Require an explicit break-glass/confirmation action for raw payload access.
- Audit every payload view.
- Prefer schema-aware redaction per provider/event.
- For unknown keys, either redact all string leaves by default or allowlist known-safe fields.
- Fully redact emails/secrets unless there is a specific operational need for suffix-preserving redaction.

### P2 — Manual background job trigger is synchronous, repeatable, and unaudited

The admin endpoint accepts a list of job names and runs each job inline in the HTTP request.

Evidence:

- `src/main.rs:178-185` exposes `/admin/trigger-jobs`.
- `src/handlers/admin.rs:171-187` accepts a caller-provided `jobs` list.
- `src/handlers/admin.rs:195-207` validates job names but does not deduplicate or cap repeated valid names.
- `src/handlers/admin.rs:207-260` runs jobs synchronously.

Impact:

A valid admin, or a compromised admin session, can repeatedly trigger expensive/provider-touching jobs and tie up request workers, database work, or provider calls. Without action provenance, post-incident investigation is weak.

Recommended fix:

- Deduplicate jobs.
- Reject duplicates and cap requested jobs to the known job set.
- Add admin/API rate limits.
- Add per-job concurrency locks so reconciliation/cleanup cannot overlap themselves.
- Prefer enqueueing a job and returning `202 Accepted` instead of running synchronously.
- Log requested jobs and per-job results with the admin principal/session metadata.

### P3 — Admin UI renders `app_url` into `href` without URL scheme validation

The UI escapes HTML but inserts `app.app_url` into an anchor `href` and opens it in a new tab.

Evidence:

- `templates/admin.html:421-426` renders `app_url` into an anchor with `target="_blank"`.

Impact:

If `app_url` can be influenced by an app/tenant record, an unsafe scheme such as `javascript:` can become an admin-click XSS vector. The link also lacks `rel="noopener noreferrer"`.

Recommended fix:

- Only render `http:` and `https:` URLs.
- Build DOM nodes with `document.createElement("a")` rather than string-concatenated `innerHTML`.
- Add `rel="noopener noreferrer"`.

### P3 — Admin page lacks browser hardening around third-party script loading

The admin page loads Clerk JS from jsDelivr and there is no obvious CSP/security header layer for the admin UI.

Evidence:

- `templates/admin.html:334-337` loads `https://cdn.jsdelivr.net/npm/@clerk/clerk-js@5/dist/clerk.browser.js`.
- No project-wide admin CSP/security header middleware was found during the audit.

Impact:

The admin UI depends on a third-party CDN script path. Without CSP/SRI or a tighter script-src policy, compromise or unexpected script substitution has a larger blast radius.

Recommended fix:

- Add CSP/security headers for the admin page.
- Prefer Clerk's recommended script loading path with pinned version guidance.
- If using a CDN, consider SRI where practical.
- At minimum, constrain `script-src`, `connect-src`, `frame-src`, and `frame-ancestors` for Clerk and Bridge admin needs.
