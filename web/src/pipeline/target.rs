//! The lines an IR module opens with that name the machine it is for: the
//! target triple and the data layout (and in LLVM IR the module's names
//! before them). A newcomer cannot read them, and they put the first
//! function below them, so the view folds them into one line that says what
//! they are, as the preprocessed text folds each header; the full lines stay
//! a switch away ("Show target").

use super::Stage;

/// The opening lines of one IR module's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening<'a> {
    /// How many lines lead the text.
    pub lines:       usize,
    /// The target triple, as `x86_64-unknown-linux-gnu`.
    pub triple:      &'a str,
    /// The data layout string, if the lines hold one.
    pub data_layout: Option<&'a str>,
}

/// The opening lines of `stage`'s `output`, if the stage is an IR and its
/// text names a target. bcc IR opens with `target triple = "…"` and `target
/// datalayout = "…"`; LLVM IR with its `; ModuleID`, `source_filename` and
/// then the same two lines, in the other order.
pub fn opening(stage: Stage, output: &str) -> Option<Opening<'_>> {
    let prefixes: &[&str] = match stage {
        | Stage::Ir | Stage::IrAbi => &[TRIPLE, DATA_LAYOUT],
        | Stage::Llvm => &["; ModuleID = ", "source_filename = ", DATA_LAYOUT, TRIPLE],
        | _ => return None,
    };
    let lines: Vec<&str> = output
        .lines()
        .take_while(|line| prefixes.iter().any(|prefix| line.starts_with(prefix)))
        .collect();
    let value = |prefix: &str| {
        lines
            .iter()
            .find_map(|line| line.strip_prefix(prefix))
            .map(|value| value.trim().trim_matches('"'))
            .filter(|value| !value.is_empty())
    };
    Some(Opening {
        lines:       lines.len(),
        triple:      value(TRIPLE)?,
        data_layout: value(DATA_LAYOUT),
    })
}

const TRIPLE: &str = "target triple = ";
const DATA_LAYOUT: &str = "target datalayout = ";

/// `output` with its opening lines folded into one (see [`summary`]), and how
/// many lines that took out; `None` if it has none to fold.
pub fn fold(stage: Stage, output: &str) -> Option<(String, usize)> {
    let opening = opening(stage, output)?;
    let rest: String = output.split_inclusive('\n').skip(opening.lines).collect();
    Some((format!("{}\n{rest}", summary(&opening)), opening.lines - 1))
}

/// The line that stands for the folded lines: a comment in both IRs.
fn summary(opening: &Opening<'_>) -> String {
    let what = if opening.data_layout.is_some() {
        "the target and its data layout"
    } else {
        "the target"
    };
    format!("; {}: {what}, {FOLDED}", opening.triple)
}

const FOLDED: &str = "folded (Show target to read them)";

/// Whether `line` of a view is the folded opening lines' summary.
pub fn is_summary(line: &str) -> bool {
    line.starts_with("; ") && line.ends_with(FOLDED)
}

/// What the folded lines say, in Markdown, for the summary line's hover.
pub fn explanation(opening: &Opening<'_>) -> String {
    let mut paragraphs = vec![format!(
        "**Target triple** `{}` names the machine the code is for{}.",
        opening.triple,
        triple_parts(opening.triple)
    )];
    if let Some(layout) = opening.data_layout {
        let entries = layout_entries(layout);
        let here = if entries.is_empty() {
            String::new()
        } else {
            format!(". Here {}", entries.join(", "))
        };
        paragraphs.push(format!(
            "**Data layout** `{layout}` records how this target stores data: its byte order, the \
             sizes and alignments of types, and the stack's alignment, as entries separated by \
             dashes{here}."
        ));
    }
    paragraphs.push("The glossary explains them. Show target puts the lines back.".to_owned());
    paragraphs.join("\n\n")
}

/// `: its processor (x86_64), …` for a triple of three or four parts.
fn triple_parts(triple: &str) -> String {
    let parts: Vec<&str> = triple.split('-').collect();
    let named = |names: &[&str]| -> String {
        names
            .iter()
            .zip(&parts)
            .map(|(name, part)| format!("{name} ({part})"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match parts.len() {
        | 3 => format!(
            ": its {}",
            named(&["processor", "vendor", "operating system"])
        ),
        | 4 => format!(
            ": its {}",
            named(&["processor", "vendor", "operating system", "environment"])
        ),
        | _ => String::new(),
    }
}

/// What the commonest entries of a data layout string say, in its own order.
fn layout_entries(layout: &str) -> Vec<String> {
    layout
        .split('-')
        .filter_map(|entry| {
            let said = match entry {
                | "e" =>
                    "little-endian: a value's least significant byte is stored first".to_owned(),
                | "E" => "big-endian: a value's most significant byte is stored first".to_owned(),
                | _ =>
                    if let Some(bits) = entry.strip_prefix('S') {
                        format!("the stack is aligned to {bits} bits")
                    } else if let Some((size, align)) = entry
                        .strip_prefix('i')
                        .and_then(|rest| rest.split_once(':'))
                        .filter(|(size, _)| *size == "64")
                    {
                        let align = align.split(':').next().unwrap_or(align);
                        format!("a {size}-bit integer is aligned to {align} bits")
                    } else {
                        return None;
                    },
            };
            Some(format!("`{entry}`: {said}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BCC_IR: &str = "target triple = \"x86_64-unknown-linux-gnu\"\ntarget datalayout = \
                          \"e-m:e-i64:64-n8:16:32:64-S128\"\n\nfunction @main() -> i32 external \
                          {\n}\n";

    #[test]
    fn folds_the_target_lines_into_one_that_says_what_they_are() {
        let (folded, removed) = fold(Stage::Ir, BCC_IR).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            folded,
            "; x86_64-unknown-linux-gnu: the target and its data layout, folded (Show target to \
             read them)\n\nfunction @main() -> i32 external {\n}\n"
        );
        assert!(is_summary(folded.lines().next().unwrap()));
        assert!(fold(Stage::Asm, BCC_IR).is_none(), "only the IRs fold");
        assert!(
            fold(Stage::Ir, "function @main() -> i32 external {\n}\n").is_none(),
            "a module without a target has nothing to fold"
        );
    }

    #[test]
    fn llvm_ir_folds_its_module_names_with_the_target() {
        let llvm = "; ModuleID = 'bcc'\nsource_filename = \"bcc\"\ntarget datalayout = \
                    \"e\"\ntarget triple = \"x86_64-unknown-linux-gnu\"\n\ndefine i32 @main() {\n";
        let opening = opening(Stage::Llvm, llvm).unwrap();
        assert_eq!(
            opening,
            Opening {
                lines:       4,
                triple:      "x86_64-unknown-linux-gnu",
                data_layout: Some("e"),
            }
        );
        let (folded, removed) = fold(Stage::Llvm, llvm).unwrap();
        assert_eq!(removed, 3);
        assert_eq!(folded.lines().nth(2), Some("define i32 @main() {"));
    }

    #[test]
    fn explains_the_triple_and_the_layout_entries_it_knows() {
        let text = explanation(&opening(Stage::Ir, BCC_IR).unwrap());
        assert!(
            text.contains(
                "its processor (x86_64), vendor (unknown), operating system (linux), environment \
                 (gnu)"
            ),
            "{text}"
        );
        assert!(text.contains("`e`: little-endian"), "{text}");
        assert!(
            text.contains("`i64:64`: a 64-bit integer is aligned to 64 bits"),
            "{text}"
        );
        assert!(
            text.contains("`S128`: the stack is aligned to 128 bits"),
            "{text}"
        );
        assert!(
            !text.contains("`n8"),
            "an entry it cannot explain is left out: {text}"
        );
    }
}
