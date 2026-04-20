---
name: rag-project-context
description: "Extracts architectural patterns, design decisions, and project ownership via subagent codebase analysis. Use before major features/changes to bootstrap deep project understanding without burning main context window."
---

# RAG Project Context

Pre-loads codebase understanding via subagent analysis, enabling code generation with architectural ownership.

## Problem

Complex projects require understanding across:
- Architectural patterns (how handlers work, error strategies)
- Invariant constraints (non-negotiable rules that code must follow)
- Integration points (how subsystems connect)
- Layer boundaries and error handling conventions

Reading all this on-demand burns the main context window. Over time, code becomes technically correct but violates invariants or architectural boundaries.

## Solution

**Offload codebase indexing to a subagent:**

1. Handoff to RAG subagent with task description
2. Subagent extracts relevant patterns, decisions, lessons from codebase
3. Returns compressed, curated summary
4. Main agent uses summary to write integrated code
5. Context window remains healthy for actual implementation

## Workflow

### Before Starting Major Work

```
User: "Build webhook retry handler for Stripe"
                ↓
Handoff to RAG subagent
                ↓
Subagent analyzes:
  - How existing webhooks work (src/webhooks/)
  - Current retry patterns in codebase
  - Database structures for webhook state
  - Error handling conventions (from INVARIANTS.md)
  - Provider integration patterns
  - Architectural constraints (from INVARIANTS.md + DESIGN.md)
                ↓
Returns summary:
  "Webhooks use [pattern], retries stored in [table],
   errors logged via [mechanism], providers validated by [method],
   must follow invariants [list]"
                ↓
Main agent writes handler with full context
```

### RAG Subagent Instructions

When handing off, use this prompt:

```
You are a codebase analyzer. Extract and summarize architecture for: [TASK]

From c:/share/tyde/bridge, start with:

**Reference Docs** (read first for hard constraints):
- INVARIANTS.md (non-negotiable rules: money handling, status, layer boundaries, webhooks, errors)
- DESIGN.md (system components, data flows, subsystem architecture)
- AGENTS.md (code style, developer principles, patterns)

Then extract from codebase:

1. **Relevant Patterns**: Find 3-5 existing code examples matching the task
   - If writing handler: show handler structure, error handling, DB queries
   - If adding service: show service patterns, testing, configuration
   - If modifying webhooks: show webhook flow, validation, retry logic

2. **Invariant Constraints** (from INVARIANTS.md)
   - Which invariants apply to this task?
   - What rules MUST be followed (not optional)?
   - What cannot change by design?

3. **Integration Points**: How does this connect to other subsystems?
   - Which modules does it touch?
   - Which database tables matter?
   - Which external services interact?
   - Reference DESIGN.md's component diagram for context

4. **Architectural Principles**:
   - Layer boundaries that must be respected (INVARIANTS.md)
   - Error handling conventions (INVARIANTS.md + existing code)
   - What patterns work well (from existing code)
   - K.I.S.S. principle: is there a simpler approach?

Compress into ONE concise summary (<500 tokens). Focus on what's needed for [TASK].

Return as markdown with:
- Code file references (e.g., src/handlers/webhook.rs#L45-60)
- INVARIANTS.md references (e.g., "See INVARIANTS.md: Webhook Processing")
- DESIGN.md section references (e.g., "See DESIGN.md §4.2 Webhook Ingress")
```

## When to Use

**Use RAG when:**
- Starting major new feature (handler, service, webhook)
- Integrating with existing subsystems
- Unsure about architectural patterns
- Want code that "feels like it belongs"

**Skip RAG for:**
- Quick fixes (1-2 file changes)
- Tasks with explicit AGENTS.md guidance
- Simple configuration changes

## Expected Cost

- **Subagent**: ~2-3k tokens (one-time codebase analysis)
- **Main agent**: Saves ~10-15k tokens by avoiding redundant exploration
- **Net**: Slightly more expensive upfront, but enables better code faster

## Integration with AGENTS.md

Add to AGENTS.md if you adopt this pattern:

```markdown
## Codebase Understanding (RAG Pattern)

For major features, use the rag-project-context skill to bootstrap architectural understanding:

1. Call `handoff` with goal + task description
2. Subagent extracts patterns, decisions, integration points
3. Use returned summary to write integrated code

See .agents/skills/rag-project-context/ for details.
```

## Example: Adding a New Payment Provider

**Without RAG:**
- Read handlers/ (10+ files)
- Search for provider patterns (Grep multiple times)
- Check database schema (migrations/)
- Look at existing providers (services/)
- ~20-30 min exploration + risk of missed patterns

**With RAG:**
- Handoff: "Add new provider integration"
- Subagent returns: provider pattern, DB structure, validation approach, error handling
- Write handler with confidence: ~5 min setup + focused implementation

## Notes

- INVARIANTS.md is non-negotiable—violations cause architectural drift
- Subagent should emphasize "show the code, not concepts"
- Compress summaries ruthlessly—only include what this task needs
- Thread history from subagent is temporary; extract actionable patterns only
- Reference INVARIANTS.md heavily—these rules are why the system works
