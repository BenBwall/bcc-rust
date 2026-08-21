use std::{
    env,
    fs,
    path::Path,
    process::ExitCode,
};
#[cfg(windows)]
use std::{
    path::PathBuf,
    process::Command,
};

const FAILURE: u8 = 1;
const MAX_SUBJECT_LENGTH: usize = 72;
const TYPES: &[&str] = &[
    "feat", "fix", "perf", "refactor", "docs", "test", "build", "ci", "chore", "revert",
];

fn main() -> ExitCode {
    let Some(message_path) = env::args_os().nth(1) else {
        return fail("missing commit message path");
    };

    #[cfg(windows)]
    match refresh_and_run_if_needed(&message_path) {
        | Ok(Some(exit_code)) => return exit_code,
        | Ok(None) => {},
        | Err(error) => return fail(&error),
    }

    let message = match fs::read_to_string(Path::new(&message_path)) {
        | Ok(message) => message,
        | Err(error) => return fail(&format!("could not read commit message: {error}")),
    };

    match validate(&message) {
        | Ok(()) => ExitCode::SUCCESS,
        | Err(error) => fail(error),
    }
}

#[cfg(windows)]
fn refresh_and_run_if_needed(message_path: &std::ffi::OsStr) -> Result<Option<ExitCode>, String> {
    let root = repository_root()?;
    let source_hash = git_output(&root, ["hash-object", ".githooks/commit-msg.rs"])?;
    let expected_suffix = format!("bcc-rust-hooks/windows-{source_hash}");
    let current_hooks_path = git_output(&root, ["config", "--local", "--get", "core.hooksPath"])?;

    if current_hooks_path
        .replace('\\', "/")
        .ends_with(&expected_suffix)
    {
        return Ok(None);
    }

    let hooks_path = git_common_dir(&root)?.join(&expected_suffix);
    let hook = hooks_path.join("commit-msg.exe");
    if !hook.exists() {
        fs::create_dir_all(&hooks_path)
            .map_err(|error| format!("could not create {}: {error}", hooks_path.display()))?;
        let status = Command::new("rustc.exe")
            .current_dir(&root)
            .args(["--edition=2024", ".githooks/commit-msg.rs", "-o"])
            .arg(&hook)
            .status()
            .map_err(|error| format!("could not compile the Windows hook: {error}"))?;
        if !status.success() {
            return Err(format!("Windows hook compilation exited with {status}"));
        }
    }

    configure_hooks_path(&root, &hooks_path)?;
    let status = Command::new(&hook)
        .current_dir(root)
        .arg(message_path)
        .status()
        .map_err(|error| format!("could not run the refreshed Windows hook: {error}"))?;

    Ok(Some(ExitCode::from(status.code().unwrap_or(1) as u8)))
}

#[cfg(windows)]
fn repository_root() -> Result<PathBuf, String> {
    git_output(Path::new("."), ["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

#[cfg(windows)]
fn git_common_dir(root: &Path) -> Result<PathBuf, String> {
    git_output(
        root,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(PathBuf::from)
}

#[cfg(windows)]
fn configure_hooks_path(root: &Path, hooks_path: &Path) -> Result<(), String> {
    let status = Command::new("git")
        .current_dir(root)
        .args(["config", "--local", "core.hooksPath"])
        .arg(hooks_path)
        .status()
        .map_err(|error| format!("could not configure Git hooks: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("Git hook configuration exited with {status}"))
    }
}

#[cfg(windows)]
fn git_output<const N: usize>(root: &Path, args: [&str; N]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| format!("could not run Git: {error}"))?;
    if !output.status.success() {
        return Err("Git command failed while checking the Windows hook".to_owned());
    }

    String::from_utf8(output.stdout)
        .map(|output| output.trim().to_owned())
        .map_err(|error| format!("Git returned non-UTF-8 output: {error}"))
}

fn validate(message: &str) -> Result<(), &'static str> {
    let subject = message
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r');

    if subject.starts_with("Merge ") {
        return Ok(());
    }

    if subject.chars().count() > MAX_SUBJECT_LENGTH {
        return Err("subject exceeds the 72-character maximum");
    }

    let Some((prefix, summary)) = subject.split_once(": ") else {
        return Err("subject does not match the required format");
    };

    if !valid_prefix(prefix) || !summary.chars().next().is_some_and(char::is_uppercase) {
        return Err("subject does not match the required format");
    }

    if subject.ends_with('.') {
        return Err("subject must not end with a period");
    }

    if message
        .lines()
        .any(|line| line.trim_end_matches('\r').starts_with("BREAKING CHANGE:"))
        && !prefix.ends_with('!')
    {
        return Err("a BREAKING CHANGE trailer requires ! before the colon");
    }

    Ok(())
}

fn valid_prefix(prefix: &str) -> bool {
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let Some(scope_start) = prefix.find('(') else {
        return TYPES.contains(&prefix);
    };

    let Some(scope) = prefix
        .strip_suffix(')')
        .and_then(|prefix| prefix.get(scope_start + 1..))
    else {
        return false;
    };
    let commit_type = &prefix[..scope_start];

    TYPES.contains(&commit_type)
        && !scope.is_empty()
        && scope.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'-')
        })
}

fn fail(message: &str) -> ExitCode {
    eprintln!("Invalid commit message: {message}\n");
    eprintln!("Expected: type(scope): Concise imperative summary");
    eprintln!("Scope is optional. See CONTRIBUTING.md for the full policy.");
    ExitCode::from(FAILURE)
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn accepts_valid_messages() {
        for message in [
            "feat(preprocess): Expand variadic macros",
            "docs: Explain compiler pipeline",
            "feat(parse)!: Replace syntax tree\n\nBREAKING CHANGE: Update parser consumers.",
            "Merge branch 'feature'",
        ] {
            assert_eq!(validate(message), Ok(()), "{message}");
        }
    }

    #[test]
    fn rejects_invalid_messages() {
        for message in [
            "Expand variadic macros",
            "feat(preprocess): expand variadic macros",
            "feat(Preprocess): Expand variadic macros",
            "feat(-preprocess): Expand variadic macros",
            "feature(preprocess): Expand variadic macros",
            "fix(parse): Preserve source ranges.",
            "feat(parse): Replace syntax tree\n\nBREAKING CHANGE: Update parser consumers.",
        ] {
            assert!(validate(message).is_err(), "{message}");
        }
    }

    #[test]
    fn rejects_long_subjects() {
        let message = format!("feat(preprocess): {}", "A".repeat(55));
        assert!(validate(&message).is_err());
    }
}
