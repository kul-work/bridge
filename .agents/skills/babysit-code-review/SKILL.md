---
name: babysit-code-review
description: Watches a local worktree like a code review queue by polling the current diff, running local check commands, and surfacing unresolved local review findings until the change is review-clean or user help is required. Use when asked to babysit a local code review, keep rerunning checks while fixing issues, or fold code-fit-evaluator results into a local review loop without GitHub or PRs.
---

# Local Code Review Babysitter

## Objective
Babysit a local change persistently until one of these terminal outcomes occurs:

- The local diff is review-clean: no failing local checks and no unresolved local review findings.
- The worktree has no tracked or untracked changes left to review.
- A situation requires user help (for example unrelated dirty state, ambiguous review guidance, missing toolchain, or repeated failures the agent cannot classify safely).

Unlike `babysit-pr`, a review-clean local diff is a terminal state by default. There is no remote PR waiting for fresh comments unless the user explicitly asks to keep watching anyway.

## Inputs
Accept any of the following:

- No arguments: infer the current repo and review the combined staged + unstaged diff.
- One or more repeated `--check` commands for local verification.
- One or more repeated `--review-file` paths containing local review findings.
- Optional `--diff staged|worktree|all` to narrow the review scope.

`--review-file` accepts either:

- a JSON list of review items
- an object with `items: [...]`
- a `code-fit-evaluator` JSON result; when `decision` is `reject`, its reasons are treated as review findings

## Core Workflow

1. When the user asks to monitor or babysit a local code review, start with the watcher's continuous mode (`--watch`) unless a one-shot snapshot is enough.
2. Run the watcher script and inspect the emitted `actions` list.
3. If `process_review_feedback` is present, handle unresolved review findings first.
4. If `diagnose_check_failure` is present, inspect the failed local check output and classify whether the fix belongs in the current diff.
5. Apply the smallest safe fix, then rerun the watcher immediately.
6. If the change is non-trivial, load `code-fit-evaluator` and treat a reject result as review feedback before declaring the diff clean.
7. Continue looping until `ready_for_local_review`, `stop_no_changes`, or a real blocker is reached.
8. Maintain terminal ownership while babysitting is active; do not leave a detached `--watch` process behind.

## Commands

### One-shot snapshot

```bash
python .agents/skills/babysit-code-review/scripts/local_code_review_watch.py --once
```

### Continuous watch

```bash
python .agents/skills/babysit-code-review/scripts/local_code_review_watch.py --watch --check "cargo check 2>&1" --check "cargo test"
```

### Use `code-fit-evaluator` output as local review input

```bash
python .agents/skills/babysit-code-review/scripts/local_code_review_watch.py --once --review-file tmp/code-fit-result.json
```

### Watch only staged changes with local findings

```bash
python .agents/skills/babysit-code-review/scripts/local_code_review_watch.py --watch --diff staged --review-file tmp/local-review.json
```

## Review Sources
The watcher can surface local review work from several places:

- local check failures from repeated `--check` commands
- user-authored JSON review files
- `code-fit-evaluator` reject output converted into actionable review items

Treat unresolved review findings like reviewer comments: either fix them or explain why they do not require code changes.

When using `code-fit-evaluator`:

1. Run it after the implementation is in place.
2. Save or otherwise capture its strict JSON output.
3. Feed that JSON into `--review-file`.
4. If the decision is `reject`, address the reasons before declaring the diff clean.

## Git Safety Rules

- Work only in the current local branch and worktree.
- Avoid destructive git commands.
- Before editing, check for unrelated uncommitted changes outside the requested scope. If present and risky, stop and ask the user.
- Do not auto-commit unless the user explicitly asks for commits.
- Re-run the watcher after each fix instead of assuming the worktree is now clean.

## Monitoring Loop Pattern
Use this loop in a live session:

1. Run `--once` or start `--watch`.
2. Read `actions`.
3. Process unresolved review feedback before local check failures when both are present.
4. Fix one coherent batch of issues.
5. Run verification commands again.
6. If the change is substantial, run `code-fit-evaluator` and include its result in the next watcher cycle.
7. Stop only when the diff is review-clean, the worktree no longer has changes to review, or a blocker needs user help.

Prefer `--watch` when the user explicitly asks to keep watching. Use `--once` for diagnosis, local testing, or a quick health snapshot.

## Polling Cadence

- While the diff still has unresolved review findings or failing checks: poll every 30 seconds.
- Reset the cadence immediately after any code change, review-file update, or check-status change.
- Once the diff is review-clean, stop and report the clean state instead of idling forever.

## Stop Conditions
Stop only when one of the following is true:

- `ready_for_local_review` is present.
- `stop_no_changes` is present.
- User intervention is required and Codex cannot proceed safely.

## Output Expectations
Provide concise progress updates while monitoring and a final summary that includes:

- current branch and HEAD SHA
- diff scope and changed files
- local check status summary
- unresolved review findings still open
- whether `code-fit-evaluator` was used and whether it accepted or rejected the change

## References

- Local review heuristics: `.agents/skills/babysit-code-review/references/heuristics.md`
- Watcher script: `.agents/skills/babysit-code-review/scripts/local_code_review_watch.py`
