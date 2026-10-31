//! Builtin checking. Every builtin spelling and keyword form has one
//! descriptor in the neutral `crate::builtins` registry, which also answers
//! preprocessing's `__has_builtin`. A binding created for a registered
//! spelling records `BindingOrigin::Intrinsic(id)`; the descriptor's type
//! rule then selects the family handler that checks the call. Operands are
//! visited by the ordinary semantic work stack before each call's types and
//! constraints are checked. No backend lowering happens here.
//!
//! Read [`Analyzer::common_builtin_call`](super::Analyzer::common_builtin_call)
//! for named-call dispatch and
//! [`Analyzer::type_builtin`](super::Analyzer::type_builtin) for keyword forms.
//! [`Analyzer::atomic_call`](super::Analyzer::atomic_call) and
//! [`Analyzer::x86_immediates`](super::Analyzer::x86_immediates) check the
//! atomic and x86 families.
//!
//! - Header intrinsics: `intrinsics.rs` handles varargs, type operands and
//!   member offsets. `registry.rs` projects signatures, diagnostic identities
//!   and keyword forms; `library.rs` retains the implicit-library warning
//!   policy; `checked.rs` forms the target `va_list` and `FILE *` parameters of
//!   checked library calls. `common.rs` checks scalar calls; their effect
//!   classes feed the retained expression effects in `facts/effects.rs`.
//!   `designators.rs` rejects non-call uses of intrinsic-only functions.
//! - Atomic calls: `atomics.rs` checks atomic operands and orders.
//! - Type-generic bit operations: `bits.rs` types, checks and folds
//!   `__builtin_popcountg`, its siblings and the `__builtin_stdc_*` forms.
//! - Target and overloaded calls: `x86_builtins.rs` checks generated x86
//!   immediates and the overloaded vector builtins. Vector shuffles,
//!   conversions and bit casts are in `../vectors.rs`; type classification,
//!   selection and binary128 primitives in `../type_generic.rs`.
//!
//! C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22 (translation phase 7).
//! C99: §7.15, pp. 249-252; PDF pp. 261-264; §7.17 paragraph 3, p. 254;
//! PDF p. 266 (header intrinsics).
//! Named calls are GCC/Clang extensions.
//! C99: §6.5.2.2, pp. 71-72; PDF pp. 83-84 (extended function calls).

// Header intrinsics and recognition
mod checked;
mod common;
mod designators;
mod floating;
mod intrinsics;
mod library;
pub(super) mod registry;

// Atomic calls
mod atomics;

// Type-generic bit operations
mod bits;

// Target immediates and overloaded operands
mod x86_builtins;

pub(super) use floating::{
    half_payload,
    nan_payload,
    nan_payload_wide,
};

use super::expressions;
use crate::{
    translation_phases::{
        parsing::syntax::Identifier,
        preprocessing::KeywordTokenType,
    },
    util::arena_list::ArenaList,
};

/// Intrinsics supporting standard headers are available even in strict modes.
/// GCC/Clang implementation keywords extending C99 §7.15, pp. 249-252;
/// PDF pp. 261-264 and §7.17p3, p. 254; PDF p. 266.
pub(super) fn modeled(keyword: KeywordTokenType) -> bool {
    registry::keyword_descriptor(keyword).is_some()
}
