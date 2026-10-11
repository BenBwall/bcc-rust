//! Runs textual IR, for tests here and in other workstreams.

use super::{
    DefaultHost,
    Limits,
    Outcome,
    RuntimeValue,
    Trap,
    run_with_limits,
};
use crate::{
    ir::{
        Profile,
        parse_module,
        verify_module,
    },
    util::bump::Bump,
};

/// What running a textual module gave: the outcome or trap, the program's
/// output, and the trap's message with function names.
#[derive(Debug)]
pub(crate) struct TextRun {
    pub(crate) result:  Result<Outcome, Trap>,
    pub(crate) output:  String,
    /// The trap described with function names, if the run trapped.
    pub(crate) message: Option<String>,
}

/// Parses and verifies `text`, then runs its function `entry` without
/// arguments under the default host and limits. Panics if the text does
/// not parse or verify.
pub(crate) fn run_text(text: &str, entry: &str) -> TextRun {
    run_text_with(text, entry, &[], Limits::default())
}

/// [`run_text`] with arguments and limits.
pub(crate) fn run_text_with(
    text: &str,
    entry: &str,
    args: &[RuntimeValue],
    limits: Limits,
) -> TextRun {
    let arena = Bump::new();
    let module = parse_module(&arena, text).unwrap_or_else(|error| panic!("{error}\n{text}"));
    let errors: Vec<String> = verify_module(&module, Profile::PreAbi, &arena)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{text}");
    let mut host = DefaultHost::new_in(&arena);
    let result = run_with_limits(&module, entry, args, &mut host, limits);
    TextRun {
        message: result
            .as_ref()
            .err()
            .map(|trap| trap.display_in(&module).to_string()),
        result,
        output: String::from_utf8_lossy(host.output()).into_owned(),
    }
}
