//! Which format a call's callee follows: a GNU `format` attribute on any of
//! its declarations, or, for the standard library's format functions, the
//! builtin each compiler knows by its name. A library function counts only
//! where the compilers treat it as their builtin: hosted, not named by
//! `-fno-builtin`, with external linkage and a declaration of the library's
//! shape, which for Clang includes a `FILE` structure for the stream.
//! C99: §7.19.6.1-§7.19.6.14, pp. 274-295; PDF pp. 286-307;
//! §7.1.4p1, pp. 166-167; PDF pp. 178-179.

use super::{
    super::{
        Analyzer,
        BindingKind,
        ExpressionInfo,
        Linkage,
        TypeId,
        TypeKind,
        facts::FormatKind,
    },
    FormatCall,
};
use crate::{
    configuration::{
        CompilerIdentity,
        LibraryBuiltins,
    },
    translation_phases::parsing::syntax::ExpressionType,
};

/// A format function's contract.
#[derive(Clone, Copy)]
pub(super) struct Contract {
    pub(super) kind:   FormatKind,
    /// The format parameter, from 0.
    pub(super) format: usize,
    /// The first data parameter, from 0, or `None` for a `va_list`.
    pub(super) first:  Option<usize>,
}

/// A standard format function: its name, its kind, its format parameter,
/// whether it takes `...` (or a `va_list` last) and whether its first
/// parameter is a stream.
struct Library {
    name:     &'static str,
    kind:     FormatKind,
    format:   usize,
    variadic: bool,
    stream:   bool,
}

const fn library(
    name: &'static str,
    kind: FormatKind,
    format: usize,
    variadic: bool,
    stream: bool,
) -> Library {
    Library {
        name,
        kind,
        format,
        variadic,
        stream,
    }
}

/// The standard library's format functions both compilers check as
/// builtins, and `strftime`, whose format GCC wants a string literal.
/// C99: §7.19.6, pp. 274-295; PDF pp. 286-307; §7.23.3.5, pp. 343-347;
/// PDF pp. 355-359.
const LIBRARY: &[Library] = &[
    library("printf", FormatKind::Printf, 0, true, false),
    library("fprintf", FormatKind::Printf, 1, true, true),
    library("sprintf", FormatKind::Printf, 1, true, false),
    library("snprintf", FormatKind::Printf, 2, true, false),
    library("vprintf", FormatKind::Printf, 0, false, false),
    library("vfprintf", FormatKind::Printf, 1, false, true),
    library("vsprintf", FormatKind::Printf, 1, false, false),
    library("vsnprintf", FormatKind::Printf, 2, false, false),
    library("scanf", FormatKind::Scanf, 0, true, false),
    library("fscanf", FormatKind::Scanf, 1, true, true),
    library("sscanf", FormatKind::Scanf, 1, true, false),
    library("vscanf", FormatKind::Scanf, 0, false, false),
    library("vfscanf", FormatKind::Scanf, 1, false, true),
    library("vsscanf", FormatKind::Scanf, 1, false, false),
    library("strftime", FormatKind::Unchecked, 2, false, false),
];

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The format contract of the function a call names, if it has one.
    pub(super) fn format_contract(&mut self, function: ExpressionInfo<'tu>) -> Option<Contract> {
        if let Some(binding) = function.binding {
            let data = self.bindings[binding];
            if data.kind != BindingKind::Function {
                return None;
            }
            if let Some(attribute) = self.format_attributes(binding).0 {
                return Some(Contract {
                    kind:   attribute.kind,
                    format: attribute.format as usize - 1,
                    first:  (attribute.first as usize).checked_sub(1),
                });
            }
            let name = self.context.string_cache.at(data.name.name);
            if let Some(name) = name.strip_prefix("__builtin_") {
                return LIBRARY
                    .iter()
                    .find(|library| library.name == name)
                    .map(library_contract);
            }
            if data.linkage != Linkage::External
                || !self.context.configuration.hosted()
                || match self.context.library_builtins {
                    | LibraryBuiltins::All => false,
                    | LibraryBuiltins::None => true,
                    | LibraryBuiltins::Except(names) => names.contains(&name),
                }
            {
                return None;
            }
            let library = LIBRARY.iter().find(|library| library.name == name)?;
            return self
                .library_shape(data.ty, library)
                .then(|| library_contract(library));
        }
        // `__builtin_printf` and its kin, whatever else the options say.
        let callee = self.selected_designator(function.expression);
        let ExpressionType::Identifier(name) = callee.kind else {
            return None;
        };
        let name = self
            .context
            .string_cache
            .at(name.name)
            .strip_prefix("__builtin_")?;
        LIBRARY
            .iter()
            .find(|library| library.name == name)
            .map(library_contract)
    }

    /// Whether the declared type `ty` has the shape of `library`'s, so that
    /// the compilers take the declaration for their builtin: as many
    /// parameters, a character pointer for the format and `...` or a
    /// `va_list` after it. Clang wants a structure behind the stream.
    fn library_shape(&self, ty: TypeId, library: &Library) -> bool {
        let TypeKind::Function {
            parameters,
            prototype: true,
            variadic,
            ..
        } = self.types.nodes[ty.index]
        else {
            return false;
        };
        let count = library.format + 1 + usize::from(!library.variadic);
        if variadic != library.variadic || parameters.len() != count {
            return false;
        }
        let character = |analyzer: &Self, ty: TypeId| {
            analyzer.pointer_target(ty).is_some_and(|target| {
                matches!(
                    analyzer.types.nodes[target.index],
                    TypeKind::Scalar(super::super::Scalar::Char)
                )
            })
        };
        if !character(self, parameters[library.format]) {
            return false;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        if library.stream
            && !self.pointer_target(parameters[0]).is_some_and(|target| {
                gcc || matches!(self.types.nodes[target.index], TypeKind::Tag(_))
            })
        {
            return false;
        }
        library.variadic
            || matches!(
                self.types.nodes[parameters[count - 1].index],
                TypeKind::Pointer(_) | TypeKind::Array(..)
            )
    }
}

const fn library_contract(library: &Library) -> Contract {
    Contract {
        kind:   library.kind,
        format: library.format,
        first:  if library.variadic {
            Some(library.format + 1)
        } else {
            None
        },
    }
}

impl FormatCall<'_, '_> {
    /// Whether the format's data come as a `va_list`, so that only the
    /// string is checked.
    pub(super) const fn takes_va_list(&self) -> bool {
        self.first.is_none()
    }
}
