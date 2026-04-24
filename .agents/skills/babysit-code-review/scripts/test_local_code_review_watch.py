import argparse
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("local_code_review_watch.py")
MODULE_SPEC = importlib.util.spec_from_file_location("local_code_review_watch", MODULE_PATH)
local_code_review_watch = importlib.util.module_from_spec(MODULE_SPEC)
assert MODULE_SPEC.loader is not None
MODULE_SPEC.loader.exec_module(local_code_review_watch)


def sample_workspace():
    return {
        "repo_root": "c:/share/tyde/bridge",
        "repo_name": "bridge",
        "branch": "feature/local-review",
        "head_sha": "abc123",
    }


def sample_diff(has_changes=True):
    return {
        "mode": "all",
        "has_changes": has_changes,
        "file_count": 1 if has_changes else 0,
        "files": ["src/main.rs"] if has_changes else [],
    }


def sample_checks(failed_count=0, passed_count=1):
    return {
        "configured_count": failed_count + passed_count,
        "passed_count": passed_count,
        "failed_count": failed_count,
        "all_passed": failed_count == 0,
        "results": [],
    }


class LocalCodeReviewWatchTests(unittest.TestCase):
    def test_recommend_actions_prioritizes_review_feedback(self):
        actions = local_code_review_watch.recommend_actions(
            sample_diff(has_changes=True),
            sample_checks(failed_count=1, passed_count=0),
            [{"id": "fit:1", "title": "Architectural drift"}],
        )
        self.assertEqual(actions, ["process_review_feedback", "diagnose_check_failure"])

    def test_recommend_actions_marks_ready_when_diff_clean_of_issues(self):
        actions = local_code_review_watch.recommend_actions(
            sample_diff(has_changes=True),
            sample_checks(failed_count=0, passed_count=2),
            [],
        )
        self.assertEqual(actions, ["ready_for_local_review"])

    def test_code_fit_reject_becomes_review_items(self):
        report = {
            "decision": "reject",
            "reasons": [
                {"type": "layer_violation", "explanation": "handler owns business logic"},
                {"type": "duplication", "explanation": "logic duplicated"},
            ],
            "rewrite_guidance": ["Move logic to application", "Reuse existing flow"],
        }
        items = local_code_review_watch.review_items_from_code_fit_report(report, "fit.json")
        self.assertEqual(len(items), 2)
        self.assertEqual(items[0]["kind"], "code_fit_reject")
        self.assertIn("Guidance: Move logic to application", items[0]["body"])

    def test_collect_snapshot_surfaces_existing_review_items_on_fresh_state(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_path = Path(tmp_dir)
            review_path = tmp_path / "review.json"
            review_path.write_text(
                json.dumps(
                    {
                        "items": [
                            {
                                "id": "r1",
                                "title": "Fix naming",
                                "body": "Variable name is unclear",
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )

            args = argparse.Namespace(
                repo_root=None,
                diff="all",
                check=[],
                review_file=[str(review_path)],
                state_file=str(tmp_path / "state.json"),
                poll_seconds=30,
                check_timeout_seconds=60,
                once=True,
                watch=False,
            )

            with mock.patch.object(local_code_review_watch, "resolve_repo_root", return_value=tmp_path), mock.patch.object(
                local_code_review_watch, "get_workspace", return_value=sample_workspace()
            ), mock.patch.object(local_code_review_watch, "get_changed_files", return_value=sample_diff(has_changes=True)), mock.patch.object(
                local_code_review_watch, "run_checks", return_value=sample_checks(failed_count=0, passed_count=1)
            ):
                snapshot = local_code_review_watch.collect_snapshot(args)

        self.assertEqual([item["id"] for item in snapshot["new_review_items"]], ["r1"])
        self.assertEqual(snapshot["actions"], ["process_review_feedback"])

    def test_run_watch_stops_when_local_review_is_ready(self):
        events = []
        snapshot = {
            "workspace": sample_workspace(),
            "diff": sample_diff(has_changes=True),
            "checks": sample_checks(failed_count=0, passed_count=2),
            "open_review_items": [],
            "new_review_items": [],
            "actions": ["ready_for_local_review"],
            "state_file": "state.json",
        }

        with mock.patch.object(local_code_review_watch, "collect_snapshot", return_value=snapshot), mock.patch.object(
            local_code_review_watch, "print_event", side_effect=lambda event, payload: events.append((event, payload))
        ):
            result = local_code_review_watch.run_watch(argparse.Namespace(poll_seconds=30))

        self.assertEqual(result, 0)
        self.assertEqual([name for name, _payload in events], ["snapshot", "stop"])


if __name__ == "__main__":
    unittest.main()
