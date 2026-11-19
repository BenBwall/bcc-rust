//! How Clang's front end reads the warning options it is given: which it
//! knows, and what it says about one it does not, with the group its typo
//! correction suggests. Its driver passes every `-W` option it does not
//! own itself (`-Wl,`, `-Wa,`, `-Wp,` and the like) to the front end, which
//! reads them once for each translation unit.
//!
//! Clang 23: `clang/lib/Basic/Warnings.cpp` (`ProcessWarningOptions` and
//! `EmitUnknownDiagWarning`), `DiagnosticIDs::getNearestOption`, and
//! `warn_unknown_diag_option` and `warn_unknown_warning_specifier` in
//! `DiagnosticFrontendKinds.td`.

use crate::{
    diagnostics::format_in,
    util::bump::Bump,
};

/// What Clang's front end says about the warning option `option` (`-W…`),
/// without the `[-Wunknown-warning-option]` it ends with, or `None` when it
/// takes the option. It names a `-Wno-error=` group `-Werror=`, as Clang
/// does, and suggests the nearest group it knows for an unknown one.
pub(super) fn clang_complaint<'a>(option: &str, arena: &'a Bump) -> Option<&'a str> {
    let complaint = read(option)?;
    Some(match complaint {
        | Complaint::Specifier(kind) =>
            format_in!(arena, "unknown {kind} warning specifier: '{option}'"),
        | Complaint::Unknown(prefix, name) => match nearest_group(name) {
            | Some(suggestion) => format_in!(
                arena,
                "unknown warning option '{prefix}{name}'; did you mean '{prefix}{suggestion}'?"
            ),
            | None => format_in!(arena, "unknown warning option '{prefix}{name}'"),
        },
    })
}

/// Whether Clang's front end takes the warning option `option` (`-W…`)
/// without a word, so that it has whatever effect it has.
pub(super) fn clang_takes(option: &str) -> bool {
    read(option).is_none()
}

/// Why Clang's front end does not take a warning option.
enum Complaint<'o> {
    /// A `-Werror` or `-Wfatal-errors` followed by something other than
    /// `=` and a group: the option it misspells.
    Specifier(&'static str),
    /// A group it does not know, after the prefix it names it with.
    Unknown(&'static str, &'o str),
}

/// Reads `option` as `ProcessWarningOptions` does, returning what it says
/// about it, if anything.
fn read(option: &str) -> Option<Complaint<'_>> {
    let original = option.strip_prefix("-W")?;
    // `-Wformat=0` is `-Wno-format`.
    let name = if original == "format=0" {
        "no-format"
    } else {
        original
    };
    let (positive, name) = match name.strip_prefix("no-") {
        | Some(name) => (false, name),
        | None => (true, name),
    };
    // Neither is a group in its table.
    if matches!(name, "system-headers" | "everything") {
        return None;
    }
    if let Some(rest) = name.strip_prefix("error") {
        // GCC's deprecated `-Werror-implicit-function-declaration` stands.
        let group = match rest {
            | "" => return None,
            | "-implicit-function-declaration" => &rest[1..],
            | _ => match rest.strip_prefix('=') {
                | Some(group) => group,
                | None => return Some(Complaint::Specifier("-Werror")),
            },
        };
        return (!group.is_empty() && !knows_group(group))
            .then_some(Complaint::Unknown("-Werror=", group));
    }
    if let Some(rest) = name.strip_prefix("fatal-errors") {
        if rest.is_empty() {
            return None;
        }
        // `-Wfatal-errors-GROUP` is `-Wfatal-errors=GROUP`.
        if !(rest.starts_with('=') || rest.starts_with('-')) || rest.len() == 1 {
            return Some(Complaint::Specifier("-Wfatal-errors"));
        }
        let group = &rest[1..];
        return (!knows_group(group)).then_some(Complaint::Unknown("-Wfatal-errors=", group));
    }
    (!knows_group(name)).then_some(Complaint::Unknown(
        if positive { "-W" } else { "-Wno-" },
        name,
    ))
}

/// Clang's warning groups, in its table's order, one to a line after the
/// header: `~` before a group it knows but never suggests, one with no
/// warnings in it.
const GROUPS: &str = include_str!("personality/options/clang-warning-groups.list");

/// Each warning group Clang's front end knows, with whether it may suggest
/// it for a misspelled one.
fn groups() -> impl Iterator<Item = (&'static str, bool)> {
    GROUPS
        .lines()
        .skip(1)
        .map(|line| match line.strip_prefix('~') {
            | Some(name) => (name, false),
            | None => (line, true),
        })
}

/// Whether Clang's front end knows the warning group `name`. Its table
/// begins with a group whose name is empty, so an empty `-Wno-` is taken.
fn knows_group(name: &str) -> bool {
    name.is_empty() || groups().any(|(group, _)| group == name)
}

/// The group Clang suggests for the unknown group `name`: the one at the
/// least edit distance, unless another is as near, or none is nearer than
/// the unnamed group its table begins with, which is as far as `name` is
/// long (`DiagnosticIDs::getNearestOption`).
fn nearest_group(name: &str) -> Option<&'static str> {
    let mut best = None;
    let mut best_distance = name.len();
    for (group, suggested) in groups() {
        if !suggested {
            continue;
        }
        let Some(distance) = edit_distance(group, name, best_distance) else {
            continue;
        };
        if distance == best_distance {
            best = None;
        } else {
            best = Some(group);
            best_distance = distance;
        }
    }
    best
}

/// The Levenshtein distance between `group` and `name`, counted in bytes as
/// LLVM's `StringRef::edit_distance` counts it, or `None` when it exceeds
/// `limit`. Clang's group names are short, so the row over `group` fits on
/// the stack; a longer one is never the nearest.
fn edit_distance(group: &str, name: &str, limit: usize) -> Option<usize> {
    const LONGEST: usize = 96;
    let (group, name) = (group.as_bytes(), name.as_bytes());
    if group.len() >= LONGEST || group.len().abs_diff(name.len()) > limit {
        return None;
    }
    let mut row = [0; LONGEST];
    for (index, cell) in row.iter_mut().enumerate().take(group.len() + 1) {
        *cell = index;
    }
    for (y, &byte) in name.iter().enumerate() {
        let mut previous = row[0];
        row[0] = y + 1;
        let mut best_in_row = row[0];
        for x in 1..=group.len() {
            let above = row[x];
            row[x] = (previous + usize::from(group[x - 1] != byte)).min(row[x - 1].min(above) + 1);
            previous = above;
            best_in_row = best_in_row.min(row[x]);
        }
        if best_in_row > limit {
            return None;
        }
    }
    Some(row[group.len()]).filter(|&distance| distance <= limit)
}
