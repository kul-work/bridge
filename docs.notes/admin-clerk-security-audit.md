# Bridge Admin Clerk Security Audit

Audit date: 2026-06-22  
Baseline commit: `05000c90245cbb4457135e6fc5a820bb57e2b992`  
Scope: admin area and Clerk-backed admin API changes introduced after the baseline commit, with emphasis on login, authorization, and admin auditability.

## Executive Summary

The admin page being public is not the main issue; a Clerk publishable key is public by design, and the HTML can be served without authentication if all sensitive data/actions are protected server-side.

The current admin API is **not ready for production admin use** because it can accept too broad a class of Clerk sessions and does not attribute sensitive/mutating admin actions to a durable admin principal. The highest-risk gaps are:

1. Admin auth can fail open to any valid Clerk user when `ADMIN_CLERK_ORG_ID` is missing.
2. Org membership alone is treated as admin authorization.
3. Admin actions are not auditable to a Clerk user/session/IP.
4. `ADMIN_CLERK_AUTHORIZED_PARTIES` is bypassable when JWTs omit `azp`.

## Scope Reviewed

- `src/middleware/admin_auth.rs`
- `src/handlers/admin.rs`
- `src/main.rs`
- `templates/admin.html`
- `src/db/webhooks.rs`
- `.env.sample`
- admin-related database changes in the diff from the baseline commit

## Findings

### P0 — Admin auth can fail open to any valid Clerk user

`ADMIN_CLERK_ORG_ID` is optional. If it is absent, the middleware accepts any valid JWT from the configured Clerk issuer without org membership enforcement.

Evidence:

- `src/middleware/admin_auth.rs:91-103` loads `ADMIN_CLERK_ORG_ID` as optional and only logs when it is missing.
- `src/middleware/admin_auth.rs:212-222` enforces org membership only inside `if let Some(required_org_id)`.
- `.env.sample:12-20` documents `ADMIN_CLERK_ORG_ID` as optional and explicitly says any valid JWT from the configured Clerk instance is accepted when omitted.
- `src/main.rs:178-185` exposes sensitive/mutating admin APIs behind this middleware.

Impact:

If the Clerk instance allows public signup, or if the same Clerk app is shared with normal non-admin users, a normal signed-in Clerk user can obtain a valid session JWT and call Bridge admin APIs.

Recommended fix:

- Fail closed in production/staging unless an explicit admin boundary is configured.
- Prefer a dedicated admin Clerk app/config:
  - `ADMIN_CLERK_PUBLISHABLE_KEY`
  - `ADMIN_CLERK_FRONTEND_API`
  - `ADMIN_CLERK_ORG_ID`
- Disable public signup for the admin Clerk app or require invitation-only org membership.
- Treat missing admin authorization config as startup/config error in production, not an info log.

### P1 — Org membership is not sufficient admin authorization

Even when `ADMIN_CLERK_ORG_ID` is configured, the backend only verifies active org ID. It does not check Clerk org role, an admin user allowlist, or a custom admin claim.

Evidence:

- `src/middleware/admin_auth.rs:28-42` models `sub`, `org_id`, and legacy org shape, but no org role/admin claim.
- `src/middleware/admin_auth.rs:44-49` only extracts the active org ID.
- `src/middleware/admin_auth.rs:212-222` only compares org ID.

Impact:

Every member of the configured Clerk organization can update app notes, view webhook payloads, retry deliveries, and trigger background jobs.

Recommended fix:

- Add a second authorization condition, such as one of:
  - Clerk org role, e.g. `org_role == "org:admin"`.
  - `ADMIN_CLERK_ADMIN_USER_IDS` allowlist.
  - Custom Clerk JWT template claim, e.g. `bridge_admin=true`.
- Make token verification return an admin principal instead of `()`.

Example target shape:

```rust
struct AdminPrincipal {
    sub: String,
    org_id: Option<String>,
    org_role: Option<String>,
    azp: Option<String>,
    sid: Option<String>,
}
```

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

### P1 — Admin actions are not auditable to a user/session/IP

The middleware validates the JWT and then discards the identity. Handlers cannot reliably log which admin performed an action.

Evidence:

- `src/middleware/admin_auth.rs:151-227` has `verify_token` return `Result<(), String>`.
- `src/middleware/admin_auth.rs:413-418` calls `next.run(request)` without inserting a principal into request extensions.
- `src/handlers/admin.rs:61-78` updates app notes with no actor audit.
- `src/handlers/admin.rs:116-135` returns webhook payload data with no actor audit.
- `src/handlers/admin.rs:151-164` logs manual retry details but not the Clerk actor.
- `src/handlers/admin.rs:207-265` logs manually triggered job names but not the Clerk actor/session/IP.

Impact:

Bridge cannot answer basic incident questions such as:

- Which admin viewed this webhook payload?
- Which admin retried this webhook delivery?
- Which admin triggered reconciliation or cleanup?
- Which session/IP/user-agent performed the action?

Recommended fix:

- Return `AdminPrincipal` from token verification.
- Insert the principal into request extensions in `admin_auth_middleware`.
- Extract the principal in all admin handlers.
- Add structured audit events for:
  - `admin.auth.success`
  - `admin.auth.denied`
  - `admin.apps.notes.update`
  - `admin.webhook.payload.view`
  - `admin.webhook.retry.request`
  - `admin.jobs.trigger`
- Include at minimum:
  - Clerk `sub`
  - session ID / JWT ID if available
  - org ID and role
  - method/path
  - target app/webhook/job
  - result success/failure
  - remote IP
  - user-agent
- Prefer a durable `pay.admin_audit_log` table over tracing logs only.
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
- Require stronger admin role / break-glass action for raw payload access.
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

A valid admin, or a compromised admin session, can repeatedly trigger expensive/provider-touching jobs and tie up request workers, database work, or provider calls. Without actor audit, post-incident investigation is weak.

Recommended fix:

- Deduplicate jobs.
- Reject duplicates and cap requested jobs to the known job set.
- Add admin/API rate limits.
- Add per-job concurrency locks so reconciliation/cleanup cannot overlap themselves.
- Prefer enqueueing a job and returning `202 Accepted` instead of running synchronously.
- Audit requested jobs and per-job results with the admin principal.

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

## Positive Observations

- Admin API routes are separated from the public admin HTML and wrapped in `admin_auth_middleware`.
- The JWT verifier checks JWT structure, `alg == RS256`, unsupported `crit`, issuer, signature, expiration, `nbf`, and non-empty subject.
- The Clerk publishable key is not secret; serving it in public HTML is expected.
- Notes and payload display mostly use escaping or `textContent`, so the reviewed changes do not show an obvious direct stored-XSS path through notes or payload content.

## Recommended Fix Order

1. Fail closed for admin auth config in production.
2. Add real admin authorization: Clerk org role, admin user allowlist, or custom admin claim.
3. Return/store `AdminPrincipal` and add durable admin audit logging.
4. Require `azp` when `ADMIN_CLERK_AUTHORIZED_PARTIES` is configured; add admin API `aud` validation if possible.
5. Make manual webhook retry atomic and audit its result.
6. Lock down webhook payload viewing with stronger authorization and audit.
7. Harden manual background job triggers with dedupe/caps/rate limits/concurrency locks.
8. Add admin UI browser hardening: CSP, safe URL rendering, and `noopener noreferrer`.

## Suggested Minimal Phase 1

For the smallest high-value fix, do these first:

- Require `ADMIN_CLERK_ORG_ID` or another explicit admin boundary in production.
- Require an admin-specific claim/role/user allowlist in addition to org membership.
- Make `verify_token` return a principal and add audit logs for all mutating/sensitive admin actions.
- Require `azp` when authorized parties are configured.

This phase addresses the primary login/admin audit concern before broadening into UI hardening and operational controls.
