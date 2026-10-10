#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
xshell = "0.2"

[dev-dependencies]
tempfile = "3"
---

//! Run from the repository with `python scripts/run.py branch-cleanup [--all]
//! [--dry-run]`; test with `CARGO_ENCODED_RUSTFLAGS= cargo +nightly -Zscript
//! test --manifest-path scripts/branch-cleanup.rs`. Both clear the repository's
//! rustflags, whose relative linker path does not resolve from `scripts/`.

use std::{
    collections::HashSet,
    io::Write,
};

use anyhow::{
    Result,
    bail,
    ensure,
};
use clap::Parser;
use serde::{
    Deserialize,
    Serialize,
};
use xshell::{
    Shell,
    cmd,
};

const PROTECTED_BRANCHES: &[&str] = &["main", "master", "develop", "dev", "production", "staging"];
const KEEP: &str = "active PR, extra commits, or insufficient cleanup evidence";

/// Switch to main, pull with --ff-only, and delete the latest merged PR's
/// local branch.
///
/// Requires Git and an authenticated GitHub CLI (gh). Only local branches are
/// deleted.
#[derive(Parser)]
struct Options {
    /// Also clean up finished PR branches and fully merged branches with no
    /// live upstream.
    #[arg(long)]
    all:     bool,
    /// Fetch/prune remote refs and preview; do not switch, pull, or delete
    /// local branches.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum State {
    Open,
    Closed,
    Merged,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PullRequest {
    number:              u64,
    state:               State,
    merged_at:           Option<String>,
    head_ref_name:       String,
    head_ref_oid:        String,
    base_ref_name:       String,
    is_cross_repository: bool,
}

struct LocalBranch {
    name:          String,
    sha:           String,
    current:       bool,
    upstream_gone: bool,
    worktree:      String,
}

fn load_pull_requests(sh: &Shell) -> Result<Vec<PullRequest>> {
    let origin = cmd!(sh, "git remote get-url origin").read()?;
    let fields = "number,state,mergedAt,headRefName,headRefOid,baseRefName,isCrossRepository";
    let list = cmd!(
        sh,
        "gh pr list --repo {origin} --state all --limit 100000 --json {fields}"
    );
    Ok(serde_json::from_str(&list.read()?)?)
}

fn load_branches(sh: &Shell) -> Result<Vec<LocalBranch>> {
    let format = "%(refname:short)%00%(objectname)%00%(HEAD)%00%(upstream)%00%(upstream:track)%00%\
                  (worktreepath)";
    let refs = cmd!(sh, "git for-each-ref --format={format} refs/heads/").read()?;
    refs.lines()
        .map(|line| {
            let [name, sha, head, upstream, track, worktree] =
                line.split('\0').collect::<Vec<_>>()[..]
            else {
                bail!("unexpected git for-each-ref line {line:?}");
            };
            Ok(LocalBranch {
                name:          name.to_owned(),
                sha:           sha.to_owned(),
                current:       head == "*",
                upstream_gone: upstream.is_empty() || track == "[gone]",
                worktree:      worktree.to_owned(),
            })
        })
        .collect()
}

/// Returns why `branch` may be deleted, or why it is kept. In latest-merge
/// mode, `latest` is the only merged PR that may authorize deletion.
fn decide(
    branch: &LocalBranch,
    pull_requests: &[&PullRequest],
    merged_into_main: bool,
    latest: Option<&PullRequest>,
) -> Result<String, &'static str> {
    if PROTECTED_BRANCHES.contains(&branch.name.as_str()) {
        return Err("protected branch");
    }
    // A dry run stays on the current branch, so its own checkout does not block
    // the preview.
    if !branch.worktree.is_empty() && !branch.current {
        return Err("checked out in a worktree");
    }
    if latest.is_some_and(|pr| pr.is_cross_repository) {
        return Err("the latest merged PR came from another repository");
    }
    // A fork's branch name must never authorize deleting a same-named local
    // branch.
    let related: Vec<_> = pull_requests
        .iter()
        .filter(|pr| !pr.is_cross_repository && pr.head_ref_name == branch.name)
        .collect();
    if related.iter().any(|pr| pr.state == State::Open) {
        return Err(KEEP);
    }
    // Compare the tip as well as the name: a merged branch may have been
    // reused.
    let into_main = |pr: &PullRequest| pr.state == State::Merged && pr.base_ref_name == "main";
    if let Some(pr) = related
        .iter()
        .find(|pr| into_main(pr) && pr.head_ref_oid == branch.sha)
    {
        return Ok(format!("merged PR #{}", pr.number));
    }
    if let Some(pr) = related
        .iter()
        .find(|pr| pr.state == State::Closed && pr.head_ref_oid == branch.sha)
    {
        return Ok(format!("closed PR #{} (no extra local commits)", pr.number));
    }
    if merged_into_main && related.iter().any(|pr| into_main(pr)) {
        return Ok("PR branch already contained in main".to_owned());
    }
    // A missing upstream alone is insufficient evidence that work is
    // disposable.
    if merged_into_main && branch.upstream_gone {
        return Ok("fully merged branch without a live upstream".to_owned());
    }
    Err(KEEP)
}

fn cleanup(
    sh: &Shell,
    options: &Options,
    out: &mut impl Write,
    load_pull_requests: impl FnOnce() -> Result<Vec<PullRequest>>,
) -> Result<()> {
    ensure!(
        options.dry_run || cmd!(sh, "git status --porcelain").read()?.is_empty(),
        "Commit or stash local changes before switching to main and cleaning branches."
    );
    writeln!(out, "Reading pull requests from GitHub...")?;
    let pull_requests = load_pull_requests()?;
    // GitHub does not list pull requests by merge time, so choose by mergedAt.
    let latest_merged = pull_requests
        .iter()
        .filter(|pr| pr.state == State::Merged && pr.base_ref_name == "main")
        .max_by_key(|pr| &pr.merged_at);
    ensure!(
        options.all || latest_merged.is_some(),
        "No merged pull request targeting main was found."
    );

    cmd!(sh, "git fetch --prune origin").run()?;
    if !options.dry_run {
        cmd!(sh, "git switch main").run()?;
        cmd!(sh, "git pull --ff-only origin main").run()?;
    }
    let merged = cmd!(
        sh,
        "git for-each-ref --merged=refs/remotes/origin/main --format=%(refname:short) refs/heads/"
    );
    let merged_into_main: HashSet<_> = merged.read()?.lines().map(str::to_owned).collect();

    // Latest-merge mode considers only the latest merged PR and open PRs.
    let latest = latest_merged.filter(|_| !options.all);
    let relevant: Vec<_> = pull_requests
        .iter()
        .filter(|pr| {
            pr.state == State::Open || latest.is_none_or(|latest| std::ptr::eq(*pr, latest))
        })
        .collect();
    let candidates: Vec<_> = load_branches(sh)?
        .into_iter()
        .filter(|branch| latest.is_none_or(|pr| pr.head_ref_name == branch.name))
        .collect();

    let mut deleted = 0;
    for branch in &candidates {
        let name = &branch.name;
        match decide(branch, &relevant, merged_into_main.contains(name), latest) {
            | Err(skip) => {
                writeln!(out, "Skipping {name}: {skip}.")?;
                continue;
            },
            | Ok(reason) if options.dry_run => writeln!(out, "Would delete {name}: {reason}.")?,
            | Ok(_) => cmd!(sh, "git branch -D -- {name}").run()?,
        }
        deleted += 1;
    }
    if let (true, Some(pr)) = (candidates.is_empty(), latest) {
        writeln!(
            out,
            "No local branch for the latest merged PR #{} ({}).",
            pr.number, pr.head_ref_name
        )?;
    }
    let verb = if options.dry_run {
        "Would delete"
    } else {
        "Deleted"
    };
    writeln!(out, "{verb} {deleted} local branch(es).")?;
    Ok(())
}

fn main() -> Result<()> {
    let options = Options::parse();
    let sh = Shell::new()?;
    cleanup(&sh, &options, &mut std::io::stdout(), || {
        load_pull_requests(&sh)
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    const SQUASHED: &str = "feature/squashed";

    struct Fixture {
        _root:   TempDir,
        sh:      Shell,
        initial: String,
        prs:     Vec<PullRequest>,
    }

    impl Fixture {
        /// Creates a local clone of a bare `origin` with one commit on main.
        fn new() -> Result<Self> {
            let root = tempfile::Builder::new()
                .prefix("bcc-rust-branch-cleanup-")
                .tempdir()?;
            let sh = Shell::new()?;
            // Keep the user's Git configuration, signing, and hooks out of the
            // test.
            for (key, value) in [
                ("GIT_CONFIG_GLOBAL", "nonexistent"),
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_AUTHOR_NAME", "Branch Cleanup Test"),
                ("GIT_AUTHOR_EMAIL", "branch-cleanup@example.invalid"),
                ("GIT_COMMITTER_NAME", "Branch Cleanup Test"),
                ("GIT_COMMITTER_EMAIL", "branch-cleanup@example.invalid"),
            ] {
                sh.set_var(key, value);
            }
            sh.change_dir(root.path());
            cmd!(sh, "git init --bare remote.git").run()?;
            cmd!(sh, "git init -b main local").run()?;
            sh.change_dir("local");
            cmd!(sh, "git commit --allow-empty -m initial").run()?;
            cmd!(sh, "git remote add origin ../remote.git").run()?;
            cmd!(sh, "git push -u origin main").run()?;
            let initial = cmd!(sh, "git rev-parse HEAD").read()?;
            Ok(Self {
                _root: root,
                sh,
                initial,
                prs: Vec::new(),
            })
        }

        /// Creates a fixture whose `feature/squashed` was squash-merged by PR
        /// #1.
        fn merged() -> Result<Self> {
            let mut fixture = Self::new()?;
            fixture.merge(SQUASHED, 1, "2026-10-01T10:00:00Z")?;
            Ok(fixture)
        }

        fn commit(&self, file: &str) -> Result<String> {
            self.sh.write_file(file, file)?;
            cmd!(self.sh, "git add -- {file}").run()?;
            cmd!(self.sh, "git commit -m {file}").run()?;
            Ok(cmd!(self.sh, "git rev-parse HEAD").read()?)
        }

        /// Squash-merges a new pushed branch and leaves it checked out.
        fn merge(&mut self, name: &str, number: u64, merged_at: &str) -> Result<()> {
            let sh = &self.sh;
            cmd!(sh, "git switch -c {name} main").run()?;
            let sha = self.commit(&format!("pr-{number}.txt"))?;
            cmd!(sh, "git push -u origin {name}").run()?;
            cmd!(sh, "git switch main").run()?;
            cmd!(sh, "git merge --squash {name}").run()?;
            cmd!(sh, "git commit -m squash").run()?;
            cmd!(sh, "git push origin main").run()?;
            cmd!(sh, "git switch {name}").run()?;
            self.prs.push(PullRequest {
                number,
                state: State::Merged,
                merged_at: Some(merged_at.to_owned()),
                head_ref_name: name.to_owned(),
                head_ref_oid: sha,
                base_ref_name: "main".to_owned(),
                is_cross_repository: false,
            });
            Ok(())
        }

        fn cleanup(&self, args: &[&str]) -> Result<String> {
            let options = Options::try_parse_from(["branch-cleanup"].iter().chain(args))?;
            let mut out = Vec::new();
            cleanup(&self.sh, &options, &mut out, || Ok(self.prs.clone()))?;
            Ok(String::from_utf8(out)?)
        }

        fn branches(&self) -> Result<Vec<String>> {
            let branches = cmd!(self.sh, "git branch --format=%(refname:short)").read()?;
            Ok(branches.lines().map(str::to_owned).collect())
        }

        fn has_branch(&self, name: &str) -> Result<bool> {
            Ok(self.branches()?.iter().any(|branch| branch == name))
        }

        fn current(&self) -> Result<String> {
            Ok(cmd!(self.sh, "git branch --show-current").read()?)
        }
    }

    #[test]
    fn latest_merge_timestamp_wins_including_squash_merges_after_main_is_pulled() -> Result<()> {
        let mut f = Fixture::new()?;
        f.merge("feature/newer", 1, "2026-10-01T10:00:00Z")?;
        f.merge("feature/older", 2, "2026-09-30T10:00:00Z")?;
        let (sh, initial) = (&f.sh, &f.initial);
        // The local main really needs a pull.
        cmd!(sh, "git branch -f main {initial}").run()?;

        f.cleanup(&[])?;

        assert_eq!(f.current()?, "main");
        assert_eq!(f.branches()?, ["feature/older", "main"]);
        assert_eq!(
            cmd!(sh, "git rev-parse main").read()?,
            cmd!(sh, "git rev-parse origin/main").read()?
        );
        Ok(())
    }

    #[test]
    fn dry_run_leaves_the_checkout_main_and_all_local_branches_intact() -> Result<()> {
        let f = Fixture::merged()?;
        let (sh, initial) = (&f.sh, &f.initial);
        cmd!(sh, "git branch -f main {initial}").run()?;
        sh.write_file("uncommitted.txt", "work in progress")?;
        let before = f.branches()?;

        let output = f.cleanup(&["--dry-run"])?;

        assert_eq!(f.branches()?, before);
        assert_eq!(f.current()?, SQUASHED);
        assert_eq!(&cmd!(sh, "git rev-parse main").read()?, initial);
        assert!(
            output.contains("Would delete feature/squashed: merged PR #1"),
            "{output}"
        );
        Ok(())
    }

    #[test]
    fn reused_merged_branches_with_new_local_commits_survive_cleanup() -> Result<()> {
        let f = Fixture::merged()?;
        f.commit("new-unpushed-work.txt")?;

        let output = f.cleanup(&["--all"])?;

        assert!(f.has_branch(SQUASHED)?);
        assert!(output.contains("extra commits"), "{output}");
        Ok(())
    }

    #[test]
    fn latest_merge_mode_keeps_later_work_from_a_closed_pr_on_the_same_branch() -> Result<()> {
        let mut f = Fixture::merged()?;
        let sha = f.commit("later-closed-pr.txt")?;
        f.prs.push(PullRequest {
            number: 2,
            state: State::Closed,
            merged_at: None,
            head_ref_oid: sha,
            ..f.prs[0].clone()
        });

        let output = f.cleanup(&[])?;

        assert!(f.has_branch(SQUASHED)?);
        assert!(output.contains("extra commits"), "{output}");
        Ok(())
    }

    #[test]
    fn an_open_pr_for_a_reused_branch_preserves_it_even_if_the_tip_matches_an_old_merge()
    -> Result<()> {
        let mut f = Fixture::merged()?;
        f.prs.push(PullRequest {
            number: 2,
            state: State::Open,
            merged_at: None,
            base_ref_name: "release".to_owned(),
            ..f.prs[0].clone()
        });

        f.cleanup(&["--all"])?;

        assert!(f.has_branch(SQUASHED)?);
        Ok(())
    }

    #[test]
    fn all_mode_removes_closed_prs_and_merged_orphans_but_keeps_unmerged_orphans_and_live_branches()
    -> Result<()> {
        let mut f = Fixture::merged()?;
        let sh = &f.sh;
        cmd!(sh, "git push origin --delete {SQUASHED}").run()?;
        cmd!(sh, "git switch main").run()?;
        for branch in ["local-merged", "gone-merged", "live-merged", "develop"] {
            cmd!(sh, "git branch {branch}").run()?;
        }
        cmd!(sh, "git push -u origin gone-merged live-merged").run()?;
        cmd!(sh, "git push origin --delete gone-merged").run()?;
        cmd!(sh, "git switch -c closed-pr").run()?;
        let closed = f.commit("abandoned.txt")?;
        cmd!(sh, "git switch -c gone-unmerged main").run()?;
        f.commit("unfinished.txt")?;
        cmd!(sh, "git push -u origin gone-unmerged").run()?;
        cmd!(sh, "git push origin --delete gone-unmerged").run()?;
        f.prs.push(PullRequest {
            number: 2,
            state: State::Closed,
            merged_at: None,
            head_ref_name: "closed-pr".to_owned(),
            head_ref_oid: closed,
            ..f.prs[0].clone()
        });

        f.cleanup(&["--all"])?;

        assert_eq!(
            f.branches()?,
            ["develop", "gone-unmerged", "live-merged", "main"]
        );
        Ok(())
    }

    #[test]
    fn branches_checked_out_in_other_worktrees_are_preserved() -> Result<()> {
        let f = Fixture::merged()?;
        cmd!(f.sh, "git switch main").run()?;
        cmd!(f.sh, "git worktree add ../other-worktree {SQUASHED}").run()?;

        let output = f.cleanup(&["--all"])?;

        assert!(f.has_branch(SQUASHED)?);
        assert!(output.contains("checked out in a worktree"), "{output}");
        Ok(())
    }

    #[test]
    fn same_named_fork_prs_never_authorize_local_deletion() -> Result<()> {
        let mut f = Fixture::merged()?;
        f.prs[0].is_cross_repository = true;

        let output = f.cleanup(&[])?;

        assert!(f.has_branch(SQUASHED)?);
        assert!(output.contains("another repository"), "{output}");
        Ok(())
    }

    #[test]
    fn a_failed_fast_forward_pull_leaves_all_feature_branches_intact() -> Result<()> {
        let f = Fixture::merged()?;
        let (sh, initial) = (&f.sh, &f.initial);
        cmd!(sh, "git switch main").run()?;
        cmd!(sh, "git reset --hard {initial}").run()?;
        f.commit("diverged-main.txt")?;
        cmd!(sh, "git switch {SQUASHED}").run()?;

        assert!(f.cleanup(&[]).is_err());

        assert!(f.has_branch(SQUASHED)?);
        Ok(())
    }

    #[test]
    fn a_dirty_checkout_stops_before_reading_pull_requests() -> Result<()> {
        let f = Fixture::merged()?;
        f.sh.write_file("uncommitted.txt", "work in progress")?;
        let options = Options {
            all:     false,
            dry_run: false,
        };

        let error = cleanup(&f.sh, &options, &mut Vec::new(), || unreachable!()).unwrap_err();

        assert!(error.to_string().contains("Commit or stash"), "{error}");
        assert_eq!(f.current()?, SQUASHED);
        Ok(())
    }

    #[test]
    fn github_failures_and_invalid_responses_stop_before_switching_or_deleting() -> Result<()> {
        let f = Fixture::merged()?;
        let options = Options {
            all:     true,
            dry_run: false,
        };

        let result = cleanup(&f.sh, &options, &mut Vec::new(), || {
            bail!("GitHub authentication failed")
        });

        assert!(result.is_err());
        assert_eq!(f.current()?, SQUASHED);
        assert!(f.has_branch(SQUASHED)?);
        assert!(serde_json::from_str::<Vec<PullRequest>>(r#"[{"number":1}]"#).is_err());
        Ok(())
    }
}
