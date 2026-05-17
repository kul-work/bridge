# Bridge Split History - Codex Conclusions

Date: 2026-05-17

Scope: Git history in `C:\share\tyde\bridge` from 2026-03-19 through 2026-05-17.

This document is a compact, data-first conclusion from the local Git history. It does not try to retell every commit. It separates observed facts from interpretation.

## 1. Executive Summary

Bridge was estimated as a short code split, but the Git history shows it became a productization effort for a standalone payment platform.

The main estimate failure was not commit volume by itself. The failure was scope classification: the work was treated as extraction, while the actual work included multi-app tenancy, RLS, webhook security, provider normalization, callback delivery, lifecycle state machines, background workers, economic payment identity, and high-fidelity provider test suites.

The most important signal is not "457 commits." It is where the churn landed:

- `src/webhooks/processor.rs`
- `src/db/subscriptions.rs`
- `src/webhooks/ingress.rs`
- `src/webhooks/scheduler.rs`
- `src/handlers/verify_purchase.rs`
- `src/application/verify_purchase.rs`
- `src/db/payments.rs`
- `docs.notes/BEHAVIORAL_SPEC_GAPS.md`
- `docs.notes/BEHAVIORAL_SPEC_AUDIT.md`

Those are the system boundary files: provider ingress, state mutation, subscription identity, payment identity, retry behavior, and verification contracts. Repeated churn there points to late discovery of domain behavior, not just normal implementation polish.

## 2. Raw Git Evidence

### 2.1 Commit Count and Duration

From 2026-03-19 through 2026-05-17:

```text
Commits: 457
Calendar duration: 59 days
Approximate duration: 8.4 weeks
```

Author distribution:

```text
450  Mihaita Nita
4    Mihai
3    Devin AI
```

This was effectively a single-developer effort with occasional external or assisted commits.

### 2.2 Commits per Week

```text
2026-W12   37
2026-W13   26
2026-W14   52
2026-W15  110
2026-W16   49
2026-W17   77
2026-W18   20
2026-W19   46
2026-W20   40
```

Week 15 was the clear peak. Week 17 was another large wave. The history does not look like a short extraction followed by quiet stabilization.

### 2.3 Phase Counts

```text
03-19..03-29   63   initial extraction/scaffold
03-30..04-09  147   gap-fixing and ports/hexagonal rewrite
04-10..04-30  157   test migration and provider behavior fixes
05-01..05-17   90   stabilization, Google Play, OTP, payment identity fixes
```

The highest volume was not the initial scaffold. The largest activity came after the first extraction, when behavior gaps, architecture boundaries, tests, and provider realities were being reconciled.

### 2.4 Top Changed Files

```text
59  src/webhooks/processor.rs
37  src/db/subscriptions.rs
37  src/webhooks/ingress.rs
32  docs.notes/BEHAVIORAL_SPEC_GAPS.md
28  src/webhooks/scheduler.rs
26  src/main.rs
23  src/handlers/verify_purchase.rs
22  docs.notes/BEHAVIORAL_SPEC_AUDIT.md
22  src/handlers/subscriptions_actions.rs
21  src/application/verify_purchase.rs
21  src/ports.rs
19  src/db/payments.rs
16  src/webhooks/forwarding.rs
16  docs/pay-tydecode-architecture.md
15  src/handlers/users.rs
15  Cargo.toml
14  src/handlers/subscriptions.rs
14  src/services/payment.rs
14  src/webhooks/processor/event_handlers.rs
```

This is the strongest evidence in the history. Most churn is not isolated in cosmetic code or scaffolding. It is concentrated in behavioral and integration boundaries.

### 2.5 Line Churn by Area

```text
67818  tests       +48183  -19635
61117  src         +40285  -20832
23890  docs.notes  +13197  -10693
7869   docs        +5421   -2448
2147   migrations  +1527   -620
```

The test and source churn are comparable in scale. That suggests tests were not just final validation; they were part of discovering and stabilizing behavior.

The `docs.notes` churn is also large. That is useful evidence that behavioral uncertainty was actively being investigated during the work.

### 2.6 Subject Keyword Counts

```text
fix              149
update           122
doc               96
test              53
gap               46
remove/dead       33
google/gpbi       24
hexa/ports        23
creem             22
webhook           16
subscription      14
OTP/one-time      13
release/RN        14
refactor           8
RLS                8
```

These counts are approximate because they are based on commit subjects, not semantic parsing. Still, they show the shape of the work: many fixes, many generic updates, repeated gap work, and repeated provider/test/webhook themes.

### 2.7 Tags

```text
2026-03-28  v0.1.2
2026-04-09  v0.2.0
2026-05-09  v0.3.0
2026-05-14  v0.3.1
```

The existing historical analysis mentions `v0.1.0`, but the current tag list does not prove that release. The first visible tag is `v0.1.2`.

## 3. Timeline Interpretation

### Phase 1: Initial Extraction and Scaffold, 2026-03-19 to 2026-03-29

The early commits show project setup, database tables, migrations, RLS appearing very early, provider settings, endpoint fixes, release notes, and a first tagged release at `v0.1.2`.

Representative subjects include:

```text
arhitecture docs
cargo init files
added Bridge db tables
added RLS to tables
moved provider details to separate table
fixed endpoints
chore: Release bridge version 0.1.2
```

Conclusion: the service was extracted quickly enough to look promising, but the early history already includes tenant/security/database concerns. It was not a pure copy-out.

### Phase 2: Behavioral Gap and Ports Rewrite, 2026-03-30 to 2026-04-09

This phase contains the clearest evidence of a process problem. There is a long series of `gap fixes`, then a burst of `hexa` and `ports` commits around 2026-04-07 and 2026-04-08.

Representative subjects include:

```text
added first GPBI tests
more GPBI tests migrated
gap fixes
spec update
reverted to semi-initial state
hexagonal refactor fix
hexa rewrite: webhook port refactor
hexa rewrite: Google lifecycle refactor
hexa rewrite: scheduler.rs port-driven end to end
ports cleanup - BridgeRepository umbrella open; removed to smaller parts
hexa fixes 1
hexa fixes 2
hexa fixes 3
hexa fixes 4
hexa fixes 5
hexa fixes 6
hexa fixes 7
hexa fixes 8
FIX (rls): scope subscription action writes to the active app
chore: Release bridge version 0.2.0
```

Conclusion: the architecture boundary was still being discovered after implementation had begun. That is not automatically a failure, but it is a strong explanation for the missed estimate. A split estimate should have included time for behavioral inventory and boundary design if the old app was not already modular around payment concerns.

### Phase 3: Test Migration and Provider Behavior, 2026-04-10 to 2026-04-30

This phase is dominated by test suite alignment, Creem behavior, webhook behavior, OTP, Google Play lifecycle mapping, and cleanup/removal of old provider code.

Representative subjects include:

```text
SUB tests update
SUB-04 fix
SUB-06 fix
SUB-08 fix
RTDN tests aligns
WHK test suite alignment and fixes
Creem integration fix implementation
OTP for Creem payments status check fix
FIX (webhooks): Handle Creem payment.failed and payment.partially_refunded for OTP
FIX (currency): Replace f64 arithmetic with integer-cent parsing and formatting
FIX (webhooks): Make subscription transitions user-aware to prevent ambiguous lookups
FEAT (webhooks): add verify_webhook_signature config and test-mode bypass to Creem handler
FIX (google-play): Align RTDN webhook event mappings
REFACTOR (google-play): Deduplicate subscription lifecycle outcomes
FIX (webhooks): resume stranded duplicate provider events
REFACTOR: Remove Coinbase provider from codebase
```

Conclusion: much of the real complexity emerged through integration and testing. This phase looks like late parity discovery: behavior from the old system, provider edge cases, and new Bridge invariants were being reconciled by tests after core implementation had started.

### Phase 4: Stabilization and Payment Identity, 2026-05-01 to 2026-05-17

This phase adds lifecycle emails and richer subscription APIs, but the important risk signal is that core Google Play and payment identity fixes still appear late.

Representative subjects include:

```text
FEAT (subscriptions): Expose lifecycle status snapshots
FEAT (subscriptions): Expand list API with full lifecycle fields
FEAT (lifecycle-emails): Wire email dispatch for paused, resumed, and refunded events
FIX (Google Play): price step-up consent flow
FIX (OTP): refund classification in Bridge webhooks
FEAT (google-play): Handle pending one-time purchase verification
FIX (google-play): Acknowledge OTP purchases only after completion
FIX (google-play): Enforce subscription acknowledgement lifecycle
FIX (payments): Prevent OTP status downgrades after approval
FIX (subscriptions): Preserve provider period end during reconciliation
FIX (google-play): Validate Pub/Sub audience and accept test notifications
BIG FIX (google-play): Preserve renewal webhooks and payment records
FIX (google-play): Separate purchase tokens from payment transaction IDs
FIX (payments): Persist Google Play payment currency
```

Conclusion: the project is probably stabilizing, but it was still finding core billing identity bugs in the final two days of this analysis window. I would not call it historically stable as of 2026-05-17. I would call it late-stage stabilization after core invariants were finally made explicit.

## 4. Comparison with Existing History Analysis

The existing document is directionally useful, but it is too confident in places.

Supported claims:

- The split took around 8.4 weeks, not a couple of weeks.
- The ports/hexagonal rewrite burst is visible in Git.
- Webhook, subscription, ingress, scheduler, payment, and verification code were high-churn areas.
- Google Play and OTP behavior caused late stabilization work.
- The May 16 renewal/payment fix was substantial.
- Test suites became a major part of the project.

Claims that need softer wording:

- `v0.1.0` is not proven by current tags. The visible tag is `v0.1.2` on 2026-03-28.
- The `f64` to cents change was important, but Git stats do not support calling it a massive refactor by itself. Commit `39f80c8` touched 4 files with 62 insertions and 5 deletions.
- "Architecture broke" may be true as an interpretation, but Git proves a heavy rewrite/gap-fix wave, not the internal reason with certainty.
- "Absence of a spec caused X" should be phrased as inference. The stronger fact is that behavioral gap docs and gap-fix commits were active during implementation.

## 5. Root Cause Analysis

### 5.1 Primary Root Cause: Wrong Work Category

The original estimate appears to have classified the project as a split from an already working app.

The Git history shows the work was actually:

- extracting payment logic,
- generalizing it from HiHa-specific behavior into a multi-app service,
- securing the boundary,
- normalizing provider behavior,
- defining payment/subscription invariants,
- building callback delivery,
- adding background workers,
- creating or migrating test suites,
- removing old provider assumptions,
- discovering provider-specific edge cases.

That is not a short split. That is a new platform boundary.

### 5.2 Secondary Root Cause: Behavioral Inventory Was Late

The repeated `gap fixes`, `BEHAVIORAL_SPEC_GAPS`, `BEHAVIORAL_SPEC_AUDIT`, test alignment, and provider-specific fixes indicate that behavior was still being discovered while code was being built.

The process should probably have started with a behavior inventory:

- What HiHa behavior must be preserved exactly?
- What behavior changes because Bridge is multi-app?
- What are the subscription lifecycle invariants?
- What are the payment audit invariants?
- What is provider-specific and what is canonical?
- What is the callback contract to HiHa?
- What test cases prove parity?

Without that inventory, commits naturally become reactive.

### 5.3 Third Root Cause: Payment Provider Reality Was Underestimated

Google Play, Creem, subscriptions, OTPs, refunds, acknowledgement, RTDN payloads, sandbox renewal cadence, purchase tokens, order IDs, and stale events are not a simple CRUD extraction.

Late commits around purchase token identity and renewal preservation are especially important. They suggest that some economic invariants were not explicit enough at the start:

- purchase token is not the same thing as transaction/order ID,
- lifecycle identity is not the same thing as economic payment identity,
- webhook deduplication must not suppress valid renewal events,
- partial provider events must not erase existing subscription fields.

These are domain invariants. They are hard to recover cheaply once implementation has spread them through DB queries, webhook processing, and tests.

### 5.4 Fourth Root Cause: Tests Were Doing Discovery Work

Tests have the highest line churn:

```text
tests  67818 total changed lines
src    61117 total changed lines
```

That is not bad by itself. But the timing matters: many test alignment and fix commits happened after major implementation and architecture work. That suggests tests were used not only to verify known behavior, but to discover the missing behavior.

For provider-heavy payment systems, that discovery needs to happen earlier.

## 6. What Went Right

The history is not just failure. Several things look healthy:

- The project did add a large test surface.
- The architecture was corrected rather than left in an unsuitable shape.
- RLS and tenant boundaries were handled early and revisited later.
- Dead provider code was removed instead of preserved indefinitely.
- Late Google Play payment identity bugs were documented and fixed rather than papered over.
- Release tags show staged stabilization points.
- The most important churn is in the right areas for a payment gateway.

The work appears to have converged toward a stronger system. The issue is that convergence happened through expensive iteration.

## 7. What Went Wrong

The missed estimate most likely came from these mistakes:

1. Treating a product boundary as a code movement task.
2. Starting implementation before the behavioral inventory was complete.
3. Discovering provider and economic invariants through late tests and live-ish scenarios.
4. Combining extraction, architecture correction, provider hardening, security, tenancy, and test harness work into one moving stream.
5. Allowing vague commit subjects like `fix`, `update`, and `gap fixes` to hide the true cost centers during the project.
6. Not freezing the minimal success criteria for "Bridge split done" early enough.

## 8. My Final Conclusion

The split did not overrun because 457 commits is inherently too many.

It overran because the estimated task was "split Bridge from HiHa," while the actual task became "design and stabilize a central multi-tenant payment gateway that preserves HiHa behavior while adding provider normalization, security boundaries, lifecycle correctness, callback delivery, economic auditability, and regression tests."

That is a fundamentally different project.

The most defensible postmortem statement is:

> Bridge was estimated as an extraction, but executed as a new payment platform. The missing up-front work was a behavioral and invariant inventory. Without that, implementation became the discovery mechanism, producing repeated gap fixes, architecture rewrites, provider-specific corrections, and late payment identity fixes.

As of 2026-05-17, the repo looks closer to stabilization than thrashing, but the stabilization is recent. Late commits still touch core payment identity and Google Play behavior, so Bridge should be treated as a system entering stabilization, not one with long-proven stability.

## 9. Recommendations

### Short Term

- Define a concrete "Bridge is stable enough" checklist.
- Make Google Play subscription renewal and OTP refund tests mandatory before release.
- Add explicit tests for purchase token versus transaction/order ID behavior.
- Keep callback contract tests for HiHa as release blockers.
- Audit recent payment rows in the database for token-keyed versus order-keyed identity mistakes.
- Convert vague remaining notes into either tests or closed decisions.

### Medium Term

- Maintain a provider behavior matrix for Google Play and Creem.
- Keep economic invariants in `INVARIANTS.md`, not only in fix docs.
- Track high-risk files by churn and require focused review when they change.
- Treat any future provider addition as a platform feature, not a small integration.
- For future splits, write the parity test plan before extraction begins.

## 10. Commands Used

Representative commands:

```powershell
git rev-list --count --since='2026-03-19 00:00:00' HEAD
git log --since='2026-03-19 00:00:00' --date=format:'%G-W%V' --pretty=format:'%ad'
git log --since='2026-03-19 00:00:00' --date=short --pretty=format:'%h`t%ad`t%s'
git log --since='2026-03-19 00:00:00' --numstat --pretty=format:
git log --since='2026-03-19 00:00:00' --pretty=format:'%s'
git log --since='2026-03-19 00:00:00' --merges --date=short --pretty=format:'%h`t%ad`t%s'
git tag --sort=creatordate --format='%(creatordate:short)`t%(refname:short)'
git show --stat --oneline 39f80c8
git show --stat --oneline 26c5e2f
```

