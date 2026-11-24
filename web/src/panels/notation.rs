//! How to read a back-end stage's output: one line in that stage's notation
//! taken apart, each part glossed beneath it, and a key to the colours of the
//! other kinds of token in the view. The line comes from the program on
//! screen when it has one of a shape the stage's reader takes apart
//! (`read.rs`), and otherwise from the first lesson. The colours are the
//! editor themes' (`editor/src/themes.js`), which `style.css` repeats as
//! `--syntax-*`.

mod read;

use leptos::prelude::*;

use crate::pipeline::Stage;

/// What a view's notation looks like: how to find a line of it to take
/// apart, and the other kinds of token the view colours.
#[derive(Clone, Copy, Debug)]
pub struct Notation {
    /// Takes one line of the view apart, if it has a shape this view's
    /// figure explains.
    read:            fn(&str) -> Option<Reading>,
    /// Whether a line of the view starts a function.
    starts_function: fn(&str) -> bool,
    /// The example when the program on screen has no line `read` takes:
    /// a line of the first lesson's output.
    pub fallback:    &'static str,
    /// The other kinds of token the view colours, each with a sample.
    pub key:         &'static [Sample],
}

impl Notation {
    /// The example for a view showing `output`: the most instructive line
    /// `read` takes in the first function that has one, or else the
    /// fallback.
    pub fn example(&self, output: &str) -> Example {
        let mut best: Option<Reading> = None;
        for line in output.lines() {
            if best.is_some() && (self.starts_function)(line) {
                break;
            }
            if let Some(reading) = (self.read)(line)
                && best.as_ref().is_none_or(|best| reading.rank < best.rank)
            {
                best = Some(reading);
            }
        }
        best.or_else(|| (self.read)(self.fallback))
            .map(|reading| reading.example)
            .expect("the fallback is a line the reader takes")
    }
}

/// A line a reader took apart, and how instructive it is: the figure shows
/// the lowest rank.
#[derive(Clone, Debug)]
pub struct Reading {
    pub rank:    u8,
    pub example: Example,
}

/// One line in parts, and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Example {
    pub parts: Vec<Part>,
    /// What the line does, in a sentence.
    pub reads: String,
}

/// A piece of the example line with what it means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// The piece's text, as the view colours it.
    pub code:   Vec<(String, Ink)>,
    pub gloss:  String,
    /// Whether it follows the part before it without a space, as `.i32`
    /// follows `iadd`.
    pub joined: bool,
}

/// A kind of token in the key, with a sample of it.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub code:  &'static [(&'static str, Ink)],
    pub gloss: &'static str,
}

/// The colour a kind of token has in every code view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// An operation: an IR instruction or a machine instruction.
    Instruction,
    Type,
    /// An SSA value, or a WebAssembly local.
    Value,
    /// Memory in the function's frame.
    Slot,
    /// A basic block or a jump target.
    Label,
    /// A function's name where it is defined or called.
    Function,
    Number,
    Register,
    /// A flag or an attribute that qualifies an instruction, and punctuation.
    Qualifier,
    /// A comment, a directive or an offset.
    Muted,
    /// The bytes of machine code.
    Bytes,
}

impl Ink {
    pub const fn class(self) -> &'static str {
        match self {
            | Self::Instruction => "ink-instruction",
            | Self::Type => "ink-type",
            | Self::Value => "ink-value",
            | Self::Slot => "ink-slot",
            | Self::Label => "ink-label",
            | Self::Function => "ink-function",
            | Self::Number => "ink-number",
            | Self::Register => "ink-register",
            | Self::Qualifier => "ink-qualifier",
            | Self::Muted => "ink-muted",
            | Self::Bytes => "ink-bytes",
        }
    }
}

const fn sample(code: &'static [(&'static str, Ink)], gloss: &'static str) -> Sample {
    Sample { code, gloss }
}

use Ink::{
    Bytes,
    Function,
    Instruction,
    Label,
    Muted,
    Number,
    Slot,
    Value,
};

/// bcc IR, before and after ABI lowering, which writes the same notation.
const BCC_IR: Notation = Notation {
    read:            read::bcc_ir,
    starts_function: |line| line.starts_with("function @"),
    fallback:        "    v3 = iadd.i32 nsw v1, v2",
    key:             &[
        sample(
            &[("block0", Label)],
            "a basic block, entered only at its top",
        ),
        sample(
            &[("slot0", Slot)],
            "a stack slot: memory in the function's frame",
        ),
        sample(&[("@main", Function)], "a function"),
        sample(&[("4", Number)], "a constant"),
        sample(&[("; …", Muted)], "a comment"),
    ],
};

const LLVM_IR: Notation = Notation {
    read:            read::llvm_ir,
    starts_function: |line| line.starts_with("define "),
    fallback:        "  %v1 = load i32, ptr %v0, align 4",
    key:             &[
        sample(
            &[("block0:", Label)],
            "a basic block; a branch names it %block0",
        ),
        sample(
            &[("%slot0", Slot)],
            "the address of memory that alloca reserves in the frame",
        ),
        sample(&[("@main", Function)], "a function: @ marks a global name"),
        sample(&[("2", Number)], "a constant"),
        sample(&[("#0", Muted)], "attributes, listed at the end"),
    ],
};

/// AT&T syntax, as GCC and Clang print x86-64 assembly by default.
const ASSEMBLY: Notation = Notation {
    read:            read::assembly,
    starts_function: |line| {
        line.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_')
            && line.ends_with(':')
    },
    fallback:        "\tmovl\t%eax, (%rdi)",
    key:             &[
        sample(
            &[("$1", Number)],
            "a constant, written into the instruction",
        ),
        sample(&[(".LBB0_0:", Label)], "a label, where a jump can go"),
        sample(&[("main:", Function)], "where a function starts"),
        sample(
            &[(".globl", Muted)],
            "a directive to the assembler, not an instruction",
        ),
    ],
};

/// The listing: offset, bytes, then the instruction they encode.
const MACHINE_CODE: Notation = Notation {
    read:            read::machine_code,
    starts_function: |line| line.starts_with('<'),
    fallback:        "     5  89 07                    movl %eax, (%rdi)",
    key:             &[
        sample(&[("<main>:", Function)], "a function, at offset 0"),
        sample(
            &[(".LBB1_3:", Label)],
            "a label, written in for reading: it takes no bytes, and the next instruction is \
             where a jump lands",
        ),
        sample(
            &[("(12 bytes of padding)", Muted)],
            "filler before a label, summed up in one line rather than listed byte by byte",
        ),
        sample(
            &[("e8 00 00 00 00", Bytes)],
            "a call; the linker fills in the zero bytes",
        ),
    ],
};

/// The WebAssembly text format, as bcc writes it (and the stepper prints
/// Clang's modules).
pub const WASM_TEXT: Notation = Notation {
    read:            read::wasm_text,
    starts_function: |line| line.trim_start().starts_with("(func "),
    fallback:        "    i32.load align=4",
    key:             &[
        sample(
            &[("$v0", Value)],
            "a local: a variable of the function, kept outside linear memory",
        ),
        sample(
            &[("$__stack_pointer", Slot)],
            "the global that holds the top of the C stack, which lives in linear memory",
        ),
        sample(
            &[("$main", Function)],
            "a function, where it is defined or called",
        ),
        sample(&[("local.get", Instruction)], "push a local's value"),
        sample(&[("1", Number)], "a constant"),
        sample(
            &[(";; block0", Muted)],
            "a comment: the bcc IR block, and the C line, the code below comes from",
        ),
    ],
};

/// How to read `stage`'s output, for the stages after the syntax.
pub const fn notation(stage: Stage) -> Option<&'static Notation> {
    match stage {
        | Stage::Ir | Stage::IrAbi => Some(&BCC_IR),
        | Stage::Llvm => Some(&LLVM_IR),
        | Stage::Asm => Some(&ASSEMBLY),
        | Stage::MachineCode => Some(&MACHINE_CODE),
        | Stage::Lines
        | Stage::PpTokens
        | Stage::Preprocessor
        | Stage::Tokens
        | Stage::SyntaxTree
        | Stage::SemanticTypes => None,
    }
}

/// The selected stage's notation, under its output, which `text` holds.
#[component]
pub fn StageNotation(
    stage: RwSignal<Stage>,
    text: Signal<String>,
    #[prop(into)] hidden: Signal<bool>,
) -> impl IntoView {
    let open = opened_unless_narrow();
    move || {
        let notation = notation(stage.get())?;
        Some(view! { <NotationFigure notation text hidden open /> })
    }
}

/// Whether the figure is open: at first, unless the screen is a phone's,
/// where the figure would push the rest of the card a screen down. The
/// reader opens and closes it from then on, in every view.
pub fn opened_unless_narrow() -> RwSignal<bool> {
    let open = RwSignal::new(true);
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let narrow = web_sys::window()
            .and_then(|window| window.match_media(NARROW).ok().flatten())
            .is_some_and(|query| query.matches());
        if narrow {
            open.set(false);
        }
    });
    open
}

/// The screens on which the figure starts closed.
#[cfg(feature = "hydrate")]
const NARROW: &str = "(max-width: 600px)";

/// How to read a line of a view in `notation`, whose text `text` holds:
/// a disclosure the reader can close, titled as the figure it holds.
#[component]
pub fn NotationFigure(
    notation: &'static Notation,
    text: Signal<String>,
    #[prop(into)] hidden: Signal<bool>,
    open: RwSignal<bool>,
) -> impl IntoView {
    let example = Memo::new(move |_| text.with(|text| notation.example(text)));
    view! {
        <details
            class="notation"
            hidden=move || hidden.get()
            open=move || open.get()
            on:toggle=move |event| {
                let details: web_sys::Element = event_target(&event);
                open.set(details.has_attribute("open"));
            }
        >
            <summary class="notation-title">"How to read a line"</summary>
            <p class="notation-line">{move || example.with(|example| whole(&example.parts))}</p>
            <p class="notation-reads">{move || example.with(|example| example.reads.clone())}</p>
            <dl class="notation-parts" aria-label="The line, part by part">
                {move || example.with(|example| example.parts.iter().map(|part| view! {
                    <div class="notation-part">
                        <dt>{code(part.code.clone())}</dt>
                        <dd>{part.gloss.clone()}</dd>
                    </div>
                }).collect_view())}
            </dl>
            <dl class="notation-key" aria-label="Other colours in this view">
                {notation.key.iter().map(|sample| view! {
                    <div>
                        <dt>{code(sample.code.iter().map(|&(text, ink)| (text.to_owned(), ink)).collect())}</dt>
                        <dd>{sample.gloss}</dd>
                    </div>
                }).collect_view()}
            </dl>
        </details>
    }
}

/// The parts as one line, as the view writes it.
fn whole(parts: &[Part]) -> impl IntoView + use<> {
    let pieces = parts
        .iter()
        .enumerate()
        .flat_map(|(index, part)| {
            let gap = (index > 0 && !part.joined).then(|| (" ".to_owned(), Ink::Qualifier));
            gap.into_iter().chain(part.code.iter().cloned())
        })
        .collect();
    code(pieces)
}

/// Pieces of code in their colours.
fn code(pieces: Vec<(String, Ink)>) -> impl IntoView + use<> {
    let pieces = pieces
        .into_iter()
        .map(|(text, ink)| view! { <span class=ink.class()>{text}</span> })
        .collect_view();
    view! { <code>{pieces}</code> }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        demos::DEMOS,
        pipeline::{
            Compilation,
            Unfolded,
            displayed,
            precomputed_llvm,
        },
    };

    /// Each lesson's views of the stages with a notation, at both levels,
    /// as the page shows them, with the lesson's index.
    fn outputs() -> Vec<(usize, Stage, String)> {
        let mut outputs = Vec::new();
        for (index, demo) in DEMOS.iter().enumerate() {
            let stem = demo.name.trim_end_matches(".c");
            for optimize in [false, true] {
                let compilation = Compilation {
                    llvm: precomputed_llvm(stem, optimize).map(Into::into),
                    ..crate::explorer::compile(demo.source, optimize)
                };
                for stage in Stage::ALL
                    .into_iter()
                    .filter(|&stage| notation(stage).is_some())
                {
                    let (text, _) = displayed(stage, &compilation, Unfolded::default())
                        .expect("every lesson compiles");
                    outputs.push((index, stage, text));
                }
            }
        }
        outputs
    }

    /// Each lesson's WebAssembly text, as bcc writes it.
    fn wasm_texts() -> Vec<String> {
        use crate::wasm::WasmBackEnd;
        DEMOS
            .iter()
            .map(|demo| {
                crate::wasm::Bcc
                    .compile(demo.source)
                    .expect("bcc compiles every lesson")
                    .text
                    .expect("with its text")
                    .text
            })
            .collect()
    }

    /// The example's parts, spaced as the figure writes them.
    fn spelled(example: &Example) -> String {
        let mut line = String::new();
        for (index, part) in example.parts.iter().enumerate() {
            if index > 0 && !part.joined {
                line.push(' ');
            }
            for (text, _) in &part.code {
                line.push_str(text);
            }
        }
        squeezed(&line)
    }

    fn squeezed(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn has_line(output: &str, line: &str) -> bool {
        output.lines().any(|each| squeezed(each) == line)
    }

    /// Each fallback is a line of the first lesson's output, and its reader
    /// takes it apart into the line it was.
    #[test]
    fn every_fallback_is_a_line_of_the_first_lessons_output() {
        let outputs = outputs();
        let wasm = wasm_texts();
        let mut views: Vec<(&Notation, Vec<&str>)> = Stage::ALL
            .into_iter()
            .filter_map(|stage| {
                let first = outputs
                    .iter()
                    .filter(|(index, each, _)| *index == 0 && *each == stage)
                    .map(|(_, _, output)| output.as_str())
                    .collect();
                Some((notation(stage)?, first))
            })
            .collect();
        views.push((&WASM_TEXT, vec![wasm[0].as_str()]));
        for (notation, first) in views {
            let example = (notation.read)(notation.fallback)
                .expect("the reader takes its fallback")
                .example;
            let line = spelled(&example);
            assert_eq!(line, squeezed(notation.fallback));
            assert!(
                first.iter().any(|output| has_line(output, &line)),
                "`{line}` is in none of the first lesson's outputs"
            );
        }
    }

    /// Each view's example is a line of the output on screen whenever that
    /// output has one the reader takes, and most lessons have one of their
    /// own in every view.
    #[test]
    fn every_example_comes_from_the_program_on_screen_when_it_can() {
        let mut own = 0;
        let mut views = 0;
        let wasm = wasm_texts();
        let pipeline = outputs()
            .into_iter()
            .map(|(_, stage, output)| (notation(stage).expect("a notation"), output));
        let wasm = wasm.into_iter().map(|output| (&WASM_TEXT, output));
        for (notation, output) in pipeline.chain(wasm) {
            views += 1;
            let readable = output.lines().any(|line| (notation.read)(line).is_some());
            let line = spelled(&notation.example(&output));
            if readable {
                own += 1;
                assert!(has_line(&output, &line), "`{line}` is not on screen");
            } else {
                assert_eq!(line, squeezed(notation.fallback));
            }
        }
        assert!(
            own * 4 >= views * 3,
            "{own} of {views} views have an example of their own"
        );
    }

    /// The example comes from the first function with a line the reader
    /// takes, and is the most instructive line there.
    #[test]
    fn the_example_is_the_best_line_of_the_first_function_with_one() {
        let output = "bump:\n\tmovq\t%rsp, %rbp\n\taddl\t$1, %eax\nmain:\n\tmovl\t%eax, (%rdi)\n";
        assert_eq!(spelled(&ASSEMBLY.example(output)), "addl $1, %eax");
        assert_eq!(
            spelled(&ASSEMBLY.example("main:\n\tretq\n")),
            "movl %eax, (%rdi)",
            "a program without one gets the first lesson's"
        );
    }

    /// Each sample in a key is in some lesson's view of that stage: a label
    /// line in the machine code, a call's zero bytes, and so on.
    #[test]
    fn every_key_sample_is_in_some_lesson() {
        let outputs = outputs();
        let wasm = wasm_texts();
        let mut keys: Vec<(&Notation, Vec<&str>)> = Stage::ALL
            .into_iter()
            .filter_map(|stage| {
                let views = outputs
                    .iter()
                    .filter(|(_, each, _)| *each == stage)
                    .map(|(_, _, output)| output.as_str())
                    .collect();
                Some((notation(stage)?, views))
            })
            .collect();
        keys.push((&WASM_TEXT, wasm.iter().map(String::as_str).collect()));
        for (notation, views) in keys {
            for sample in notation.key {
                let text: String = sample.code.iter().map(|(text, _)| *text).collect();
                let text = text.trim_end_matches('…').trim();
                assert!(
                    views.iter().any(|output| output.contains(text)),
                    "no lesson shows `{text}`"
                );
            }
        }
    }
}
