//! GCC's and Clang's `-Wformat` family: a call to a `printf`- or
//! `scanf`-like function whose format string does not match the arguments
//! that follow it. The conversion specifications C99 §7.19.6.1 and
//! §7.19.6.2 define say what each data argument must be; a mismatch, too
//! few arguments, an invalid specification or a format that is no string
//! literal earns a warning, which §5.1.1.3p1 permits.
//!
//! A function has a format when it is the standard library's
//! (`fprintf`, `printf`, `sprintf`, `snprintf`, their `v` forms and the
//! `scanf` family, by name with external linkage, as both compilers know
//! them as builtins) or when a declaration gives it GNU
//! `format(archetype, string-index, first-to-check)` (`attribute.rs`). The
//! format argument is followed to its string literals (`literal.rs`):
//! through casts, a conditional's arms, a `format_arg` function's argument
//! and a constant object's initializer, each compiler as far as it goes.
//! Each compiler then reads each string its own way, in its own words and
//! at its own places: GCC as `c-format.cc` does (`gcc.rs`), Clang as its
//! format-string analysis does (`clang.rs`).
//!
//! C99: §7.19.6.1, pp. 274-281; PDF pp. 286-293 (`fprintf`);
//! §7.19.6.2, pp. 282-288; PDF pp. 294-300 (`fscanf`);
//! §6.5.2.2p7, p. 72; PDF p. 84 (default argument promotions).

mod clang;
mod contract;
mod gcc;
mod literal;

use super::{
    Analyzer,
    ExpressionInfo,
    SemanticErrorKind,
    facts::FormatKind,
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticGroup,
        RuleSet,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Initializer,
                InitializerType,
            },
            syntax::Expression,
        },
    },
    util::arena_list::ArenaList,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the call `e` of `function` with `arguments` against the
    /// format its callee declares, as the compiler bcc stands for does.
    /// C99: §7.19.6.1p2, p. 274; PDF p. 286; §7.19.6.2p2, p. 282; PDF p. 294.
    pub(super) fn check_format_call(
        &mut self,
        e: &'tu Expression<'tu>,
        function: ExpressionInfo<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) {
        let configuration = self.context.configuration;
        let gcc = configuration.compiler_identity() == CompilerIdentity::Gcc;
        if !(gcc || configuration.compiler_identity().rules() == RuleSet::Clang)
            || !matches!(
                configuration.target(),
                crate::target::Target::LinuxGnu | crate::target::Target::LinuxMusl
            )
            || self.tainted
        {
            return;
        }
        // GCC checks formats only while `-Wformat` is on, whatever the
        // options of the groups under it say.
        if gcc
            && configuration
                .diagnostic_severity(DiagnosticGroup::Format)
                .is_none()
        {
            return;
        }
        let Some(contract) = self.format_contract(function) else {
            return;
        };
        // Clang does not read a `strftime` format, and needs none literal.
        if contract.kind == FormatKind::Unchecked && !gcc {
            return;
        }
        let arguments = self
            .scratch
            .alloc_slice_fill_iter(arguments.iter().copied());
        if arguments.iter().any(|argument| argument.recovered)
            || arguments
                .iter()
                .any(|&argument| self.types.unanalyzed(self.expression_info(argument).ty))
        {
            return;
        }
        let Some(&format) = arguments.get(contract.format) else {
            return;
        };
        let call = FormatCall {
            input_location: if gcc {
                self.gcc_input_location(e)
            } else {
                e.source_vectors
            },
            call: e,
            kind: contract.kind,
            format,
            format_index: contract.format,
            first: contract.first,
            arguments,
        };
        if gcc {
            self.gcc_check_format(&call);
        } else {
            self.clang_check_format(&call);
        }
    }

    /// Reports the format diagnostic `words` of `group` at `site`, with the
    /// note `note` at `note_site` when there is one, under the diagnostic
    /// settings where the call `settings_at` is. GCC withholds it when a
    /// system header spells the place; both withhold it in a system header's
    /// invocation.
    fn format_warning(
        &mut self,
        settings_at: SourceVectors,
        group: DiagnosticGroup,
        words: &str,
        site: SourceVectors,
        note: Option<(FormatNote, SourceVectors)>,
    ) {
        let name = self.context.string_cache.intern(words);
        let kind = SemanticErrorKind::Format(group, note.map_or(FormatNote::DefinedHere, |n| n.0));
        let previous = note.map(|n| n.1);
        if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc
            && self.context.spelled_in_system_header(site)
        {
            return;
        }
        self.group_diagnostic_read_at(group, kind, site, Some(name), previous, settings_at);
    }
}

impl Analyzer<'_, '_, '_> {
    /// GCC's `input_location` at the end of the call `e`, which the first
    /// token of each line moves: the first token on the line of the call's
    /// closing parenthesis, or of its invocation when a macro wrote it.
    fn gcc_input_location(&mut self, e: &Expression<'_>) -> SourceVectors {
        let last = crate::translation_phases::Context::last_source_vector(e.source_vectors);
        if last.length() == 0 {
            return e.source_vectors;
        }
        let vector = self.context.file_vector(last).clone();
        let Some(text) = self.context.source_text(vector.source_file_index) else {
            return e.source_vectors;
        };
        let bytes = text.as_bytes();
        let end = (vector.index as usize).min(bytes.len());
        let line_start = bytes[..end]
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |at| at + 1);
        let Some(first) = bytes[line_start..end]
            .iter()
            .position(|&byte| !matches!(byte, b' ' | b'\t' | b'\x0c' | b'\x0b' | b'\r'))
            .map(|offset| line_start + offset)
        else {
            return last;
        };
        // A call on one line is its own first token, which GCC underlines
        // with the call.
        if self.context.file_vector(e.source_vectors).index as usize == first
            && self.context.file_vector(e.source_vectors).source_file_index
                == vector.source_file_index
        {
            return e.source_vectors;
        }
        let column = 1 + text[line_start..first].chars().count();
        self.context.create_retained_source_vectors(
            crate::translation_phases::SourcePosition {
                index:  first,
                line:   vector.line,
                column: u32::try_from(column).unwrap_or(u32::MAX),
            },
            vector.source_file_index,
            text[first..].chars().next().map_or(1, char::len_utf8),
        )
    }
}

/// A call to check: its callee's format contract and its arguments.
struct FormatCall<'a, 'tu> {
    call:           &'tu Expression<'tu>,
    /// Where GCC reports what concerns the call: its `input_location`.
    input_location: SourceVectors,
    kind:           FormatKind,
    /// The format argument.
    format:         &'tu Expression<'tu>,
    /// Its position among the arguments, from 0.
    format_index:   usize,
    /// The position of the first data argument, from 0, or `None` when the
    /// function takes its data as a `va_list` (`first-to-check` 0), so that
    /// only the format string is checked.
    first:          Option<usize>,
    arguments:      &'a [&'tu Expression<'tu>],
}

impl<'tu> FormatCall<'_, 'tu> {
    /// The data arguments the format converts.
    fn data(&self) -> &[&'tu Expression<'tu>] {
        self.first
            .map_or(&[], |first| self.arguments.get(first..).unwrap_or_default())
    }
}

/// What a format diagnostic's note says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormatNote {
    /// Where the format string is, when the diagnostic is elsewhere.
    DefinedHere,
    /// Clang's advice for a format that is no string literal.
    TreatAsArgument,
    /// Clang: where the function it suggests an attribute for, by name, is
    /// declared.
    DeclaredHere(crate::util::string_cache::StringCacheId),
}

impl FormatNote {
    pub(super) fn words<'d>(
        self,
        context: &crate::translation_phases::Context<'_>,
        arena: &'d crate::util::bump::Bump,
    ) -> &'d str {
        match self {
            | Self::DefinedHere => "format string is defined here",
            | Self::TreatAsArgument => "treat the string as an argument to avoid this",
            | Self::DeclaredHere(name) => crate::diagnostics::format_in!(
                arena,
                "'{}' declared here",
                context.string_cache.at(name)
            ),
        }
    }
}

/// The expression an initializer is, a braced list of one included, as
/// `{ "..." }` initializes a character array.
/// C99: §6.7.8p14, p. 126; PDF p. 138.
fn initializer_expression<'tu>(initializer: &'tu Initializer<'tu>) -> Option<&'tu Expression<'tu>> {
    match initializer.kind {
        | InitializerType::AssignmentExpression(expression) => Some(expression),
        | InitializerType::InitializerList(list) => {
            let mut elements = list.elements.iter();
            let element = elements.next()?;
            if elements.next().is_some() || element.designation.is_some() {
                return None;
            }
            initializer_expression(element.initializer)
        },
    }
}
