//! The predefined function-name identifiers `__func__`, `__FUNCTION__` and
//! `__PRETTY_FUNCTION__`.
//!
//! `__func__` is declared at function entry (`define_func_name` in
//! `functions.rs`). The GNU spellings follow Clang: `__FUNCTION__` is the same
//! object as `__func__`, and `__PRETTY_FUNCTION__` holds the function's
//! signature as Clang prints it, even in C. Both are made when an expression
//! first names them, because few functions do. Outside a function all three
//! are an empty string (`top level` for `__PRETTY_FUNCTION__`) behind a
//! warning, as in Clang and GCC.
//!
//! C99: §6.4.2.2 paragraph 1, p. 52; PDF p. 64. GNU extension: GCC manual,
//! "Function Names as Strings".
//! <https://gcc.gnu.org/onlinedocs/gcc/Function-Names.html>

use super::{
    Analyzer,
    Binding,
    BindingKind,
    BindingOrigin,
    Duration,
    Linkage,
    SemanticErrorKind,
    functions::{
        FunctionContext,
        function_derivation,
    },
    results::{
        Definition,
        DefinitionKind,
    },
    scopes::Namespace,
    types::{
        ArrayBound,
        Scalar,
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::CStandard,
    diagnostics::format_in,
    translation_phases::parsing::{
        declaration_syntax::{
            DirectDeclarator,
            TypeQualifiers,
            TypeSpecifiers,
        },
        syntax::Identifier,
    },
    util::{
        bump::{
            ArenaString,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// Which string a predefined identifier holds. `__FUNCTION__` is `Name`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Predefined {
    Name,
    Pretty,
}

/// A typedef name that stands for its type where a signature prints it, as
/// Clang prints the type as written.
#[derive(Clone, Copy)]
struct Spelled {
    ty:   TypeId,
    name: StringCacheId,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The object a predefined identifier designates when no declaration is
    /// visible, or `None` for any other name. Inside a function the name is
    /// installed in the function's scope, so later uses find it by lookup.
    /// C99: §6.4.2.2p1, p. 52; PDF p. 64.
    pub(super) fn predefined_identifier(&mut self, name: Identifier) -> Option<usize> {
        let which = match self.context.string_cache.at(name.name) {
            | "__func__" | "__FUNCTION__" => Predefined::Name,
            | "__PRETTY_FUNCTION__" => Predefined::Pretty,
            | _ => return None,
        };
        let current = self
            .functions
            .current
            .filter(|current| current.name.is_some());
        let Some(current) = current else {
            return Some(self.predefined_outside_function(which, name));
        };
        let binding = match which {
            | Predefined::Name => current.name?,
            | Predefined::Pretty => match current.pretty {
                | Some(binding) => binding,
                | None => {
                    let binding = self.define_pretty_function(current)?;
                    if let Some(current) = &mut self.functions.current {
                        current.pretty = Some(binding);
                    }
                    binding
                },
            },
        };
        let scope = std::mem::replace(&mut self.scope, current.scope);
        self.install(name, Namespace::Ordinary, binding);
        self.scope = scope;
        Some(binding)
    }

    /// Clang warns at every use and gives the object an empty string, or
    /// `top level` for the pretty form. The objects are made once per
    /// translation unit and are not installed, so each use warns.
    fn predefined_outside_function(&mut self, which: Predefined, name: Identifier) -> usize {
        self.group_diagnostic(
            crate::configuration::DiagnosticGroup::PredefinedOutsideFunction,
            SemanticErrorKind::PredefinedOutsideFunction,
            name.source_vectors,
            Some(name.name),
            None,
        );
        let slot = usize::from(which == Predefined::Pretty);
        if let Some(binding) = self.functions.outside[slot] {
            return binding;
        }
        let (text, kind) = if which == Predefined::Pretty {
            let text = self.context.string_cache.intern("top level");
            let kind = DefinitionKind::PrettyFunctionName {
                function: None,
                text,
            };
            (text, kind)
        } else {
            let text = self.context.string_cache.intern("");
            (text, DefinitionKind::FunctionName(text))
        };
        let binding = self.define_function_string(name, text, kind, 0);
        self.functions.outside[slot] = Some(binding);
        binding
    }

    /// `__PRETTY_FUNCTION__` of the function being analyzed.
    fn define_pretty_function(&mut self, current: FunctionContext<'tu>) -> Option<usize> {
        let function = current.syntax.declarator.identifier()?;
        let scratch = self.scratch;
        let syntax = current.syntax;
        let prototype = match function_derivation(syntax.declarator, scratch) {
            | Some(DirectDeclarator::Function {
                parameter_list,
                is_variadic,
            }) => Some((parameter_list.as_slice(), *is_variadic)),
            | _ => None,
        };
        let declaration_scope = self.scopes[current.scope].parent.unwrap_or(0);
        let result = self.typedef_sugar(
            syntax.declaration_specifiers.type_specifiers,
            declaration_scope,
        );
        let mut parameters = ArenaVec::new_in(scratch);
        for parameter in prototype.map_or(&[][..], |(list, _)| list) {
            parameters.push(self.typedef_sugar(
                parameter.declaration_specifiers.type_specifiers,
                declaration_scope,
            ));
        }
        let mut signature = ArenaString::new_in(scratch);
        self.write_signature(
            current,
            function,
            prototype.map(|(_, variadic)| variadic),
            (result, &parameters),
            scratch,
            &mut signature,
        );
        let text = self.context.string_cache.intern(signature.as_str());
        let name = Identifier {
            name:           self.context.string_cache.intern("__PRETTY_FUNCTION__"),
            source_vectors: function.source_vectors,
        };
        let kind = DefinitionKind::PrettyFunctionName {
            function: Some(function.name),
            text,
        };
        Some(self.define_function_string(name, text, kind, current.scope))
    }

    /// A static `const char` array holding `text` and its null character.
    fn define_function_string(
        &mut self,
        name: Identifier,
        text: StringCacheId,
        kind: DefinitionKind,
        scope: usize,
    ) -> usize {
        let element = self
            .types
            .scalar(Scalar::Char)
            .qualified(TypeQualifiers::CONST);
        let count = self.context.string_cache.at(text).len() as u64 + 1;
        let ty = self
            .types
            .intern(TypeKind::Array(element, ArrayBound::Constant(count)));
        let binding = self.bindings.len();
        self.bindings.push(Binding {
            origin: BindingOrigin::User,
            name,
            ty,
            scope,
            kind: BindingKind::Object,
            linkage: Linkage::None,
            duration: Duration::Static,
            value: None,
        });
        self.definitions.push(Definition { binding, kind });
        binding
    }

    /// Clang's `PredefinedExpr::PrettyFunction` text for a C function: the
    /// return type, then the name and the parameter types as declared,
    /// typedef names included. A definition without a prototype
    /// (`variadic` is `None`) prints no parameters.
    fn write_signature(
        &self,
        current: FunctionContext<'tu>,
        function: Identifier,
        variadic: Option<bool>,
        (result, parameters): (Option<Spelled>, &[Option<Spelled>]),
        scratch: &Bump,
        out: &mut ArenaString<'_>,
    ) {
        let mut proto = ArenaString::new_in(scratch);
        proto.push_str(self.context.string_cache.at(function.name));
        proto.push('(');
        if let Some(variadic) = variadic {
            let void = matches!(
                current.parameters,
                [parameter] if parameter.name.is_none()
                    && self.types.nodes[parameter.ty.index] == TypeKind::Scalar(Scalar::Void)
            );
            for (index, parameter) in current.parameters.iter().enumerate().filter(|_| !void) {
                if index != 0 {
                    proto.push_str(", ");
                }
                let sugar = parameters.get(index).copied().flatten();
                self.write_declarator(parameter.ty, "", sugar, scratch, &mut proto);
            }
            if variadic {
                proto.push_str(if current.parameters.is_empty() {
                    "..."
                } else {
                    ", ..."
                });
            } else if void || current.parameters.is_empty() {
                proto.push_str("void");
            }
        }
        proto.push(')');
        self.write_declarator(current.result, proto.as_str(), result, scratch, out);
    }

    /// A typedef-name specifier and the type it names. The built-in typedef
    /// names exist only in the type table of a program that uses them.
    fn typedef_sugar(&mut self, specifiers: TypeSpecifiers<'tu>, scope: usize) -> Option<Spelled> {
        let TypeSpecifiers::TypedefName(name) = specifiers else {
            return None;
        };
        let ty = match self.context.string_cache.at(name.name) {
            | "__builtin_va_list" => self.va_list_type?,
            | "__int128_t" => self.types.scalar(Scalar::Int128),
            | "__uint128_t" => self.types.scalar(Scalar::UnsignedInt128),
            | _ => {
                let binding = self.bindings[self
                    .lookup_at(Namespace::Ordinary, name.name, scope)?
                    .binding];
                (binding.kind == BindingKind::Typedef).then_some(binding.ty)?
            },
        };
        Some(Spelled {
            ty,
            name: name.name,
        })
    }

    /// Writes `ty` as a declaration of `placeholder`: the base type, then the
    /// declarator around the placeholder, as a C type printer does (`int
    /// (*f(void))(int)`). Derivations are read outermost first and written
    /// innermost first, with an explicit stack in place of recursion.
    /// C99: declarators §6.7.5, pp. 114-120; PDF pp. 126-132; type names
    /// §6.7.6, p. 122; PDF p. 134.
    fn write_declarator<'a>(
        &'a self,
        ty: TypeId,
        placeholder: &'a str,
        sugar: Option<Spelled>,
        scratch: &'a Bump,
        out: &mut ArenaString<'_>,
    ) {
        enum Part<'a, 'tu> {
            Text(&'a str),
            /// A nested type, or with `true` the declared type itself.
            Type(TypeId, bool),
            Function(&'tu [TypeId], bool, bool),
        }
        enum Base<'a> {
            Text(&'a str),
            Atomic(TypeId),
        }
        let c23 = self.context.configuration.standard() >= CStandard::C23;
        let mut work = ArenaVec::new_in(scratch);
        work.push(Part::Type(ty, true));
        while let Some(part) = work.pop() {
            match part {
                | Part::Text(text) => out.push_str(text),
                | Part::Function(parameters, prototype, variadic) => {
                    work.push(Part::Text(")"));
                    if variadic {
                        work.push(Part::Text(if parameters.is_empty() {
                            "..."
                        } else {
                            ", ..."
                        }));
                    }
                    if prototype && parameters.is_empty() && !variadic {
                        work.push(Part::Text("void"));
                    }
                    for (index, &parameter) in parameters.iter().enumerate().rev() {
                        work.push(Part::Type(parameter, false));
                        if index != 0 {
                            work.push(Part::Text(", "));
                        }
                    }
                    work.push(Part::Text("("));
                },
                | Part::Type(mut ty, declared) => {
                    let (placeholder, sugar) = if declared {
                        (placeholder, sugar)
                    } else {
                        ("", None)
                    };
                    let mut left = ArenaVec::new_in(scratch);
                    let mut right = ArenaVec::new_in(scratch);
                    let mut pointer = false;
                    let mut outer_qualified = false;
                    let (base, qualifiers) = loop {
                        if let Some(sugar) = sugar
                            && sugar.ty.index == ty.index
                            && ty.qualifiers.contains(sugar.ty.qualifiers)
                        {
                            break (
                                Base::Text(self.context.string_cache.at(sugar.name)),
                                ty.qualifiers.difference(sugar.ty.qualifiers),
                            );
                        }
                        match self.types.nodes[ty.index] {
                            | TypeKind::Pointer(next) => {
                                let words = qualifier_words(ty.qualifiers);
                                if !words.is_empty() {
                                    if left.is_empty() {
                                        outer_qualified = true;
                                        left.push(Part::Text(words));
                                    } else {
                                        left.push(Part::Text(format_in!(scratch, "{words} ")));
                                    }
                                }
                                left.push(Part::Text("*"));
                                pointer = true;
                                ty = next;
                            },
                            | TypeKind::Array(next, bound) => {
                                if pointer {
                                    left.push(Part::Text("("));
                                    right.push(Part::Text(")"));
                                }
                                right.push(Part::Text(match bound {
                                    | ArrayBound::Incomplete => "[]",
                                    | ArrayBound::Variable(_) | ArrayBound::Star => "[*]",
                                    | ArrayBound::Constant(count) =>
                                        format_in!(scratch, "[{count}]"),
                                }));
                                pointer = false;
                                ty = next;
                            },
                            | TypeKind::Function {
                                result,
                                parameters,
                                prototype,
                                variadic,
                            } => {
                                if pointer {
                                    left.push(Part::Text("("));
                                    right.push(Part::Text(")"));
                                }
                                right.push(Part::Function(parameters, prototype, variadic));
                                pointer = false;
                                ty = result;
                            },
                            | TypeKind::Atomic(value) =>
                                break (Base::Atomic(value), ty.qualifiers),
                            | TypeKind::Vector { element, count, .. } => {
                                let element_name = self.scalar_spelling(element, c23);
                                break (
                                    Base::Text(format_in!(
                                        scratch,
                                        "__attribute__((__vector_size__({count} * \
                                         sizeof({element_name})))) {element_name}"
                                    )),
                                    ty.qualifiers,
                                );
                            },
                            | TypeKind::Nullptr => break (Base::Text("nullptr_t"), ty.qualifiers),
                            | TypeKind::Unknown | TypeKind::Unmodeled(_) =>
                                break (Base::Text("int"), ty.qualifiers),
                            | TypeKind::Scalar(scalar) =>
                                break (Base::Text(scalar_spelling(scalar, c23)), ty.qualifiers),
                            | TypeKind::Tag(id) => {
                                let tag = self.types.tags[id];
                                let kind = match tag.kind {
                                    | TagKind::Struct => "struct",
                                    | TagKind::Union => "union",
                                    | TagKind::Enum => "enum",
                                };
                                break (
                                    Base::Text(format_in!(
                                        scratch,
                                        "{kind} {}",
                                        tag.name.map_or("(unnamed)", |name| {
                                            match self.context.string_cache.at(name) {
                                                // Clang names the SysV record
                                                // so.
                                                | "__builtin_va_list" => "__va_list_tag",
                                                | name => name,
                                            }
                                        })
                                    )),
                                    ty.qualifiers,
                                );
                            },
                        }
                    };
                    let derived = !left.is_empty() || !right.is_empty();
                    for part in right.into_iter().rev() {
                        work.push(part);
                    }
                    if !placeholder.is_empty() {
                        work.push(Part::Text(placeholder));
                        if outer_qualified {
                            work.push(Part::Text(" "));
                        }
                    }
                    for part in left {
                        work.push(part);
                    }
                    if derived || !placeholder.is_empty() {
                        work.push(Part::Text(" "));
                    }
                    match base {
                        | Base::Text(text) => work.push(Part::Text(text)),
                        | Base::Atomic(value) => {
                            work.push(Part::Text(")"));
                            work.push(Part::Type(value, false));
                            work.push(Part::Text("_Atomic("));
                        },
                    }
                    let words = qualifier_words(qualifiers);
                    if !words.is_empty() {
                        work.push(Part::Text(" "));
                        work.push(Part::Text(words));
                    }
                },
            }
        }
    }

    /// The spelling of an element type of a vector.
    fn scalar_spelling(&self, ty: TypeId, c23: bool) -> &'static str {
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(scalar) => scalar_spelling(scalar, c23),
            | _ => "int",
        }
    }
}

/// Clang's spelling of a scalar type: complex types lead with `_Complex`,
/// and `bool` is the C23 spelling of `_Bool`.
fn scalar_spelling(scalar: Scalar, c23: bool) -> &'static str {
    match scalar {
        | Scalar::Bool if c23 => "bool",
        | Scalar::ComplexFloat => "_Complex float",
        | Scalar::ComplexDouble => "_Complex double",
        | Scalar::ComplexLongDouble => "_Complex long double",
        | Scalar::ComplexFloat128 => "_Complex __float128",
        | scalar => scalar.spelling(),
    }
}

/// The qualifiers a signature prints. `restrict` exists only from C99, and
/// the others print in Clang's order.
const fn qualifier_words(qualifiers: TypeQualifiers) -> &'static str {
    match qualifiers.bits() & 7 {
        | 0 => "",
        | 1 => "const",
        | 2 => "volatile",
        | 3 => "const volatile",
        | 4 => "restrict",
        | 5 => "const restrict",
        | 6 => "volatile restrict",
        | _ => "const volatile restrict",
    }
}
