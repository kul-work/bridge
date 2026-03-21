---
name: release-notes-update
description: Updates Release Notes by analyzing git commits since a given SHA. Examines commit messages to categorize changes (features, fixes, breaking changes) and recommends the appropriate cargo release command (patch, minor, or major). Use when preparing a new release.
---

# Release Notes Update Skill

Updates `Release Notes.md` with changes from recent git commits and recommends the appropriate version bump.

## Usage

```bash
/release-notes-update {GIT-SHA}
```

**Parameters:**
- `{GIT-SHA}`: Git commit SHA. The skill will analyze all commits **after** (more recent than) this SHA.
  - If omitted, the skill automatically finds the SHA of the latest git tag and uses that.

## How It Works

1. **Resolve SHA**: If SHA not provided, runs `git describe --tags --abbrev=0` to find latest tag, then `git rev-list -n 1 {TAG}` to get its SHA
2. **Analyze Commits**: Runs `git log {GIT-SHA}..HEAD` to get all commits since the specified SHA
3. **Categorize Changes**:
   - **Breaking Changes** (major): Commits with `BREAKING CHANGE:` or `!:` in message
   - **Features** (minor): Commits with `feat:` or `FEAT:`
   - **Fixes** (patch): Commits with `fix:` or `FIX:`
   - **Other**: Docs, refactoring, chores (informational only)
4. **Update Release Notes**: Prepends a new version entry with categorized changes to `Release Notes.md`
5. **Recommend Release Command**: Based on the highest severity change detected:
   - **Breaking changes detected** → `cargo release major --execute`
   - **Features detected** → `cargo release minor --execute`
   - **Fixes only** → `cargo release patch --execute`

## Workflow

**Before:**
```
Release Notes.md contains entries for v0.1.0, v0.0.9, etc.
```

**After running `/release-notes-update abc123def`:**
```
Release Notes.md now has:
- New entry at top with next version and categorized changes
- Recommended command printed: e.g., "RECOMMEND: cargo release minor --execute"
```

## Example Output

```
✓ Found 5 commits since abc123def
  - 1 breaking change
  - 2 features
  - 2 fixes

✓ Updated Release Notes.md

RECOMMEND:
  cargo release patch --execute    # 0.0.1 -> 0.0.2
```

## Notes

- Works in projects with `Release Notes.md` and `Cargo.toml`
- Commit messages should follow [Conventional Commits](https://www.conventionalcommits.org/) format
- The skill handles commits with or without the conventional format gracefully
- Always review the generated Release Notes before executing the cargo release command
