#!/usr/bin/env python3
"""Repository-local compatibility adapter for the vendored PR watcher."""

import importlib.util
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VENDORED_WATCHER = (
    REPOSITORY_ROOT
    / "vendor"
    / "openai-codex-skills"
    / "babysit-pr"
    / "scripts"
    / "gh_pr_watch.py"
)
MODULE_SPEC = importlib.util.spec_from_file_location("vendored_gh_pr_watch", VENDORED_WATCHER)
if MODULE_SPEC is None or MODULE_SPEC.loader is None:
    raise RuntimeError(f"Unable to load vendored watcher from {VENDORED_WATCHER}")
vendored = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(vendored)


def graphql(query, variables):
    args = ["api", "graphql", "-f", f"query={query}"]
    for name, value in variables.items():
        if value is not None:
            args.extend(["-F", f"{name}={value}"])
    return vendored.gh_json(args)


def connection(payload, *path):
    value = payload
    try:
        for component in path:
            value = value[component]
    except (KeyError, TypeError) as err:
        joined = ".".join(path)
        raise vendored.GhCommandError(f"Unexpected GraphQL connection at {joined}") from err
    if not isinstance(value, dict):
        joined = ".".join(path)
        raise vendored.GhCommandError(f"Unexpected GraphQL connection at {joined}")
    return value


def paginated_connection(query, variables, *path, initial_cursor=None):
    cursor = initial_cursor
    while True:
        payload = graphql(query, {**variables, "cursor": cursor})
        current = connection(payload, *path)
        yield from current.get("nodes") or []
        page_info = current.get("pageInfo") or {}
        if not page_info.get("hasNextPage"):
            return
        cursor = page_info.get("endCursor")
        if not cursor:
            raise vendored.GhCommandError("GraphQL pagination omitted endCursor")


def fetch_resolved_review_comment_ids(repo, pr_number):
    parts = repo.split("/")
    if len(parts) != 2 or not all(parts):
        raise vendored.GhCommandError(f"Invalid repository slug: {repo}")
    owner, name = parts
    thread_query = (
        "query($owner:String!,$name:String!,$number:Int!,$cursor:String){"
        "repository(owner:$owner,name:$name){pullRequest(number:$number){"
        "reviewThreads(first:100,after:$cursor){nodes{id isResolved "
        "comments(first:100){nodes{databaseId} pageInfo{hasNextPage endCursor}}}"
        "pageInfo{hasNextPage endCursor}}}}}"
    )
    comment_query = (
        "query($threadId:ID!,$cursor:String){node(id:$threadId){"
        "... on PullRequestReviewThread{comments(first:100,after:$cursor){"
        "nodes{databaseId} pageInfo{hasNextPage endCursor}}}}}"
    )

    resolved_comment_ids = set()
    remaining_comment_pages = []
    for thread in paginated_connection(
        thread_query,
        {"owner": owner, "name": name, "number": pr_number},
        "data",
        "repository",
        "pullRequest",
        "reviewThreads",
    ):
        if isinstance(thread, dict) and thread.get("isResolved") and thread.get("id"):
            comments = thread.get("comments") or {}
            for comment in comments.get("nodes") or []:
                if isinstance(comment, dict) and comment.get("databaseId") not in (None, ""):
                    resolved_comment_ids.add(str(comment["databaseId"]))
            page_info = comments.get("pageInfo") or {}
            if page_info.get("hasNextPage"):
                cursor = page_info.get("endCursor")
                if not cursor:
                    raise vendored.GhCommandError("GraphQL pagination omitted endCursor")
                remaining_comment_pages.append((str(thread["id"]), cursor))

    for thread_id, cursor in remaining_comment_pages:
        for comment in paginated_connection(
            comment_query,
            {"threadId": thread_id},
            "data",
            "node",
            "comments",
            initial_cursor=cursor,
        ):
            if isinstance(comment, dict) and comment.get("databaseId") not in (None, ""):
                resolved_comment_ids.add(str(comment["databaseId"]))
    return resolved_comment_ids


_vendored_fetch_new_review_items = vendored.fetch_new_review_items
_vendored_normalize_review_comments = vendored.normalize_review_comments
_vendored_get_pr_checks = vendored.get_pr_checks


def fetch_new_review_items(pr, state, fresh_state, authenticated_login=None):
    resolved_ids = fetch_resolved_review_comment_ids(pr["repo"], pr["number"])

    def normalize_unresolved_review_comments(items, review_states):
        return [
            item
            for item in _vendored_normalize_review_comments(items, review_states)
            if item["id"] not in resolved_ids
        ]

    previous_normalizer = vendored.normalize_review_comments
    vendored.normalize_review_comments = normalize_unresolved_review_comments
    try:
        return _vendored_fetch_new_review_items(
            pr,
            state,
            fresh_state,
            authenticated_login=authenticated_login,
        )
    finally:
        vendored.normalize_review_comments = previous_normalizer


def get_pr_checks(pr_spec, repo):
    try:
        return _vendored_get_pr_checks(pr_spec, repo)
    except vendored.GhCommandError as err:
        if "no checks reported" in str(err).lower():
            return []
        raise


vendored.fetch_new_review_items = fetch_new_review_items
vendored.get_pr_checks = get_pr_checks


def main():
    return vendored.main()


if __name__ == "__main__":
    raise SystemExit(main())
