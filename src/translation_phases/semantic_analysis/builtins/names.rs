use super::super::{
    atomic_builtin,
    type_generic,
    vectors,
    x86_builtins,
};

/// Builtins parsed as ordinary calls, queried by the preprocessor.
/// GCC/Clang extensions to C99 §6.5.2.2, pp. 71-72; PDF pp. 83-84.
pub(crate) fn named_builtin(name: &str) -> bool {
    type_generic::named_builtin(name)
        || vectors::named_builtin(name)
        || atomic_builtin(name)
        || x86_builtins::known(name)
}
