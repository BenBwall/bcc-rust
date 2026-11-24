//! How to read a back-end stage's output: one line in that stage's notation
//! taken apart, each part glossed beneath it, and a key to the colours of the
//! other kinds of token in the view. The colours are the editor themes'
//! (`editor/src/themes.js`), which `style.css` repeats as `--syntax-*`.

use leptos::prelude::*;

use crate::pipeline::Stage;

/// What a stage's notation looks like: one line of its output in parts, and
/// the other kinds of token its view colours.
#[derive(Clone, Copy, Debug)]
pub struct Notation {
    /// A line of output, in the parts a reader takes it apart into.
    pub parts: &'static [Part],
    /// What the line does, in a sentence.
    pub reads: &'static str,
    /// The other kinds of token the view colours, each with a sample.
    pub key:   &'static [Part],
}

/// A piece of a line with what it means.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    /// The piece's text, as the view colours it.
    pub code:   &'static [(&'static str, Ink)],
    pub gloss:  &'static str,
    /// Whether it follows the part before it without a space, as `.i32`
    /// follows `iadd`.
    pub joined: bool,
}

/// The colour a kind of token has in every code view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// An operation: an IR instruction or a machine instruction.
    Instruction,
    Type,
    /// An SSA value.
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

const fn part(code: &'static [(&'static str, Ink)], gloss: &'static str) -> Part {
    Part {
        code,
        gloss,
        joined: false,
    }
}

/// A part written against the one before it.
const fn joined(code: &'static [(&'static str, Ink)], gloss: &'static str) -> Part {
    Part {
        code,
        gloss,
        joined: true,
    }
}

use Ink::{
    Bytes,
    Function,
    Instruction,
    Label,
    Muted,
    Number,
    Qualifier,
    Register,
    Slot,
    Type,
    Value,
};

/// bcc IR, before and after ABI lowering, which writes the same notation.
const BCC_IR: Notation = Notation {
    parts: &[
        part(
            &[("v3", Value), (" =", Qualifier)],
            "the result: a new value, assigned only here",
        ),
        part(&[("iadd", Instruction)], "the operation: integer add"),
        joined(&[(".i32", Type)], "its type: a 32-bit integer, as int is"),
        part(
            &[("nsw", Qualifier)],
            "no signed wrap: bcc may assume the sum fits, since signed overflow is undefined",
        ),
        part(
            &[("v1", Value), (",", Qualifier), (" v2", Value)],
            "the operands: values made earlier",
        ),
    ],
    reads: "Add v1 and v2 as 32-bit integers, and call the sum v3.",
    key:   &[
        part(
            &[("block0", Label)],
            "a basic block, entered only at its top",
        ),
        part(
            &[("slot0", Slot)],
            "a stack slot: memory in the function's frame",
        ),
        part(&[("@main", Function)], "a function"),
        part(&[("4", Number)], "a constant"),
        part(&[("; …", Muted)], "a comment"),
    ],
};

const LLVM_IR: Notation = Notation {
    parts: &[
        part(
            &[("%v1", Value), (" =", Qualifier)],
            "the result: % marks a local name, assigned only here",
        ),
        part(&[("load", Instruction)], "the operation: read memory"),
        part(
            &[("i32", Type), (",", Qualifier)],
            "the type it reads: a 32-bit integer",
        ),
        part(
            &[("ptr", Type), (" %v0", Value), (",", Qualifier)],
            "from the address the pointer %v0 holds",
        ),
        part(
            &[("align", Qualifier), (" 4", Number)],
            "a promise that the address is a multiple of 4",
        ),
    ],
    reads: "Read the 32-bit integer at the address in %v0, and call it %v1.",
    key:   &[
        part(
            &[("block0:", Label)],
            "a basic block; a branch names it %block0",
        ),
        part(
            &[("%slot0", Slot)],
            "the address of memory that alloca reserves in the frame",
        ),
        part(&[("@main", Function)], "a function: @ marks a global name"),
        part(&[("2", Number)], "a constant"),
        part(&[("#0", Muted)], "attributes, listed at the end"),
    ],
};

/// AT&T syntax, as GCC and Clang print x86-64 assembly by default.
const ASSEMBLY: Notation = Notation {
    parts: &[
        part(&[("mov", Instruction)], "the instruction: copy"),
        joined(&[("l", Instruction)], "the size: l for long, 32 bits"),
        part(
            &[("%eax", Register), (",", Qualifier)],
            "the source: register eax",
        ),
        part(
            &[("(", Qualifier), ("%rdi", Register), (")", Qualifier)],
            "the destination, last: the memory at the address in register rdi",
        ),
    ],
    reads: "Store the 32 bits in eax at the address rdi holds.",
    key:   &[
        part(
            &[("$1", Number)],
            "a constant, written into the instruction",
        ),
        part(&[(".LBB0_0:", Label)], "a label, where a jump can go"),
        part(&[("main:", Function)], "where a function starts"),
        part(
            &[(".cfi_offset", Muted)],
            "a directive to the assembler, not an instruction",
        ),
    ],
};

/// The listing: offset, bytes, then the instruction they encode.
const MACHINE_CODE: Notation = Notation {
    parts: &[
        part(
            &[("5", Muted)],
            "the offset from the function's first byte, in hexadecimal",
        ),
        part(
            &[("89", Bytes)],
            "the opcode: copy a 32-bit register into a register or memory",
        ),
        part(
            &[("07", Bytes)],
            "the ModRM byte: from eax into the memory at the address in rdi",
        ),
        part(
            &[
                ("movl", Instruction),
                (" %eax", Register),
                (", (", Qualifier),
                ("%rdi", Register),
                (")", Qualifier),
            ],
            "the assembly the two bytes encode",
        ),
    ],
    reads: "Two bytes, 89 07, are the whole of the assembly view's movl %eax, (%rdi).",
    key:   &[
        part(&[("<main>:", Function)], "a function, at offset 0"),
        part(
            &[(".LBB1_3:", Label)],
            "a label: the next instruction is where a jump lands",
        ),
        part(
            &[("e8 00 00 00 00", Bytes)],
            "a call; the linker fills in the zero bytes",
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

/// The selected stage's notation, under its output.
#[component]
pub fn StageNotation(stage: RwSignal<Stage>, #[prop(into)] hidden: Signal<bool>) -> impl IntoView {
    move || {
        let notation = notation(stage.get())?;
        Some(view! {
            <figure class="notation" hidden=move || hidden.get()>
                <figcaption class="notation-title">"How to read a line"</figcaption>
                <p class="notation-line">{whole(notation.parts)}</p>
                <p class="notation-reads">{notation.reads}</p>
                <dl class="notation-parts" aria-label="The line, part by part">
                    {notation.parts.iter().map(|part| view! {
                        <div class="notation-part">
                            <dt>{code(part)}</dt>
                            <dd>{part.gloss}</dd>
                        </div>
                    }).collect_view()}
                </dl>
                <dl class="notation-key" aria-label="Other colours in this view">
                    {notation.key.iter().map(|part| view! {
                        <div>
                            <dt>{code(part)}</dt>
                            <dd>{part.gloss}</dd>
                        </div>
                    }).collect_view()}
                </dl>
            </figure>
        })
    }
}

/// The parts as one line, as the view writes it.
fn whole(parts: &'static [Part]) -> impl IntoView + use<> {
    let pieces = parts
        .iter()
        .enumerate()
        .flat_map(|(index, part)| {
            let gap = (index > 0 && !part.joined).then_some((" ", Qualifier));
            gap.into_iter().chain(part.code.iter().copied())
        })
        .map(|(text, ink)| view! { <span class=ink.class()>{text}</span> })
        .collect_view();
    view! { <code>{pieces}</code> }
}

/// A part's text in its colours.
fn code(part: &Part) -> impl IntoView + use<> {
    let pieces = part
        .code
        .iter()
        .map(|&(text, ink)| view! { <span class=ink.class()>{text}</span> })
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

    /// Every lesson's views of the stages with a notation, at both levels,
    /// as the page shows them.
    fn outputs(demos: &[crate::demos::Demo]) -> Vec<(Stage, String)> {
        let mut outputs = Vec::new();
        for demo in demos {
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
                    outputs.push((stage, text));
                }
            }
        }
        outputs
    }

    fn spelled(part: &Part) -> String {
        part.code.iter().map(|(text, _)| *text).collect()
    }

    fn squeezed(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Each example, as its parts spell it, is a line of real output: the
    /// first lesson's at -O0 or -O2.
    #[test]
    fn every_example_is_a_line_of_the_first_lessons_output() {
        let outputs = outputs(&DEMOS[..1]);
        for stage in Stage::ALL {
            let Some(notation) = notation(stage) else {
                continue;
            };
            // The parts, spaced as the figure writes them.
            let mut line = String::new();
            for (index, part) in notation.parts.iter().enumerate() {
                if index > 0 && !part.joined {
                    line.push(' ');
                }
                line.push_str(&spelled(part));
            }
            let line = squeezed(&line);
            assert!(
                outputs.iter().any(|(each, output)| *each == stage
                    && output.lines().any(|each| squeezed(each) == line)),
                "{stage:?}: `{line}` is in none of the first lesson's outputs"
            );
        }
    }

    /// Each sample in a key is in some lesson's view of that stage: a label
    /// line in the machine code, a call's zero bytes, and so on.
    #[test]
    fn every_key_sample_is_in_some_lesson() {
        let outputs = outputs(&DEMOS);
        for stage in Stage::ALL {
            let Some(notation) = notation(stage) else {
                continue;
            };
            for part in notation.key {
                let sample = spelled(part);
                let sample = sample.trim_end_matches('…').trim();
                assert!(
                    outputs
                        .iter()
                        .any(|(each, output)| *each == stage && output.contains(sample)),
                    "{stage:?}: no lesson shows `{sample}`"
                );
            }
        }
    }
}
