//! Named builtin recognition for preprocessing queries in support of
//! translation phase 7. Operand checking stays with each builtin call family.
//! These calls are GCC/Clang extensions.
//! C99: §6.5.2.2, pp. 71-72; PDF pp. 83-84 (extended function calls).

use super::{
    atomics::modeled as atomic_builtin,
    x86_builtins,
};
use crate::translation_phases::semantic_analysis::{
    type_generic,
    vectors,
};

/// Builtins parsed as ordinary calls, queried by the preprocessor.
/// GCC/Clang extensions to C99 §6.5.2.2, pp. 71-72; PDF pp. 83-84.
pub(crate) fn named_builtin(name: &str) -> bool {
    type_generic::named_builtin(name)
        || vectors::named_builtin(name)
        || atomic_builtin(name)
        || x86_builtins::known(name)
}
