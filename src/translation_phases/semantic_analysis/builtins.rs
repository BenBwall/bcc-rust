//! Builtin recognition lets preprocessing ask which calls semantic analysis can
//! check. Keyword intrinsics, named atomic calls and x86 calls have separate
//! classifiers. Their operands are visited by the ordinary semantic work stack
//! before each call's types and constraints are checked. No backend lowering
//! happens here.
//!
//! Read [`modeled`] for keyword recognition and [`named_builtin`] for name
//! recognition, then [`Analyzer::type_builtin`](super::Analyzer::type_builtin)
//! for header intrinsic dispatch.
//! [`Analyzer::atomic_call`](super::Analyzer::atomic_call) and
//! [`Analyzer::x86_builtin_operand`](super::Analyzer::x86_builtin_operand)
//! check the named call families.
//!
//! - Header intrinsics: `intrinsics.rs` handles varargs, type operands and
//!   member offsets; `names.rs` joins the named-call recognition queries.
//! - Atomic calls: `atomics.rs` classifies and checks atomic operands.
//! - Target calls: `x86_builtins.rs` decodes signatures and checks operands;
//!   `x86_builtin_table.rs` holds the generated signature and immediate table.
//!
//! C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22 (translation phase 7).
//! C99: §7.15, pp. 249-252; PDF pp. 261-264; §7.17 paragraph 3, p. 254;
//! PDF p. 266 (header intrinsics).
//! Named calls are GCC/Clang extensions.
//! C99: §6.5.2.2, pp. 71-72; PDF pp. 83-84 (extended function calls).

// Header intrinsics and recognition
mod intrinsics;
mod names;

// Atomic calls
pub(super) mod atomics;

// Target signatures and operands
mod x86_builtin_table;
pub(crate) mod x86_builtins;

pub(crate) use modeled as implemented_builtin;
pub(crate) use names::named_builtin;

use super::{
    expressions,
    integer::Integer,
};
use crate::{
    translation_phases::{
        parsing::syntax::{
            ExpressionType,
            Identifier,
        },
        preprocessing::KeywordTokenType,
    },
    util::arena_list::ArenaList,
};

/// Intrinsics supporting standard headers are available even in strict modes.
/// GCC/Clang implementation keywords extending C99 §7.15, pp. 249-252;
/// PDF pp. 261-264 and §7.17p3, p. 254; PDF p. 266.
pub(crate) fn modeled(keyword: KeywordTokenType) -> bool {
    matches!(
        keyword,
        KeywordTokenType::BuiltinVaArg
            | KeywordTokenType::BuiltinVaStart
            | KeywordTokenType::BuiltinVaEnd
            | KeywordTokenType::BuiltinVaCopy
            | KeywordTokenType::BuiltinTypesCompatible
            | KeywordTokenType::BuiltinChooseExpr
            | KeywordTokenType::BuiltinBitCast
            | KeywordTokenType::BuiltinConvertVector
            | KeywordTokenType::BuiltinOffsetof
    )
}
