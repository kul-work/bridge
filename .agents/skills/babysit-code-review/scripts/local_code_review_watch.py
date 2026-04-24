#!/usr/bin/env python
"""Watch a local worktree like a code review queue."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

RESOLVED_REVIEW_STATUSES = {"resolved", "closed", "done", "ignored"}


class WatcherError(RuntimeError):
    pass


def parse_args():
    parser = argparse.ArgumentParser(
        description="Normalize local diff, check, and review state for code review babysitting."
    )
    parser.add_argument("--repo-root", help="Optional repository root override")
    parser.add_argument(
        "--diff",
        choices=["worktree", "staged", "all"],
        default="all",
        help="Which local diff scope to review",
    )
    parser.add_argument(
        "--check",
        action="append",
        default=[],
        help="Local verification command to run. Repeat for multiple checks.",
    )
    parser.add_argument(
        "--review-file",
        action="append",
        default=[],
        help="JSON file with local review items or a code-fit-evaluator result. Repeatable.",
    )
    parser.add_argument("--state-file", help="Path to watcher state JSON file")
    parser.add_argument(
        "--poll-seconds",
        type=int,
        default=30,
        help="Watch poll interval in seconds",
    )
    parser.add_argument(
        "--check-timeout-seconds",
        type=int,
        default=900,
        help="Timeout for each local check command",
    )
    parser.add_argument("--once", action="store_true", help="Emit one snapshot and exit")
    parser.add_argument("--watch", action="store_true", help="Continuously emit JSONL snapshots")
    args = parser.parse_args()

    if args.poll_seconds <= 0:
        parser.error("--poll-seconds must be > 0")
    if args.check_timeout_seconds <= 0:
        parser.error("--check-timeout-seconds must be > 0")
    if not args.once and not args.watch:
        args.once = True
    return args


def run_text(command, cwd):
    try:
        proc = subprocess.run(
            command,
            cwd=str(cwd),
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as err:
        raise WatcherError(f"command not found: {command[0]}") from err
    except subprocess.CalledProcessError as err:
        stdout = (err.stdout or "").strip()
        stderr = (err.stderr or "").strip()
        pieces = [f"command failed: {' '.join(command)}"]
        if stdout:
            pieces.append(f"stdout: {stdout}")
        if stderr:
            pieces.append(f"stderr: {stderr}")
        raise WatcherError("\n".join(pieces)) from err
    return proc.stdout


def resolve_repo_root(repo_root_override=None):
    cwd = Path(repo_root_override or os.getcwd())
    output = run_text(["git", "rev-parse", "--show-toplevel"], cwd=cwd).strip()
    if not output:
        raise WatcherError("unable to determine git repository root")
    return Path(output)


def git_lines(repo_root, args):
    output = run_text(["git", *args], cwd=repo_root)
    return [line.strip() for line in output.splitlines() if line.strip()]


def get_workspace(repo_root):
    branch = run_text(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=repo_root).strip()
    head_sha = run_text(["git", "rev-parse", "HEAD"], cwd=repo_root).strip()
    return {
        "repo_root": str(repo_root),
        "repo_name": repo_root.name,
        "branch": branch,
        "head_sha": head_sha,
    }


def get_changed_files(repo_root, diff_mode):
    files = set()
    if diff_mode in {"worktree", "all"}:
        files.update(git_lines(repo_root, ["diff", "--name-only"]))
        files.update(git_lines(repo_root, ["ls-files", "--others", "--exclude-standard"]))
    if diff_mode in {"staged", "all"}:
        files.update(git_lines(repo_root, ["diff", "--cached", "--name-only"]))
    ordered = sorted(files)
    return {
        "mode": diff_mode,
        "has_changes": bool(ordered),
        "file_count": len(ordered),
        "files": ordered,
    }


def default_state_file_for(workspace):
    name = workspace["repo_name"]
    temp_dir = Path(tempfile.gettempdir())
    return temp_dir / f"amp-babysit-code-review-{name}.json"


def load_state(path):
    if path.exists():
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as err:
            raise WatcherError(f"state file is not valid JSON: {path}") from err
        if not isinstance(data, dict):
            raise WatcherError(f"state file must contain an object: {path}")
        return data, False
    return {
        "started_at": None,
        "last_snapshot_at": None,
        "last_seen_head_sha": None,
        "seen_review_item_ids": [],
    }, True


def save_state(path, state):
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(state, indent=2, sort_keys=True) + "\n"
    path.write_text(payload, encoding="utf-8")


def summarize_text(text, max_lines=20, max_chars=3000):
    clipped = text[:max_chars]
    lines = clipped.splitlines()
    if len(lines) > max_lines:
        lines = lines[:max_lines]
    return "\n".join(lines).strip()


def run_check_command(command, repo_root, timeout_seconds):
    try:
        proc = subprocess.run(
            command,
            cwd=str(repo_root),
            shell=True,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
        )
        output = "\n".join(part for part in [proc.stdout, proc.stderr] if part).strip()
        return {
            "command": command,
            "ok": proc.returncode == 0,
            "exit_code": proc.returncode,
            "timed_out": False,
            "output_excerpt": summarize_text(output),
        }
    except subprocess.TimeoutExpired as err:
        stdout = err.stdout or ""
        stderr = err.stderr or ""
        output = "\n".join(part for part in [stdout, stderr] if part).strip()
        return {
            "command": command,
            "ok": False,
            "exit_code": None,
            "timed_out": True,
            "output_excerpt": summarize_text(output),
        }


def run_checks(commands, repo_root, timeout_seconds):
    results = [run_check_command(command, repo_root, timeout_seconds) for command in commands]
    failed_count = sum(1 for item in results if not item["ok"])
    passed_count = sum(1 for item in results if item["ok"])
    return {
        "configured_count": len(commands),
        "passed_count": passed_count,
        "failed_count": failed_count,
        "all_passed": failed_count == 0,
        "results": results,
    }


def stable_review_id(item):
    explicit = item.get("id")
    if explicit not in (None, ""):
        return str(explicit)
    raw = json.dumps(item, sort_keys=True, ensure_ascii=True)
    return hashlib.sha1(raw.encode("utf-8")).hexdigest()


def normalize_review_item(item, source):
    if not isinstance(item, dict):
        return None
    status = str(item.get("status") or "open").strip().lower()
    if status in RESOLVED_REVIEW_STATUSES:
        return None
    normalized = {
        "id": stable_review_id(item),
        "source": source,
        "kind": str(item.get("kind") or "local_review"),
        "title": str(item.get("title") or "").strip(),
        "body": str(item.get("body") or item.get("explanation") or "").strip(),
        "path": item.get("path"),
        "line": item.get("line"),
        "status": status,
    }
    return normalized


def review_items_from_code_fit_report(data, source):
    decision = str(data.get("decision") or "").strip().lower()
    if decision != "reject":
        return []
    reasons = data.get("reasons") or []
    rewrite_guidance = data.get("rewrite_guidance") or []
    items = []
    for index, reason in enumerate(reasons, start=1):
        if not isinstance(reason, dict):
            continue
        explanation = str(reason.get("explanation") or "").strip()
        if not explanation:
            continue
        guidance = ""
        if index - 1 < len(rewrite_guidance):
            guidance = str(rewrite_guidance[index - 1] or "").strip()
        body = explanation
        if guidance:
            body = f"{body} Guidance: {guidance}"
        item = {
            "id": f"code-fit:{index}:{reason.get('type') or 'reason'}",
            "kind": "code_fit_reject",
            "title": f"Code fit rejected ({reason.get('type') or 'issue'})",
            "body": body,
            "status": "open",
        }
        normalized = normalize_review_item(item, source=source)
        if normalized:
            items.append(normalized)
    return items


def load_review_items(paths):
    items = []
    for raw_path in paths:
        path = Path(raw_path)
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except FileNotFoundError as err:
            raise WatcherError(f"review file not found: {path}") from err
        except json.JSONDecodeError as err:
            raise WatcherError(f"review file is not valid JSON: {path}") from err

        source = str(path)
        if isinstance(data, dict) and "decision" in data and "reasons" in data:
            items.extend(review_items_from_code_fit_report(data, source))
            continue

        raw_items = data
        if isinstance(data, dict):
            raw_items = data.get("items") or []
        if not isinstance(raw_items, list):
            raise WatcherError(f"review file must contain a list or object with items: {path}")

        for raw_item in raw_items:
            normalized = normalize_review_item(raw_item, source=source)
            if normalized:
                items.append(normalized)
    items.sort(key=lambda item: (item["source"], item["title"], item["id"]))
    return items


def new_review_items(all_review_items, state):
    seen = {str(item) for item in state.get("seen_review_item_ids") or []}
    fresh = []
    for item in all_review_items:
        item_id = str(item.get("id") or "")
        if not item_id:
            continue
        if item_id not in seen:
            fresh.append(item)
            seen.add(item_id)
    state["seen_review_item_ids"] = sorted(seen)
    return fresh


def recommend_actions(diff_summary, checks_summary, open_review_items):
    actions = []
    if open_review_items:
        actions.append("process_review_feedback")
    if checks_summary["failed_count"] > 0:
        actions.append("diagnose_check_failure")
    if not diff_summary["has_changes"]:
        if not open_review_items and checks_summary["failed_count"] == 0:
            actions.append("stop_no_changes")
    elif not open_review_items and checks_summary["failed_count"] == 0:
        actions.append("ready_for_local_review")
    if not actions:
        actions.append("idle")
    return actions


def collect_snapshot(args):
    repo_root = resolve_repo_root(args.repo_root)
    workspace = get_workspace(repo_root)
    state_path = Path(args.state_file) if args.state_file else default_state_file_for(workspace)
    state, _fresh_state = load_state(state_path)
    if not state.get("started_at"):
        state["started_at"] = int(time.time())

    diff_summary = get_changed_files(repo_root, args.diff)
    checks_summary = run_checks(args.check, repo_root, args.check_timeout_seconds)
    open_review_items = load_review_items(args.review_file)
    surfaced_review_items = new_review_items(open_review_items, state)
    actions = recommend_actions(diff_summary, checks_summary, open_review_items)

    state["last_snapshot_at"] = int(time.time())
    state["last_seen_head_sha"] = workspace["head_sha"]
    save_state(state_path, state)

    return {
        "workspace": workspace,
        "diff": diff_summary,
        "checks": checks_summary,
        "open_review_items": open_review_items,
        "new_review_items": surfaced_review_items,
        "actions": actions,
        "state_file": str(state_path),
    }


def print_json(obj):
    sys.stdout.write(json.dumps(obj, sort_keys=True) + "\n")
    sys.stdout.flush()


def print_event(event, payload):
    print_json({"event": event, "payload": payload})


def run_watch(args):
    while True:
        snapshot = collect_snapshot(args)
        print_event(
            "snapshot",
            {
                "snapshot": snapshot,
                "next_poll_seconds": args.poll_seconds,
            },
        )
        actions = set(snapshot.get("actions") or [])
        if "ready_for_local_review" in actions or "stop_no_changes" in actions:
            print_event("stop", {"actions": snapshot.get("actions"), "workspace": snapshot.get("workspace")})
            return 0

        time.sleep(args.poll_seconds)


def main():
    args = parse_args()
    try:
        if args.watch:
            return run_watch(args)
        print_json(collect_snapshot(args))
        return 0
    except WatcherError as err:
        sys.stderr.write(f"local_code_review_watch.py error: {err}\n")
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
