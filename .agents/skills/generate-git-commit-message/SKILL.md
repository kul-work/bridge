---
name: generate-git-commit-message
description: Analyzes git diffs and generates comprehensive commit messages with conventional commit prefixes (FIX, FEAT, DOCS, etc.). Use when you need to create professional git commit messages from code changes.
---

# Git Commit Message Generator

Automatically analyzes unstaged git changes and generates a professional commit message ready to use.

## What It Does

- **Runs `git diff`** automatically to fetch unstaged changes
- **Analyzes code changes** to determine impact and scope
- **Generates commit messages** with semantic prefixes (FIX, FEAT, REFACTOR, DOCS, CHORE, etc.)
- **Returns ONLY the message** formatted for immediate use
- **Follows conventional commits** specification for consistency

## Workflow

1. Make your code changes
2. Request a commit message: "Generate commit message for my changes"
3. Agent:
   - Runs `git diff` to analyze changes
   - Generates a conventional commit message
   - Returns formatted message only
4. Copy the message and run:
   ```bash
   git commit -m "YOUR_MESSAGE_HERE"
   ```

## Variants

**For staged changes only:**
- Request: "Generate commit message for staged changes"
- Agent runs: `git diff --cached`

**For a specific commit:**
- Request: "Generate commit message for commit abc123"
- Agent runs: `git show abc123`

## Commit Message Format

The skill generates messages in this format:

```
<TYPE> (<SCOPE>): <SUBJECT>

<BODY>

<FOOTER>
```

### Supported Types

- **FEAT**: New feature
- **FIX**: Bug fix
- **REFACTOR**: Code refactoring without feature/fix
- **PERF**: Performance improvement
- **DOCS**: Documentation changes only
- **STYLE**: Code style changes (formatting, semicolons, etc.)
- **TEST**: Test-only changes
- **CHORE**: Build, dependencies, tooling
- **CI**: CI/CD configuration changes
- **BUILD**: Build system changes

### Output Example

The agent returns **only** the commit message, ready to copy:

```
FIX(validation): Reject invalid purchase tokens in STRICT mode

- Add token format validation to reject obviously fake tokens
- Tokens containing "invalid", "not-a-valid" keywords are rejected
- Pure numeric tokens (all digits) are rejected as suspicious
- Real tokens must match expected patterns (test tokens, Google Play tokens)

Fixes ERR-01 test where malformed tokens were incorrectly accepted.
```

Then you run:
```bash
git commit -m "FIX(validation): Reject invalid purchase tokens in STRICT mode

- Add token format validation to reject obviously fake tokens
- Tokens containing \"invalid\", \"not-a-valid\" keywords are rejected
- Pure numeric tokens (all digits) are rejected as suspicious
- Real tokens must match expected patterns (test tokens, Google Play tokens)

Fixes ERR-01 test where malformed tokens were incorrectly accepted."
```

## Analysis Details

The agent analyzes:
- **File changes**: Which files were modified and their purpose
- **Function/class changes**: What methods/functions were added/modified
- **Test changes**: Whether tests were added or modified
- **Config changes**: Environment, build, or deployment configuration
- **Dependencies**: New imports, external packages

## Usage Instructions for Agent

When invoked, the agent MUST:

1. **Run git diff** in the current working directory
   - Use `git diff` for unstaged changes (default)
   - Use `git diff --cached` if user requests staged changes
   - Use `git show <hash>` if user specifies a commit

2. **Analyze the diff** to determine:
   - Type (FEAT, FIX, REFACTOR, etc.)
   - Scope (affected module/component)
   - Files changed
   - Semantic meaning of changes

3. **Generate message** following format:
   ```
   <TYPE> (<SCOPE>): <SHORT_DESCRIPTION>
   
   <BULLET_POINT_DETAILS>
   
   <OPTIONAL_FOOTER>
   ```

4. **Return ONLY the message text** (nothing else)
   - No explanations
   - No instructions
   - No analysis output
   - Just the commit message ready to use

## Example Requests

User says: "Generate commit message for my changes"
→ Agent runs `git diff`, returns message

User says: "Generate commit message for staged changes"
→ Agent runs `git diff --cached`, returns message

User says: "Generate commit message for abc123"
→ Agent runs `git show abc123`, returns message
