import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace

import pytest


MODULE_PATH = Path(__file__).resolve().with_name("babysit_pr_watch.py")
MODULE_SPEC = importlib.util.spec_from_file_location("babysit_pr_watch", MODULE_PATH)
assert MODULE_SPEC.loader is not None
babysit_pr_watch = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(babysit_pr_watch)


def test_codex_watcher_entrypoint_routes_through_the_repository_adapter():
    repository_root = MODULE_PATH.resolve().parents[2]
    exposed_watcher = (
        repository_root
        / ".codex"
        / "skills"
        / "babysit-pr"
        / "scripts"
        / "gh_pr_watch.py"
    )

    assert exposed_watcher.resolve() == MODULE_PATH.resolve()


def sample_pr():
    return {
        "number": 123,
        "repo": "openai/codex",
    }


def test_resolved_comment_collection_paginates_threads_and_comments(monkeypatch):
    calls = []

    def fake_graphql(query, variables):
        calls.append((query, variables.copy()))
        if "reviewThreads" in query:
            if variables.get("cursor") is None:
                return {
                    "data": {"repository": {"pullRequest": {"reviewThreads": {
                        "nodes": [
                            {
                                "id": "thread-1",
                                "isResolved": True,
                                "comments": {
                                    "nodes": [{"databaseId": 20}],
                                    "pageInfo": {
                                        "hasNextPage": True,
                                        "endCursor": "next-comment",
                                    },
                                },
                            },
                            {"id": "thread-2", "isResolved": False, "comments": {}},
                        ],
                        "pageInfo": {"hasNextPage": True, "endCursor": "next-thread"},
                    }}}}
                }
            return {
                "data": {"repository": {"pullRequest": {"reviewThreads": {
                    "nodes": [{
                        "id": "thread-3",
                        "isResolved": True,
                        "comments": {
                            "nodes": [{"databaseId": 30}],
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                        },
                    }],
                    "pageInfo": {"hasNextPage": False, "endCursor": None},
                }}}}
            }
        thread_id = variables["threadId"]
        assert thread_id == "thread-1"
        assert variables.get("cursor") == "next-comment"
        return {"data": {"node": {"comments": {
            "nodes": [{"databaseId": 21}],
            "pageInfo": {"hasNextPage": False, "endCursor": None},
        }}}}

    monkeypatch.setattr(babysit_pr_watch, "graphql", fake_graphql)

    assert babysit_pr_watch.fetch_resolved_review_comment_ids("openai/codex", 123) == {
        "20",
        "21",
        "30",
    }
    assert len([call for call in calls if "reviewThreads" in call[0]]) == 2
    assert len([call for call in calls if "reviewThreads" not in call[0]]) == 1


def test_resolved_review_threads_do_not_surface_on_fresh_state(monkeypatch):
    review_comment = {
        "id": 20,
        "pull_request_review_id": 10,
        "user": {"login": "octocat"},
        "author_association": "MEMBER",
        "body": "Already addressed.",
        "created_at": "2026-06-08T10:00:00Z",
        "path": "src/example.rs",
        "line": 7,
        "html_url": "https://github.com/openai/codex/pull/123#discussion_r20",
    }

    def fake_list(endpoint, **kwargs):
        if endpoint.endswith("/pulls/123/comments"):
            return [review_comment]
        if endpoint.endswith("/issues/123/comments") or endpoint.endswith("/pulls/123/reviews"):
            return []
        raise AssertionError(f"unexpected endpoint: {endpoint}")

    monkeypatch.setattr(
        babysit_pr_watch,
        "fetch_resolved_review_comment_ids",
        lambda repo, number: {"20"},
    )
    monkeypatch.setattr(babysit_pr_watch.vendored, "gh_api_list_paginated", fake_list)

    assert (
        babysit_pr_watch.fetch_new_review_items(
            sample_pr(),
            {},
            fresh_state=True,
            authenticated_login="octocat",
        )
        == []
    )


def test_no_configured_checks_are_an_empty_check_set(monkeypatch):
    monkeypatch.setattr(
        babysit_pr_watch.subprocess,
        "run",
        lambda *args, **kwargs: SimpleNamespace(
            returncode=1,
            stdout="",
            stderr="no checks reported on the branch",
        ),
    )

    assert babysit_pr_watch.get_pr_checks("2", "openai/codex") == []


def test_pending_check_exit_code_preserves_the_json_payload(monkeypatch):
    pending_checks = [{"name": "tests", "state": "IN_PROGRESS", "bucket": "pending"}]

    def fake_run(command, **kwargs):
        assert command[:5] == ["gh", "-R", "openai/codex", "pr", "checks"]
        assert "2" in command
        return SimpleNamespace(
            returncode=8,
            stdout=json.dumps(pending_checks),
            stderr="",
        )

    monkeypatch.setattr(babysit_pr_watch.subprocess, "run", fake_run)

    assert babysit_pr_watch.get_pr_checks("2", "openai/codex") == pending_checks


def test_other_check_failures_are_not_hidden(monkeypatch):
    monkeypatch.setattr(
        babysit_pr_watch.subprocess,
        "run",
        lambda *args, **kwargs: SimpleNamespace(
            returncode=1,
            stdout="",
            stderr="authentication failed",
        ),
    )

    with pytest.raises(babysit_pr_watch.vendored.GhCommandError, match="authentication failed"):
        babysit_pr_watch.get_pr_checks("2", "openai/codex")
