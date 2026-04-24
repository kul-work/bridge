# Local Code Review Heuristics

## Check failure classification

Treat a failure as **change-related** when the output points to the files or behavior touched by the current diff:

- compile, lint, or typecheck errors in changed files
- deterministic test failures in modules touched by the change
- snapshot or output drift caused by the current edits
- architectural reject output from `code-fit-evaluator`

Treat a failure as **environmental or unclear** when the output points somewhere else:

- missing local toolchain or command not found
- network or registry outages during dependency fetches
- unrelated failures in untouched modules with no visible connection to the diff
- ambiguous `code-fit-evaluator` guidance that needs a product or architecture decision

If uncertain, inspect the failing output once before changing code.

## Review feedback priorities

1. Unresolved local review findings
2. Failing local checks
3. Extra cleanup or optional improvements

If both review feedback and failing checks are present, fix the review feedback first when it will naturally rerun the same checks.

## When To Use `code-fit-evaluator`

Run `code-fit-evaluator` when the change is non-trivial:

- multiple modules changed
- new abstraction, handler, service, or worker
- architectural boundary is part of the task

Skip it for tiny edits like comments, typos, or clearly local fixes.

## Stop-And-Ask Conditions

Stop and ask the user instead of continuing automatically when:

- the worktree has unrelated dirty files that make safe edits uncertain
- required local tools are missing
- the failing output does not point to a safe local fix
- architectural reject guidance conflicts with explicit user instructions
