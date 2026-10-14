//! Builtin recognition lets preprocessing ask which calls semantic analysis can
//! check. Scalar/library contracts come from the neutral registry; remaining
//! atomic, x86, vector and type-generic classifiers are explicit M6 bridges.
//! Their operands are visited by the ordinary semantic work stack before each
//! call's types and constraints are checked. No backend lowering happens here.
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
//!   `registry.rs` projects signatures and diagnostic identities; `library.rs`
//!   retains the implicit-library warning policy. `common.rs` checks scalar
//!   calls; `effects.rs` produces the retained dual-context summary once during
//!   the ordinary expression postorder. `designators.rs` rejects non-call uses
//!   of intrinsic-only functions.
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
mod common;
mod designators;
mod effects;
mod floating;
mod intrinsics;
mod library;
mod names;
pub(super) mod registry;

// Atomic calls
pub(super) mod atomics;

// Target signatures and operands
mod x86_builtin_table;
mod x86_builtins;

pub(crate) use effects::ExpressionEffects;
pub(crate) use modeled as implemented_builtin;
pub(super) use names::legacy_named_builtin;
pub(crate) use names::named_builtin;

use super::expressions;
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
    registry::keyword_descriptor(keyword).is_some()
        || matches!(
            keyword,
            KeywordTokenType::BuiltinTypesCompatible
                | KeywordTokenType::BuiltinChooseExpr
                | KeywordTokenType::BuiltinBitCast
                | KeywordTokenType::BuiltinConvertVector
        )
}
