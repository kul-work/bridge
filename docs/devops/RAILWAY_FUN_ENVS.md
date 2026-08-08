# Railway Fun Envs / Disposable Research Sandboxes

This note captures a future direction for temporary Railway environments used for scary experiments: migrations, provider config changes, PgBouncer tuning, destructive account-deletion paths, load tests, or any change that should not touch staging or production.

The goal is not to build a heavy platform. The goal is a repeatable, low-friction way to spin up an isolated environment, test hard, and destroy it cleanly.

## Mental Model

```text
Dockerfile       = how to build/run one container
railway.ts       = how Railway cloud resources should exist
CLI scripts      = repeatable lifecycle actions
MCP/agent prompts = inspection, logs, diagnosis, guided ops
```

The useful combination is all three:

```text
.railway/railway.ts = baseline infra shape
CLI scripts         = create/destroy/deploy workflows
MCP/agent prompts   = observability, logs, diagnosis, health checks
```

## Target Environment Strategy

```text
staging = always-on integration lab, follows main
PROD    = manual controlled release, from a verified commit
sandbox = temporary disposable research env, follows a CR branch
```

Recommended operating flow:

```text
main -> auto staging
staging passed -> manual PROD

CR/scary branch -> disposable sandbox
experiment done -> destroy sandbox
```

## Responsibility Split

### 1. `.railway/railway.ts` - Source of Truth for Shape

Use Railway IaC for the stable infrastructure recipe:

- services that should exist;
- source repositories and branch wiring;
- root directories/build/start config;
- environment-level shape;
- Railway-owned resources when needed;
- stable non-secret variable names and references.

Role:

```text
"The cloud should look like this."
```

Keep it readable and boring. Avoid turning `railway.ts` into a procedural script. If an action has steps, branching, cleanup, or user-provided parameters, it probably belongs in a CLI script.

### 2. CLI Scripts - Lifecycle Automation

Use scripts for repeatable actions:

- create a research sandbox;
- inject a Neon branch database URL;
- point services at a CR branch;
- deploy services;
- run a smoke check;
- grab a small diagnostic bundle;
- destroy the sandbox.

Role:

```text
"Create/destroy this experiment environment safely."
```

Possible future scripts:

```text
scripts/railway/create-research-sandbox.ps1
scripts/railway/destroy-research-sandbox.ps1
scripts/railway/deploy-staging.ps1
scripts/railway/grab-logs.ps1
scripts/railway/smoke-sandbox.ps1
```

Expected create command shape:

```powershell
scripts/railway/create-research-sandbox.ps1 `
  -Name "hh-pgbouncer-spike" `
  -Branch "cr/pgbouncer-spike" `
  -DatabaseUrl "postgresql://...neon..." `
  -AdminDatabaseUrl "postgresql://...neon..."
```

Expected destroy command shape:

```powershell
scripts/railway/destroy-research-sandbox.ps1 -Name "hh-pgbouncer-spike"
```

Destructive scripts should always print what they will delete and require an explicit confirmation unless run with a deliberate force flag.

### 3. MCP / Agent Prompts - Operator Assistant

Use Railway MCP and prepared agent prompts for platform inspection, not as the only source of truth for creating infrastructure.

Good MCP/agent jobs:

- fetch deployment status;
- grab recent build/deploy/http logs;
- summarize crash loops;
- compare staging/prod variable names without printing secrets;
- check service domains and health;
- inspect resource usage after a load test;
- find the deployment associated with a commit/branch;
- explain why a deploy failed.

Role:

```text
"Look at the platform and tell me what is happening."
```

Avoid making a conversational agent the only memory of how infra is created. Prefer:

```text
IaC/script in repo = source of truth
MCP/agent          = executor/inspector/helper
```

## Suggested Sandbox Architecture

For scary experiments, prefer an external disposable database branch, such as Neon:

```text
Neon branch DB
   |
   v
Railway sandbox env DATABASE_URL / ADMIN_DATABASE_URL
   |
   v
Bridge + HouseHold BE + HouseHold FE on CR branch
   |
   v
destructive test
   |
   v
destroy Railway sandbox + Neon branch
```

Why Neon-style DB branching is useful:

- fast disposable database creation;
- destructive tests do not touch staging/prod data;
- cleanup is clear;
- experiments can use a real PostgreSQL database without creating permanent Railway DB clutter.

Limitations:

- not identical to Railway Postgres networking/latency;
- not proof of PgBouncer production behavior unless PgBouncer is included in the sandbox;
- not a replacement for final staging/prod acceptance.

Use this for research and destructive tests. Use Railway staging/prod-like infrastructure for final launch confidence.

## Good Use Cases

Disposable Railway sandboxes are especially useful for:

- migration experiments;
- rollback rehearsal;
- account deletion dry-runs;
- Bridge callback and webhook failure testing;
- PgBouncer pool-size and TLS experiments;
- Clerk/Resend/Pub/Sub config experiments;
- destructive provider sandbox tests;
- load tests that should not pollute staging;
- branch-specific UI/API integration checks.

## Non-Goals

Do not use disposable sandboxes as:

- permanent staging replacements;
- production-readiness proof by themselves;
- long-lived forgotten mini-prod environments;
- places with real production provider secrets unless absolutely necessary;
- environments with custom domains/DNS unless the experiment specifically needs them.

## Safety Rules

- Name sandboxes clearly, e.g. `research-hh-pgbouncer-2026-07`.
- Prefer fake/sandbox provider credentials.
- Prefer generated secrets per sandbox.
- Never reuse production database URLs.
- Print variable names, not secret values, in diagnostics by default.
- Always record the CR branch and commit deployed.
- Destroy the sandbox when the experiment is over.
- Destroy the external DB branch after the Railway sandbox is gone.
- Keep production deploy manual regardless of sandbox automation.

## Prepared Prompt Ideas

Keep prompts short and operational. Examples:

```text
Inspect Railway sandbox <name>. Summarize latest deployment status for Bridge, HouseHold BE, and HouseHold FE. Include only service name, deployment status, latest deploy time, and the first actionable error if any. Do not print secrets.
```

```text
Grab the last 200 deploy logs and last 200 HTTP logs for sandbox <name>. Group findings into build failures, runtime failures, 4xx/5xx HTTP failures, and no-action noise. Do not modify anything.
```

```text
Compare staging and sandbox <name> variable names for HouseHold BE. Report missing or extra variable names only. Do not print values.
```

```text
After sandbox deploy <deployment-id>, check whether the service reached SUCCESS and whether /ready or /health responds. If not, return the smallest likely fix and the logs that support it.
```

```text
Summarize cost/risk for sandbox <name>: services running, databases/buckets/volumes attached, public domains, and anything that would keep billing alive after the experiment.
```

## Future Implementation Sketch

Phase 1 - document and manual practice:

- manually create one sandbox project or environment;
- use a Neon DB branch;
- deploy one CR branch;
- destroy everything;
- note the exact steps that were annoying.

Phase 2 - scripts:

- add `create-research-sandbox.ps1`;
- add `destroy-research-sandbox.ps1`;
- add `grab-logs.ps1`;
- keep them explicit and boring.

Phase 3 - Railway IaC:

- import or define baseline `.railway/railway.ts`;
- keep the project shape declarative;
- use `railway config plan` before `railway config apply`;
- do not use destructive apply without deliberate confirmation.

Phase 4 - agent/MCP playbook:

- save prompt templates;
- standardize log grabs;
- standardize health checks;
- standardize variable-name comparisons without secrets.

## Bottom Line

The sweet spot is:

```text
railway.ts = declarative baseline
scripts    = safe lifecycle buttons
MCP/agent  = eyes and diagnosis
Neon DB    = disposable data plane for scary tests
```

This gives fast experiments without letting staging become messy or production become scary.
