//! The links between the source and the view beside it (the compiler
//! pipeline's output, or the WebAssembly text), from the pointer and from
//! the keyboard.
//!
//! Hovering a line in either editor marks the lines linked to it in the
//! other. So does the cursor's line in the editor that has the keyboard
//! focus, for readers who move through the code with the arrow keys (or
//! tap it, on a touch screen). When both point somewhere, the one that
//! moved last wins: a pointer resting over an editor does not hide what
//! the arrow keys select, and the cursor's link shows again once the
//! pointer moves away. Each cursor move is also announced to screen
//! readers, politely and only once the cursor rests.

// `State` is the page's bundle of `Copy` signal handles, passed by value as
// everywhere in `app.rs`.
#![expect(clippy::large_types_passed_by_value, reason = "See above.")]

use gloo_timers::callback::Timeout;
use leptos::prelude::*;

use super::{
    Mode,
    State,
};
use crate::pipeline;

/// How long the cursor must rest on a line before its link is announced,
/// so holding an arrow key reads out only where it stops.
const ANNOUNCE_DELAY_MS: u32 = 700;

/// The lines the reader points at in the source and in the view beside it,
/// with the pointer and with the cursor. Every line is 1-based; 0 is none.
#[derive(Clone, Copy)]
pub struct Pointing {
    hover_source:     RwSignal<usize>,
    hover_output:     RwSignal<usize>,
    /// The cursor's line in the source while the source has the focus.
    cursor_source:    RwSignal<usize>,
    /// The cursor's line in the view beside it while that has the focus.
    cursor_output:    RwSignal<usize>,
    /// Whether the pointer moved after the cursor.
    pointer_latest:   RwSignal<bool>,
    /// The text of the links' live region.
    pub announcement: RwSignal<String>,
}

/// One side of a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Source,
    Output,
}

impl Pointing {
    pub fn new() -> Self {
        Self {
            hover_source:   RwSignal::new(0),
            hover_output:   RwSignal::new(0),
            cursor_source:  RwSignal::new(0),
            cursor_output:  RwSignal::new(0),
            pointer_latest: RwSignal::new(false),
            announcement:   RwSignal::new(String::new()),
        }
    }

    /// The pointer moved over `line` of `side` (0 when it left).
    pub fn hover(self, side: Side, line: usize) {
        if line > 0 {
            self.pointer_latest.set(true);
        }
        self.signals(side).0.set(line);
    }

    /// The cursor moved to `line` of `side` while it had the focus (0 when
    /// the focus left).
    pub fn cursor(self, side: Side, line: usize) {
        if line > 0 {
            self.pointer_latest.set(false);
        }
        self.signals(side).1.set(line);
    }

    /// The line of `side` whose linked lines on the other side to mark, if
    /// any (0).
    pub fn line(self, side: Side) -> usize {
        let hovered = (self.hover_source.get(), self.hover_output.get());
        let cursor = (self.cursor_source.get(), self.cursor_output.get());
        let pointer = hovered != (0, 0) && (cursor == (0, 0) || self.pointer_latest.get());
        let (source, output) = if pointer { hovered } else { cursor };
        match side {
            | Side::Source => source,
            | Side::Output => output,
        }
    }

    /// Forget the output's lines, as when its view unmounts: an editor
    /// disposed under the pointer or with the focus reports no leaving.
    pub fn clear_output(self) {
        self.hover_output.set(0);
        self.cursor_output.set(0);
    }

    fn signals(self, side: Side) -> (RwSignal<usize>, RwSignal<usize>) {
        match side {
            | Side::Source => (self.hover_source, self.cursor_source),
            | Side::Output => (self.hover_output, self.cursor_output),
        }
    }
}

/// Announce where each cursor move links, once the cursor rests. Only the
/// cursor's moves are announced (not the pointer's, nor recompilations
/// while typing on one line), and a link is not repeated while the focus
/// stays in the editors. A move made before the view has anything to link
/// (the compiler still loading) is announced when its output arrives.
pub(super) fn announce(state: State) {
    let pointing = state.pointing;
    let timer = StoredValue::new_local(None::<Timeout>);
    let waiting = StoredValue::new(None::<(Side, usize)>);
    let say = move |side: Side, line: usize| {
        let text = untrack(|| state.describe_link(side, line));
        waiting.set_value(text.is_none().then_some((side, line)));
        if let Some(text) = text
            && pointing.announcement.get_untracked() != text
        {
            pointing.announcement.set(text);
        }
    };
    Effect::new(move || {
        state.compilation.track();
        state.wasm.program.track();
        if let Some((side, line)) = waiting.get_value() {
            say(side, line);
        }
    });
    Effect::new(move || {
        let source = pointing.cursor_source.get();
        let output = pointing.cursor_output.get();
        // Dropping the pending timer cancels it.
        timer.set_value(None);
        waiting.set_value(None);
        let (side, line) = match (source, output) {
            | (0, 0) => {
                pointing.announcement.set(String::new());
                return;
            },
            | (_, 0) => (Side::Source, source),
            | _ => (Side::Output, output),
        };
        timer.set_value(Some(Timeout::new(ANNOUNCE_DELAY_MS, move || say(side, line))));
    });
}

impl State {
    /// What `line` of `side` links to in the view on screen, as a sentence,
    /// or `None` in a view without links.
    fn describe_link(self, side: Side, line: usize) -> Option<String> {
        match self.mode.get() {
            | Mode::Machine => None,
            | Mode::Pipeline => {
                let stage = self.stage.get();
                let headers = self.headers.get();
                self.compilation.with(|compilation| {
                    let compilation = compilation.as_ref()?;
                    let links = self.editor_text.with(|source| {
                        pipeline::links(stage, compilation, source, headers)
                    });
                    (!links.is_empty()).then(|| {
                        let linked = match side {
                            | Side::Source => links.outputs_for(line),
                            | Side::Output => links.sources_for(line),
                        };
                        // The machine code and the views that label their
                        // lines with source lines show no line numbers to
                        // refer to.
                        let numbered = stage != pipeline::Stage::MachineCode
                            && pipeline::gutter(stage, compilation, headers).is_empty();
                        let view = View {
                            name: stage.view_name(),
                            exact: links.is_exact(),
                            numbered,
                        };
                        describe(side, line, &linked, view)
                    })
                })
            },
            | Mode::Wasm => self.wasm.program.with(|program| {
                let program = program.as_ref().filter(|program| program.has_lines)?;
                let linked = match side {
                    | Side::Source => program.wat_lines_for(line),
                    | Side::Output => program.source_for(line).into_iter().collect(),
                };
                let view = View {
                    name:     "WebAssembly text",
                    exact:    true,
                    numbered: true,
                };
                Some(describe(side, line, &linked, view))
            }),
        }
    }
}

/// The output view a link sentence names.
#[derive(Clone, Copy)]
struct View {
    /// As in "line 3 of the bcc IR".
    name:     &'static str,
    /// Whether its lines link one by one rather than per function.
    exact:    bool,
    /// Whether its gutter numbers its lines, so a sentence can cite them.
    numbered: bool,
}

/// A sentence saying that `line` of `side` links to the `linked` lines of
/// the other side, the output being `view`.
fn describe(side: Side, line: usize, linked: &[usize], view: View) -> String {
    let View {
        name,
        exact,
        numbered,
    } = view;
    let lines = line_list(linked);
    let outputs = if numbered {
        lines.clone()
    } else if linked.len() == 1 {
        "1 line".to_owned()
    } else {
        format!("{} lines", linked.len())
    };
    let this = if numbered {
        format!("Line {line} of the {name}")
    } else {
        format!("This line of the {name}")
    };
    match (side, linked.is_empty(), exact) {
        | (Side::Source, true, _) => format!("Nothing in the {name} came from source line {line}."),
        | (Side::Source, false, true) => format!("Source line {line} became {outputs} of the {name}."),
        | (Side::Source, false, false) => {
            format!("Source line {line} is in a function, on {outputs} of the {name}.")
        },
        | (Side::Output, true, _) => format!("{this} has no source line."),
        | (Side::Output, false, true) => format!("{this} came from source {lines}."),
        | (Side::Output, false, false) => format!("{this} is in a function from source {lines}."),
    }
}

/// `lines` (ascending) in words, runs joined: `line 4`, `lines 3 to 5 and
/// 9`. Beyond three runs, only the count and the span.
fn line_list(lines: &[usize]) -> String {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &line in lines {
        match runs.last_mut() {
            | Some((_, last)) if *last + 1 == line => *last = line,
            | _ => runs.push((line, line)),
        }
    }
    match runs.as_slice() {
        | [] => String::new(),
        | [(first, last)] if first == last => format!("line {first}"),
        | [(first, last)] => format!("lines {first} to {last}"),
        | [.., (_, last)] if runs.len() > 3 => {
            format!("{} lines between {} and {last}", lines.len(), runs[0].0)
        },
        | [init @ .., end] => {
            let run = |&(first, last): &(usize, usize)| {
                if first == last {
                    first.to_string()
                } else {
                    format!("{first} to {last}")
                }
            };
            let init: Vec<String> = init.iter().map(run).collect();
            format!("lines {} and {}", init.join(", "), run(end))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_or_the_cursor_that_moved_last_wins() {
        let owner = Owner::new();
        owner.set();
        let pointing = Pointing::new();
        assert_eq!(pointing.line(Side::Source), 0);

        // The keyboard alone.
        pointing.cursor(Side::Source, 4);
        assert_eq!((pointing.line(Side::Source), pointing.line(Side::Output)), (4, 0));

        // The pointer moves over the output: it wins, and the source's
        // cursor no longer marks anything.
        pointing.hover(Side::Output, 9);
        assert_eq!((pointing.line(Side::Source), pointing.line(Side::Output)), (0, 9));

        // The arrow keys move the cursor under the resting pointer.
        pointing.cursor(Side::Source, 5);
        assert_eq!((pointing.line(Side::Source), pointing.line(Side::Output)), (5, 0));

        // The pointer moves again, then leaves: the cursor shows again.
        pointing.hover(Side::Output, 10);
        assert_eq!(pointing.line(Side::Output), 10);
        pointing.hover(Side::Output, 0);
        assert_eq!((pointing.line(Side::Source), pointing.line(Side::Output)), (5, 0));

        // The focus leaves the editors.
        pointing.cursor(Side::Source, 0);
        assert_eq!((pointing.line(Side::Source), pointing.line(Side::Output)), (0, 0));

        // The output's editor goes away under the pointer.
        pointing.hover(Side::Output, 3);
        pointing.clear_output();
        assert_eq!(pointing.line(Side::Output), 0);
    }

    #[test]
    fn links_read_as_sentences() {
        let ir = View {
            name:     "bcc IR",
            exact:    true,
            numbered: true,
        };
        let asm = View {
            name:     "x86-64 assembly",
            exact:    false,
            numbered: true,
        };
        let machine = View {
            name:     "machine code",
            exact:    false,
            numbered: false,
        };
        assert_eq!(
            describe(Side::Source, 5, &[12, 13, 14], ir),
            "Source line 5 became lines 12 to 14 of the bcc IR."
        );
        assert_eq!(
            describe(Side::Source, 2, &[], ir),
            "Nothing in the bcc IR came from source line 2."
        );
        assert_eq!(
            describe(Side::Output, 14, &[5], ir),
            "Line 14 of the bcc IR came from source line 5."
        );
        assert_eq!(
            describe(Side::Output, 1, &[], asm),
            "Line 1 of the x86-64 assembly has no source line."
        );
        assert_eq!(
            describe(Side::Source, 7, &[3, 4, 5, 6], asm),
            "Source line 7 is in a function, on lines 3 to 6 of the x86-64 assembly."
        );
        // The machine code shows offsets, not line numbers.
        assert_eq!(
            describe(Side::Output, 8, &[3, 4, 9], machine),
            "This line of the machine code is in a function from source lines 3 to 4 and 9."
        );
        assert_eq!(
            describe(Side::Source, 3, &[6, 7, 8], machine),
            "Source line 3 is in a function, on 3 lines of the machine code."
        );
    }

    #[test]
    fn line_lists_join_runs() {
        assert_eq!(line_list(&[4]), "line 4");
        assert_eq!(line_list(&[3, 4, 5, 9]), "lines 3 to 5 and 9");
        assert_eq!(line_list(&[1, 3, 5]), "lines 1, 3 and 5");
        assert_eq!(line_list(&[1, 3, 5, 7, 8]), "5 lines between 1 and 8");
    }
}
