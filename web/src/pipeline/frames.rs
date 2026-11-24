//! The assembly's call-frame directives: `.cfi_startproc`, `.cfi_offset
//! %rbp, -16` and the rest, about a third of a function's lines at `-O0`.
//! They are not instructions: the assembler turns them into the `.eh_frame`
//! table (GNU as, "CFI directives"; bcc's own assembler likewise, in
//! `src/backend/native/object/unwind.rs`), which says at each instruction
//! where the caller's frame and the saved registers are, for a debugger's
//! backtrace and for unwinding the stack. A newcomer cannot read them and
//! they bury the instructions, so the view folds them into one line that
//! says what they are and how many there are, as it folds the IRs' target
//! lines (`target.rs`); the full listing stays a switch away ("Show call
//! frames").

/// `output` with its call-frame directives folded into one line at the top
/// (see [`summary`]), and for each line of the result the 0-based line of
/// `output` it shows (`None` for the summary); `None` if it has none to fold.
pub fn fold(output: &str) -> Option<(String, Vec<Option<usize>>)> {
    let count = count(output);
    if count == 0 {
        return None;
    }
    let mut text = summary(count);
    text.push('\n');
    let mut shown = vec![None];
    for (index, line) in output.split_inclusive('\n').enumerate() {
        if !is_directive(line) {
            text.push_str(line);
            shown.push(Some(index));
        }
    }
    Some((text, shown))
}

/// How many call-frame directives `output` has.
pub fn count(output: &str) -> usize {
    output.lines().filter(|line| is_directive(line)).count()
}

fn is_directive(line: &str) -> bool {
    line.trim_start().starts_with(".cfi_")
}

/// The line that stands for the folded directives: a comment in the
/// assembly.
fn summary(count: usize) -> String {
    let directives = if count == 1 {
        "1 .cfi_ directive".to_owned()
    } else {
        format!("{count} .cfi_ directives")
    };
    format!("# {directives}: call-frame information for debuggers and unwinding, {FOLDED}")
}

const FOLDED: &str = "folded (Show call frames to read them)";

/// Whether `line` of the view is the folded directives' summary.
pub fn is_summary(line: &str) -> bool {
    line.starts_with("# ") && line.ends_with(FOLDED)
}

/// What the folded lines are, in Markdown, for the summary line's hover.
pub fn explanation(count: usize) -> String {
    format!(
        "**Call-frame information.** The {count} `.cfi_` lines folded here are directives, not \
         instructions: they encode to no bytes of code. The assembler turns them into a table, \
         `.eh_frame`, that says at each instruction where the function's caller's frame and its \
         saved registers are. A debugger reads it to list the calls that led to a line (a \
         backtrace), and unwinding reads it to leave functions without returning through \
         them.\n\nShow call frames puts them back in place."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSEMBLY: &str = concat!(
        "\t.text\n",
        "bump:\n",
        "\t.cfi_startproc\n",
        "\tpushq\t%rbp\n",
        "\t.cfi_def_cfa_offset 16\n",
        "\tpopq\t%rbp\n",
        "\tretq\n",
        "\t.cfi_endproc\n",
    );

    #[test]
    fn folds_the_directives_into_one_line_that_says_what_they_are() {
        let (folded, shown) = fold(ASSEMBLY).unwrap();
        let summary = "# 3 .cfi_ directives: call-frame information for debuggers and unwinding, \
                       folded (Show call frames to read them)";
        assert_eq!(
            folded,
            format!("{summary}\n\t.text\nbump:\n\tpushq\t%rbp\n\tpopq\t%rbp\n\tretq\n")
        );
        assert_eq!(shown, [None, Some(0), Some(1), Some(3), Some(5), Some(6)]);
        assert!(is_summary(folded.lines().next().unwrap()));
        assert!(fold("main:\n\tretq\n").is_none(), "nothing to fold");
    }
}
