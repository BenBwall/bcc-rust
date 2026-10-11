//! Definitions retained until a macro is undefined or preprocessing ends.
//!
//! C99: translation phase 4; macro definitions, §6.10.3, pp. 151-153;
//! PDF pp. 163-165; macro definition lifetime, §6.10.3.5 paragraph 1, p. 155;
//! PDF p. 167. Replacement execution belongs to the macro reader.

use std::fmt::Debug;

use crate::{
    translation_phases::preprocessor_tokenizer::TokenSource,
    util::string_cache::StringCacheId,
};

/// A macro name's current definition. It lasts until `#undef` names it or
/// preprocessing ends.
///
/// C99: §6.10.3.5 paragraph 1, p. 155; PDF p. 167.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum MacroDefinition<'pp> {
    /// `# define identifier replacement-list new-line`; the tokenizer reads
    /// the replacement list.
    ///
    /// C99: §6.10.3 paragraph 9, p. 152; PDF p. 164.
    ObjectLike { tokenizer: TokenSource<'pp> },
    /// `# define identifier lparen identifier-list(opt) ) replacement-list
    /// new-line` and its `...` forms.
    ///
    /// C99: §6.10.3 paragraph 10, p. 152; PDF p. 164.
    FunctionLike {
        argument_names: &'pp [StringCacheId],
        tokenizer:      TokenSource<'pp>,
        is_variadic:    bool,
        /// GNU named variadic parameter, distinct from standard `__VA_ARGS__`.
        variadic_alias: Option<StringCacheId>,
    },
    /// A predefined macro or the `_Pragma` operator, which the macro-replacing
    /// reader expands through
    /// [`Expander::expand_builtin`](crate::translation_phases::preprocessing::Expander::expand_builtin).
    ///
    /// C99: §6.10.8 paragraph 1, p. 160; PDF p. 172, and §6.10.9 paragraph
    /// 1, p. 161; PDF p. 173.
    BuiltIn,
}
