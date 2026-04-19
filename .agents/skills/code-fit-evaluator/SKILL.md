---
name: code-fit-evaluator
description: "Evaluates whether proposed code changes FIT the existing system's architecture, patterns, and direction. Strict accept/reject with JSON output. Use after implementing changes to validate architectural alignment before committing."
---

# Code Fit Evaluator

Strict architectural fit evaluation for proposed code changes. Accepts or rejects based on pattern alignment, not syntax or style.

## Problem

Code can be technically correct but architecturally wrong:
- Introduces a new pattern when a canonical one exists
- Solves locally but harms global coherence
- Creates one-off solutions that resist future changes
- Violates layer boundaries even when practical

These issues compound over time into architectural drift that's expensive to fix.

## What It Does

- **Evaluates architectural FIT**, not correctness
- **Accepts or rejects** changes with clear reasoning
- **Provides rewrite guidance** on rejection (minimal, directional)
- **Outputs strict JSON** for programmatic consumption
- **Runs iteratively** — rejected changes get fed back with guidance

## When to Use

**Use code-fit-evaluator when:**
- Finishing a non-trivial feature before committing
- Adding a new handler, service, or integration
- Touching multiple modules or introducing abstractions
- Unsure if implementation matches project patterns

**Skip for:**
- Typo fixes, comment updates, docs-only changes
- Config changes with no logic impact
- Changes explicitly designed to break from existing patterns (user-approved)

## Workflow

```
1. Implement feature/change
2. Run code-fit-evaluator
3. If ACCEPT → commit
4. If REJECT → apply rewrite_guidance → re-evaluate
5. Max 2-3 iterations
```

### Step-by-Step

1. **Gather the diff**: Run `git diff` (or `git diff --cached` for staged)
2. **Build the intent** (structured, not free text)
3. **Identify reference patterns** from the codebase
4. **Run evaluation** using the prompt templates below
5. **Act on result**: accept → done, reject → fix and re-evaluate

## Evaluation Persona (System Prompt)

Use this **once** as system context. Do not change per request.

```
You are a senior software architect acting as a strict code fit evaluator.

Your role is NOT to check syntax, style, or low-level correctness.
Your role is to evaluate whether a proposed change FITS the existing system.

You must decide:
- Does this change align with the system's architecture, patterns, and direction?
- Or does it introduce drift, inconsistency, or hidden long-term cost?

You are strict. If in doubt → REJECT.

You do NOT suggest redesigns.
You do NOT optimize.
You ONLY:
- Accept
- Reject with clear reasons
- Provide minimal rewrite guidance

Evaluation principles:

1. Prefer consistency over novelty
2. Prefer existing patterns over new abstractions
3. Reject "one-off" solutions
4. Reject layer violations even if practical
5. Reject duplication when a canonical solution exists
6. Reject solutions that solve locally but harm global coherence
7. Reject unclear or implicit behavior

You must output STRICT JSON only. No prose outside JSON.
```

## User Prompt Template (Dynamic)

Fill this per evaluation. All sections are required unless marked optional.

```
## INTENT
{{intent_json}}

## CONSTRAINTS
{{constraints_list}}

## ARCHITECTURE SUMMARY
{{short_arch_summary}}

## EXISTING PATTERNS (REFERENCE IMPLEMENTATIONS)
{{code_examples_or_descriptions}}

## CHANGED CODE (DIFF)
{{diff}}

## RELATED CONTEXT
{{optional_related_files}}

---

## TASK

Evaluate whether the proposed change FITS the system.

Focus on:
- alignment with existing patterns
- architectural consistency
- risk of long-term drift

---

## DECISION RULES

- ACCEPT only if strongly aligned
- REJECT if any meaningful deviation exists
- If unsure → REJECT

---

## OUTPUT FORMAT (STRICT)

{
  "decision": "accept" | "reject",
  "confidence": 0.0-1.0,
  "reasons": [
    {
      "type": "pattern_deviation | duplication | layer_violation | abstraction_mismatch | inconsistency | hidden_complexity",
      "explanation": "short, specific reason"
    }
  ],
  "rewrite_guidance": [
    "concrete, minimal directional fixes"
  ]
}

Rules:
- Max 5 reasons
- Keep explanations concise and technical
- rewrite_guidance must be actionable
- No empty fields
```

## Intent Template

**Always structured. Never free text.**

```json
{
  "feature": "what is being added/changed",
  "expected_pattern": "which existing pattern should it follow",
  "touches": ["moduleA", "moduleB"],
  "anti_goals": [
    "what must NOT happen"
  ]
}
```

### Example Intent

```json
{
  "feature": "subscription grace period",
  "expected_pattern": "reuse billing retry flow",
  "touches": ["billing", "subscriptions"],
  "anti_goals": ["no duplicate retry logic", "no new scheduler"]
}
```

## Architecture Summary Template (Bridge-Specific)

Use this compressed summary for Bridge evaluations. Adapt if project changes.

```
- handlers: HTTP endpoints, orchestration only, no business logic
- services: provider integrations (creem, google_play, coinbase), core logic (payment, email)
- webhooks: ingress routing, status processing, idempotent logging
- application: domain orchestrators, business rules
- db: queries separated by domain, SQLx + PostgreSQL
- middleware: auth, rate limiting, tracing layers
- workers: reconciliation, cleanup, price step-up, pause scheduler
- ports: external trait definitions (abstraction layer)
```

## Reference Patterns Section

**Critical for evaluation quality.** Provide real distilled patterns, not raw code dumps.

### Good Example

```
Pattern: Webhook ingress processing
- All webhooks go through webhooks/ingress.rs
- Provider signature verified first
- Idempotency checked via webhook_log table
- Status normalized to canonical states
- No business logic in webhook handlers

Pattern: Error handling
- thiserror for typed domain errors
- anyhow for context propagation
- Handlers map errors to HTTP status codes
- No unwrap() in production paths
```

### Bad Example

```
(dumping 200 lines of raw handler code without explanation)
(pasting unrelated module as "context")
```

## Output Format

### Accept

```json
{
  "decision": "accept",
  "confidence": 0.92,
  "reasons": [
    {
      "type": "pattern_deviation",
      "explanation": "none — follows existing webhook ingress pattern"
    }
  ],
  "rewrite_guidance": []
}
```

### Reject

```json
{
  "decision": "reject",
  "confidence": 0.85,
  "reasons": [
    {
      "type": "duplication",
      "explanation": "retry logic duplicated from BillingRetryService into handler"
    },
    {
      "type": "layer_violation",
      "explanation": "business rule (grace period calc) placed in HTTP handler instead of application layer"
    }
  ],
  "rewrite_guidance": [
    "Move grace period calculation to application/billing.rs",
    "Reuse existing retry_with_backoff from services/payment.rs"
  ]
}
```

## Optional: Score Layer

Extend output for finer-grained control:

```json
"score": {
  "pattern_alignment": 0.0-1.0,
  "architectural_consistency": 0.0-1.0,
  "duplication_risk": 0.0-1.0
}
```

Auto-reject if any score < 0.7 (enforce outside the model).

## Agent Instructions

When this skill is invoked, the agent MUST:

1. **Collect the diff**
   - Run `git diff` for unstaged changes (default)
   - Run `git diff --cached` for staged changes
   - Or accept a user-provided diff

2. **Build the intent** by asking the user or inferring from context:
   - What feature is this?
   - What existing pattern should it follow?
   - What modules does it touch?
   - What must NOT happen?

3. **Extract reference patterns** from the codebase:
   - Find 2-4 existing implementations of the expected pattern
   - Distill each into a short description (not raw code)

4. **Assemble the evaluation prompt** using the templates above

5. **Present the JSON result** to the user:
   - If **accept**: confirm and proceed
   - If **reject**: show reasons + rewrite_guidance, ask if user wants to fix

6. **On rejection iteration** (max 2-3 rounds):
   - Apply rewrite_guidance
   - Re-run evaluation with updated diff
   - Stop if stuck in reject loop — escalate to user

## Pitfalls

- **No reference patterns** → quality drops drastically, always provide them
- **Vague intent** → false accepts increase, always structure it
- **Huge diffs** → split into logical chunks, evaluate each separately
- **Ignoring rewrite_guidance** → defeats the purpose of the loop
