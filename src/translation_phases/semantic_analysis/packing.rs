//! Interpret translation-ordered packing directives once, before declaration
//! work. Record definitions retain the cap at their opening token; completion
//! applies it to members. Includes and pragma operators share output-token
//! ordinals. C99: §6.10.6p1, p. 159; PDF p. 171 (implementation-defined
//! behavior). GCC: <https://gcc.gnu.org/onlinedocs/gcc/Structure-Layout-Pragmas.html>.
//! MSVC: <https://learn.microsoft.com/en-us/cpp/preprocessor/pack>.
//! Clang: <https://clang.llvm.org/doxygen/SemaAttr_8cpp_source.html>.

use super::{
    SemanticError,
    SemanticErrorKind,
};
use crate::{
    translation_phases::{
        Context,
        TranslationError,
        pragma_pack::PackToken,
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// Build searchable states without source-location joins or a syntax walk.
/// The packing before any pragma, which `#pragma pack()` restores, is the
/// configured default (`-fpack-struct=N`, `cl /ZpN`), as GCC and MSVC
/// document; under `-fpack-struct` alone the pragmas are ignored. C99:
/// §6.10.6p1, p. 159; PDF p. 171. GCC/MSVC stack extension.
pub(super) fn states<'s>(context: &mut Context<'_>, scratch: &'s Bump) -> &'s [(usize, u32)] {
    let mut stack = ArenaVec::new_in(scratch);
    let mut states = ArenaVec::new_in(scratch);
    let default = context.configuration.default_pack();
    let mut current = default;
    if default != 0 {
        states.push((0, default));
    }
    for index in 0..context.pack_pragmas.len() {
        let pragma = context.pack_pragmas[index];
        // GCC's `-fpack-struct` without a value packs every structure and
        // ignores the pragmas, which `-fpack-struct=N` lets override it.
        if context.configuration.pack_structures() {
            warn(context, SemanticErrorKind::PragmaPackIgnored, pragma.source);
            continue;
        }
        let warning = match parse(context, pragma.operands) {
            | Some(Action::Set(value)) => {
                current = if value == 0 { default } else { value };
                None
            },
            | Some(Action::Push(name, value)) => {
                stack.push((name, current, pragma.source));
                if let Some(value) = value {
                    current = value;
                }
                None
            },
            | Some(Action::Pop(name, value)) => {
                let warning = stack
                    .is_empty()
                    .then_some(SemanticErrorKind::PragmaPackPopEmpty);
                if name.is_some() && value.is_some() {
                    warn(
                        context,
                        SemanticErrorKind::PragmaPackPopNameAlignment,
                        pragma.source,
                    );
                }
                let slot = name.map_or_else(
                    || stack.len().checked_sub(1),
                    |name| stack.iter().rposition(|&(id, _, _)| id == Some(name)),
                );
                if let Some(slot) = slot {
                    current = stack[slot].1;
                    stack.truncate(slot);
                }
                if let Some(value) = value {
                    current = value;
                }
                warning
            },
            | Some(Action::Show) => Some(SemanticErrorKind::PragmaPackShow(if current == 0 {
                8
            } else {
                current
            })),
            | None => Some(SemanticErrorKind::PragmaPackMalformed),
        };
        if let Some(kind) = warning {
            warn(context, kind, pragma.source);
        }
        states.push((pragma.position, current));
    }
    for &(_, _, source) in &stack {
        warn(context, SemanticErrorKind::PragmaPackUnterminated, source);
    }
    states.leak()
}

fn warn(
    context: &mut Context<'_>,
    kind: SemanticErrorKind,
    source: crate::translation_phases::SourceVectors,
) {
    if context.withholds(
        crate::translation_phases::ErrorSeverity::Warning,
        false,
        source,
    ) {
        return;
    }
    context.append_pending_errors([TranslationError::Semantic(SemanticError {
        extension_severity: None,
        kind,
        source_vectors: source,
        name: None,
        previous: None,
    })]);
}

/// Pack operands are macro-replaced preprocessing tokens, not C expressions.
/// GCC/MSVC forms permit a named push/pop and a literal power-of-two cap.
fn parse(context: &Context<'_>, tokens: &[PackToken]) -> Option<Action> {
    let spelling = |token: PackToken| context.string_cache.at(token.spelling);
    if tokens.len() < 2 || spelling(tokens[0]) != "(" || spelling(*tokens.last()?) != ")" {
        return None;
    }
    let inner = &tokens[1..tokens.len() - 1];
    if inner.is_empty() {
        return Some(Action::Set(0));
    }
    let first = inner[0];
    if inner.len() == 1 {
        return match spelling(first) {
            | "show" if first.identifier => Some(Action::Show),
            | "push" if first.identifier => Some(Action::Push(None, None)),
            | "pop" if first.identifier => Some(Action::Pop(None, None)),
            | number => alignment(number).map(Action::Set),
        };
    }
    if !first.identifier
        || !matches!(spelling(first), "push" | "pop")
        || !matches!(inner.len(), 3 | 5)
        || spelling(inner[1]) != ","
    {
        return None;
    }
    let (name, value) = if inner[2].identifier {
        let value = if inner.len() == 5 {
            if spelling(inner[3]) != "," {
                return None;
            }
            Some(alignment(spelling(inner[4]))?)
        } else {
            None
        };
        (Some(inner[2].spelling), value)
    } else {
        if inner.len() != 3 {
            return None;
        }
        (None, Some(alignment(spelling(inner[2]))?))
    };
    Some(if spelling(first) == "push" {
        Action::Push(name, value)
    } else {
        Action::Pop(name, value)
    })
}

fn alignment(text: &str) -> Option<u32> {
    // Phase-3 number spellings carry the terminator used by literal conversion.
    let text = text.strip_suffix('\0').unwrap_or(text);
    let digits = text.trim_end_matches(['u', 'U', 'l', 'L']);
    let suffix = &text[digits.len()..];
    if !["", "u", "l", "ll", "ul", "ull", "lu", "llu"]
        .iter()
        .any(|allowed| suffix.eq_ignore_ascii_case(allowed))
    {
        return None;
    }
    let text = digits;
    let (digits, radix) =
        if let Some(digits) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (digits, 16)
        } else if let Some(digits) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
            (digits, 2)
        } else if text.len() > 1 && text.starts_with('0') {
            (&text[1..], 8)
        } else {
            (text, 10)
        };
    let value = u32::from_str_radix(digits, radix).ok()?;
    matches!(value, 0 | 1 | 2 | 4 | 8 | 16).then_some(value)
}

enum Action {
    Set(u32),
    Push(Option<StringCacheId>, Option<u32>),
    Pop(Option<StringCacheId>, Option<u32>),
    Show,
}
