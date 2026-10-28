//! The explorer's options and outputs. They hold only owned standard types,
//! so they cross a Web Worker boundary or serialize (with the `serde`
//! feature) without borrowing the compiler's arenas.

#![expect(
    clippy::disallowed_types,
    reason = "The explorer's interface hands owned strings and vectors to its caller, outside \
              every arena."
)]

/// What to compile and how.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ExplorerOptions {
    /// The `--target` triple, `x86_64-unknown-linux-gnu` by default.
    pub target:            String,
    /// The `--std` language mode, `c17` by default.
    pub std:               String,
    /// Run bcc's optimizer and define `__OPTIMIZE__`, as `-O2` does; without
    /// it, the module is lowered as at `-O0`.
    pub optimize:          bool,
    /// The `--max-nesting-depth` limit. `None` picks the CLI's default on a
    /// native host and
    /// [`WASM_MAX_NESTING_DEPTH`](super::WASM_MAX_NESTING_DEPTH)
    /// in WebAssembly, whose stack is far smaller.
    pub max_nesting_depth: Option<u32>,
    /// The `--source-date-epoch` that `__DATE__` and `__TIME__` spell, in
    /// seconds since the Unix epoch. `None` spells the epoch itself, so the
    /// output never depends on the clock.
    pub source_date_epoch: Option<i64>,
    /// Headers the source can include, by the name it includes them by
    /// (`#include "list.h"` finds `list.h`), searched before the embedded
    /// resource headers. The host file system is never read.
    pub files:             Vec<ExplorerFile>,
    /// Ends every syntax-tree line with the `@line:column` of its node, as
    /// `--syntax-locations` does, so a viewer can link the tree to the
    /// source line by line.
    pub syntax_locations:  bool,
}

/// An in-memory header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerFile {
    /// The name an `#include` spells, such as `list.h`.
    pub name:     String,
    /// The header's text.
    pub contents: String,
}

/// Every stage's output. A stage that failed, or that an earlier failure
/// kept from running, is `None`, and the diagnostics say why.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerOutput {
    /// The compilation's diagnostics in the order the CLI prints them, then
    /// any a back-end stage reported.
    pub diagnostics:        Vec<ExplorerDiagnostic>,
    /// The source after translation phases 1 and 2: its logical lines,
    /// with what mapping characters and splicing lines changed.
    pub logical_lines:      Option<Vec<ExplorerLogicalLine>>,
    /// The source's preprocessing tokens and white space, as phase 3
    /// decomposes it.
    pub pp_tokens:          Option<Vec<ExplorerPpToken>>,
    /// The translation unit as phase 4 leaves it, as text with line
    /// markers, written as `-E` writes it but with each preprocessing token
    /// spelled as written and adjacent string literals still apart: `-E`
    /// shows them after phases 5 and 6.
    pub preprocessed:       Option<String>,
    /// Where each line of `preprocessed` came from.
    pub preprocessed_lines: Option<Vec<ExplorerOutputLine>>,
    /// What phase 4 did to produce `preprocessed`.
    pub preprocessing:      Option<ExplorerPreprocessing>,
    /// The preprocessed tokens, one per line, as `--tokens` prints them.
    pub tokens:             Option<String>,
    /// The syntax tree, as `--syntax-tree` prints it.
    pub syntax_tree:        Option<String>,
    /// The bcc IR before ABI lowering, as `--emit=ir --backend=native`
    /// prints it.
    pub ir:                 Option<String>,
    /// The bcc IR after ABI lowering, as `--emit=ir-abi --backend=native`
    /// prints it.
    pub ir_abi:             Option<String>,
    /// The native back end's assembly, as `--emit=asm --backend=native`
    /// prints it.
    pub asm:                Option<String>,
    /// Each function's machine code, encoded from `asm` as
    /// `--native-object` would encode it. ELF targets only.
    pub machine_code:       Option<Vec<ExplorerFunctionBytes>>,
    /// Which lines of the outputs each function definition became.
    pub line_map:           Vec<ExplorerLineMapping>,
}

/// One diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerDiagnostic {
    /// `"error"`, `"warning"` or `"note"`. A note explains why an output is
    /// missing without the program being wrong, such as machine code for a
    /// Windows target.
    pub severity:   String,
    /// Which part of the compiler reported it: `"front-end"` (preprocessing,
    /// parsing and semantic analysis), `"middle-end"` (lowering to the IR)
    /// or `"back-end"` (assembly and machine code).
    pub stage:      String,
    /// The headline, such as `expected ';' after expression`.
    pub message:    String,
    /// The file of the range the diagnostic is about: `<input>` for the
    /// source, or a header's name. `None` with the other location fields
    /// when the diagnostic has no place. This and the lines and columns
    /// are physical: `#line` changes only the place `rendered` names.
    pub file:       Option<String>,
    /// The 1-based line where the range starts.
    pub line:       Option<u32>,
    /// The 1-based column, in characters, where the range starts.
    pub column:     Option<u32>,
    /// The 1-based line of the range's end, in `file`.
    pub end_line:   Option<u32>,
    /// The 1-based column, in characters, just past the range's end.
    pub end_column: Option<u32>,
    /// The diagnostic as the CLI renders it, with its source excerpt,
    /// notes and help, without color.
    pub rendered:   String,
}

/// A function's machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerFunctionBytes {
    /// The symbol name.
    pub name:         String,
    /// The function's bytes, from its symbol's address to its end.
    pub bytes:        Vec<u8>,
    /// Each instruction in address order, as `objdump -d` would list it.
    pub instructions: Vec<ExplorerInstruction>,
}

/// One machine instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerInstruction {
    /// The offset from the function's first byte.
    pub offset: u32,
    /// The instruction's encoding.
    pub bytes:  Vec<u8>,
    /// The assembly line it encodes, as `asm` spells it.
    pub asm:    String,
}

/// The lines of each output that one function definition produced. The IR
/// keeps no source locations, so the mapping is per function: a function's
/// source lines map to its whole body in each output.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerLineMapping {
    /// The function's name in C. Each output names it by the symbol it is
    /// emitted as, which an `__asm__` label can change.
    pub function: String,
    /// The definition's physical lines in the source, which `#line` does
    /// not change.
    pub source:   ExplorerLineRange,
    /// Its lines in [`ExplorerOutput::ir`], from its header to its closing
    /// brace.
    pub ir:       Option<ExplorerLineRange>,
    /// Its lines in [`ExplorerOutput::ir_abi`].
    pub ir_abi:   Option<ExplorerLineRange>,
    /// Its lines in [`ExplorerOutput::asm`], from its label to the
    /// directive that ends it.
    pub asm:      Option<ExplorerLineRange>,
}

/// One logical source line: physical lines joined by line splicing, after
/// trigraph replacement and new-line mapping.
/// C99: translation phases 1-2, §5.1.1.2 paragraph 1, pp. 9-10; PDF
/// pp. 21-22.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerLogicalLine {
    /// The line's characters, without its new-line.
    pub text:    String,
    /// The physical lines it was read from.
    pub source:  ExplorerLineRange,
    /// What phases 1 and 2 changed in it, in order.
    pub changes: Vec<ExplorerSourceChange>,
}

/// One place where phases 1 and 2 changed the source, or where it holds a
/// character outside the basic source character set.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerSourceChange {
    /// `"splice"` (a backslash and new-line deleted), `"trigraph"` (three
    /// characters replaced by one), `"crlf"` (a carriage return and line
    /// feed read as one new-line), or `"extended"` (a UTF-8 character kept
    /// as one source character).
    pub kind:          String,
    /// The 1-based column, in characters, in the logical line's text where
    /// it happened: of the replacement, or of the character after a splice.
    pub column:        u32,
    /// The 1-based physical line where the source spelled it.
    pub line:          u32,
    /// The 1-based character column there.
    pub source_column: u32,
    /// What the source spelled, such as `??=` or a backslash and new-line.
    pub spelling:      String,
    /// What phases 1 and 2 left: `#` for `??=`, nothing for a splice.
    pub replacement:   String,
}

/// One phase-3 decomposition entry: a preprocessing token, or white space,
/// in which a comment is one space.
/// C99: §5.1.1.2 paragraph 1 item 3, p. 10; PDF p. 22; §6.4 paragraphs
/// 1-3, p. 49; PDF p. 61.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent property of the entry that the view shows."
)]
pub struct ExplorerPpToken {
    /// `"identifier"`, `"pp-number"`, `"character-constant"`,
    /// `"string-literal"`, `"punctuator"`, `"other"`, `"whitespace"` or
    /// `"newline"`. bcc's lexer forms no header names: `#include` reads
    /// its operand's tokens as one, which `header_name` marks.
    pub kind:          String,
    /// The token's spelling after phases 1-2, or one space for white space.
    pub spelling:      String,
    /// The 1-based physical line where it starts.
    pub line:          u32,
    /// The 1-based character column where it starts.
    pub column:        u32,
    /// The physical line where it ends.
    pub end_line:      u32,
    /// The character column just past its end.
    pub end_column:    u32,
    /// White space that holds a comment.
    pub comment:       bool,
    /// A token that begins a logical line, where `#` starts a directive.
    pub line_start:    bool,
    /// A token that white space precedes.
    pub leading_space: bool,
    /// A token that `#include` read as part of a header name.
    pub header_name:   bool,
    /// For a universal-character identifier, the name it stands for.
    pub canonical:     Option<String>,
}

/// Where one line of the preprocessed text came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerOutputLine {
    /// The physical file its text comes from, `<input>` or a header's
    /// name; for a line marker, the file the next text comes from.
    pub file:    String,
    /// The physical line its text comes from; `None` for a line marker, a
    /// pragma written back, or a blank line with no known place.
    pub line:    Option<u32>,
    /// For a line from a header, the physical line of `<input>` whose
    /// `#include` brought it in, at the outermost level.
    pub include: Option<u32>,
}

/// What phase 4 did: its macro expansions, conditional groups, includes and
/// definitions, in source order.
/// C99: §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerPreprocessing {
    /// Every replacement of a macro name in the source, with what it
    /// produced and how.
    pub expansions:   Vec<ExplorerExpansion>,
    /// Every conditional directive that decided whether its group is
    /// processed.
    pub conditionals: Vec<ExplorerConditional>,
    /// The lines of each group that was skipped.
    pub skipped:      Vec<ExplorerSkippedLines>,
    /// Every `#include`, with the header it found.
    pub includes:     Vec<ExplorerInclude>,
    /// `#define` and `#undef`, in order.
    pub definitions:  Vec<ExplorerDefinition>,
    /// Every directive that was executed, in order; those in skipped groups
    /// were not.
    pub directives:   Vec<ExplorerDirective>,
}

/// One replacement of a macro name in a file, and everything it set off.
/// C99: §6.10.3 paragraphs 9-10, p. 152; PDF p. 164; §6.10.3.4 paragraph
/// 1, p. 155; PDF p. 167.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerExpansion {
    /// The file the invocation is in: `<input>` or a header's name.
    pub file:       String,
    /// Its 1-based physical line.
    pub line:       u32,
    /// Its 1-based character column.
    pub column:     u32,
    /// The physical line where the invocation ends.
    pub end_line:   u32,
    /// The character column just past its end.
    pub end_column: u32,
    /// The place `#line` gives it, as `file:line`, when that differs from
    /// the physical one.
    pub presumed:   Option<String>,
    /// The invocation as written, each run of white space as one space.
    pub invocation: String,
    /// Whether a directive read it, as `#if` reads its condition; its
    /// result then goes to the directive, not to the output.
    pub directive:  bool,
    /// The tokens it produced, spelled as `-E` would write them; empty for
    /// a replacement that produced none or that a directive read.
    pub result:     String,
    /// The replacements, `#` and `##` operations it took, in order.
    pub steps:      Vec<ExplorerExpansionStep>,
}

/// One step of an expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerExpansionStep {
    /// `"replace"` (a macro name replaced), `"builtin"` (a predefined
    /// macro such as `__LINE__` replaced), `"stringify"` (`#`) or
    /// `"paste"` (`##`).
    pub kind:       String,
    /// Whether the name was read from an argument, which is macro-replaced
    /// before it is substituted (C99 §6.10.3.1p1).
    pub prescan:    bool,
    /// The macro replaced, or whose replacement list holds the operator.
    pub macro_name: String,
    /// How deep in the tree of replacements the step is: 0 for the name in
    /// the source, 1 for a step in its replacement or its arguments, and so
    /// on.
    pub depth:      u32,
    /// The index, among the expansion's steps, of the replacement this step
    /// happened in: the innermost one being read. `None` at depth 0.
    pub parent:     Option<u32>,
    /// For a function-like macro, its parameters and their arguments.
    pub arguments:  Vec<ExplorerArgument>,
    /// For `##`, its operands, empty for a placemarker.
    pub operands:   Vec<String>,
    /// For `#`, `##` and a predefined macro, the token made; empty for a
    /// placemarker.
    pub result:     String,
}

/// A parameter and the argument it stands for, as written.
/// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerArgument {
    /// The parameter's name, or `__VA_ARGS__`.
    pub parameter: String,
    /// The argument's tokens, each run of white space as one space.
    pub argument:  String,
}

/// A conditional directive and whether it chose its group.
/// C99: §6.10.1 paragraph 6, p. 149; PDF p. 161.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerConditional {
    /// The file the directive is in.
    pub file:      String,
    /// The directive's 1-based physical line.
    pub line:      u32,
    /// The place `#line` gives it, when that differs.
    pub presumed:  Option<String>,
    /// The directive's line as written, such as `#if DEBUG > 1`.
    pub directive: String,
    /// Whether the group it controls is processed.
    pub taken:     bool,
    /// Whether its condition was evaluated; an arm after a processed group
    /// is skipped without it.
    pub evaluated: bool,
}

/// The physical lines of a group that was skipped, both inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerSkippedLines {
    /// The file the group is in.
    pub file:  String,
    /// The group's lines, without the directives around it.
    pub lines: ExplorerLineRange,
}

/// An `#include` and the header it found.
/// C99: §6.10.2 paragraphs 2-4, pp. 149-150; PDF pp. 161-162.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerInclude {
    /// The file the directive is in.
    pub file:   String,
    /// The directive's 1-based physical line.
    pub line:   u32,
    /// The header name without its delimiters.
    pub header: String,
    /// Whether it was written `<…>` rather than `"…"`.
    pub angled: bool,
    /// The header found: `None` when there was none; the embedded resource
    /// headers are system headers.
    pub found:  Option<String>,
    /// Whether the header found is a system header.
    pub system: bool,
    /// The number of lines the header holds.
    pub lines:  u32,
}

/// A `#define` or `#undef`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerDefinition {
    /// The file the directive is in.
    pub file:      String,
    /// The directive's 1-based physical line.
    pub line:      u32,
    /// The macro's name.
    pub name:      String,
    /// `"object"`, `"function"` or `"undef"`.
    pub kind:      String,
    /// The directive's line as written.
    pub directive: String,
}

/// A directive that was executed.
/// C99: §6.10 paragraph 2, pp. 146-147; PDF pp. 158-159.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerDirective {
    /// The file the directive is in.
    pub file:      String,
    /// Its 1-based physical line.
    pub line:      u32,
    /// The place `#line` gives it, when that differs.
    pub presumed:  Option<String>,
    /// Its name, such as `pragma` or `error`.
    pub name:      String,
    /// Its line as written.
    pub directive: String,
}

/// 1-based lines, both inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerLineRange {
    /// The first line.
    pub start: u32,
    /// The last line.
    pub end:   u32,
}

impl Default for ExplorerOptions {
    fn default() -> Self {
        Self {
            target:            "x86_64-unknown-linux-gnu".into(),
            std:               "c17".into(),
            optimize:          false,
            max_nesting_depth: None,
            source_date_epoch: None,
            files:             Vec::new(),
            syntax_locations:  false,
        }
    }
}
