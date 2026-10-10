//! Phase-7 declaration typing, nominal tags, bindings and aggregate completion.
//! C99: §6.2.1-§6.2.4, pp. 29-32; PDF pp. 41-44; §6.7-§6.7.7,
//! pp. 97-124; PDF pp. 109-136. Initializer constraints use initializers.rs;
//! function-definition constraints use functions.rs.

use super::{
    Analyzer,
    ArenaVec,
    ArrayBound,
    Binding,
    BindingKind,
    CStandard,
    Cell,
    Collection,
    DeclarationSpecifiers,
    Declarator,
    DirectDeclarator,
    Duration,
    Identifier,
    Integer,
    Layout,
    Linkage,
    Member,
    Namespace,
    Parameter,
    Scalar,
    ScopeKind,
    SemanticErrorKind,
    SourceVectors,
    StorageClass,
    StructOrUnion,
    Tag,
    TagKind,
    TypeId,
    TypeKind,
    TypeQualifiers,
    TypeSpecifiers,
    Work,
    align_up,
    types::{
        Field,
        FieldPath,
    },
};

/// The x86-64 System V target ignores MSVC calling conventions.
/// MSVC extension: Microsoft Learn, "Argument Passing and Naming Conventions".
/// <https://learn.microsoft.com/en-us/cpp/cpp/argument-passing-and-naming-conventions>
fn calling_convention(keyword: super::super::preprocessing::KeywordTokenType) -> bool {
    use super::super::preprocessing::KeywordTokenType as K;
    matches!(
        keyword,
        K::Cdecl | K::Stdcall | K::Fastcall | K::Vectorcall | K::Thiscall
    )
}

/// The standard integer type an MSVC sized-integer keyword names, as in Clang
/// and MSVC: `__int8` is plain `char`, and `__int16`, `__int32` and `__int64`
/// are `short`, `int` and `long long`, so `__int32` is never `long`. An
/// explicit sign selects the signed or unsigned type.
///
/// MSVC extension; the integer types are C99: §6.7.2p2, pp. 99-100;
/// PDF pp. 111-112.
/// MSVC extension: Microsoft Learn, "__int8, __int16, __int32, __int64".
/// <https://learn.microsoft.com/en-us/cpp/cpp/int8-int16-int32-int64>
fn ms_integer(width: u8, signedness: Option<bool>) -> Scalar {
    match (width, signedness) {
        | (8, None) => Scalar::Char,
        | (8, Some(true)) => Scalar::SignedChar,
        | (8, Some(false)) => Scalar::UnsignedChar,
        | (16, Some(false)) => Scalar::UnsignedShort,
        | (16, _) => Scalar::Short,
        | (32, Some(false)) => Scalar::UnsignedInt,
        | (32, _) => Scalar::Int,
        | (64, Some(false)) => Scalar::UnsignedLongLong,
        | (64, _) => Scalar::LongLong,
        | _ => unreachable!("MSVC sized integers are 8, 16, 32 or 64 bits wide"),
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Resolves the parser's validated specifier multiset into a canonical
    /// type. C99: §6.7.2p2-5, pp. 99-100; PDF pp. 111-112.
    pub(super) fn resolve_spec(
        &mut self,
        spec: TypeSpecifiers<'tu>,
        q: TypeQualifiers,
        source: SourceVectors,
        force: bool,
    ) {
        use TypeSpecifiers as S;
        let scalar = match spec {
            | S::Void => Some(Scalar::Void),
            | S::Bool => Some(Scalar::Bool),
            | S::Char => Some(Scalar::Char),
            | S::SignedChar => Some(Scalar::SignedChar),
            | S::UnsignedChar => Some(Scalar::UnsignedChar),
            | S::Short | S::SignedShort | S::ShortInt | S::SignedShortInt => Some(Scalar::Short),
            | S::UnsignedShort | S::UnsignedShortInt => Some(Scalar::UnsignedShort),
            | S::Empty | S::Int | S::Signed | S::SignedInt => Some(Scalar::Int),
            | S::Unsigned | S::UnsignedInt => Some(Scalar::UnsignedInt),
            | S::Long | S::SignedLong | S::LongInt | S::SignedLongInt => Some(Scalar::Long),
            | S::UnsignedLong | S::UnsignedLongInt => Some(Scalar::UnsignedLong),
            | S::LongLong | S::SignedLongLong | S::LongLongInt | S::SignedLongLongInt =>
                Some(Scalar::LongLong),
            | S::UnsignedLongLong | S::UnsignedLongLongInt => Some(Scalar::UnsignedLongLong),
            | S::Extended(super::ExtendedType::Int128 { signedness }) =>
                Some(if signedness == &Some(false) {
                    Scalar::UnsignedInt128
                } else {
                    Scalar::Int128
                }),
            | S::Extended(super::ExtendedType::Float128 { complex }) => {
                if self.context.configuration.target() == crate::target::Target::WindowsMsvc {
                    self.error(SemanticErrorKind::UnsupportedFloat128, source, None, None);
                    None
                } else {
                    Some(if *complex {
                        Scalar::ComplexFloat128
                    } else {
                        Scalar::Float128
                    })
                }
            },
            | S::Float => Some(Scalar::Float),
            | S::Double => Some(Scalar::Double),
            | S::LongDouble => Some(Scalar::LongDouble),
            | S::ComplexFloat => Some(Scalar::ComplexFloat),
            | S::ComplexDouble => Some(Scalar::ComplexDouble),
            | S::ComplexLongDouble => Some(Scalar::ComplexLongDouble),
            | S::Extended(super::ExtendedType::MsInteger { width, signedness }) =>
                Some(ms_integer(*width, *signedness)),
            | _ => None,
        };
        self.work.push(Work::Qualify(q, source));
        if let S::Extended(super::ExtendedType::Typeof {
            operand,
            unqualified,
        }) = spec
        {
            self.work.push(Work::TypeofDone(*operand, *unqualified));
            self.syntax_operand(*operand);
            return;
        }
        if let Some(s) = scalar {
            let ty = self.types.scalar(s);
            self.values.push(ty);
            return;
        }
        match spec {
            | S::TypedefName(name) => {
                if self.context.string_cache.at(name.name) == "__builtin_va_list" {
                    let ty = self.builtin_va_list();
                    self.values.push(ty);
                    return;
                }
                let builtin = match self.context.string_cache.at(name.name) {
                    | "__int128_t" => Some(Scalar::Int128),
                    | "__uint128_t" => Some(Scalar::UnsignedInt128),
                    | _ => None,
                };
                if let Some(scalar) = builtin {
                    let ty = self.types.scalar(scalar);
                    self.values.push(ty);
                    return;
                }
                let ty = self
                    .lookup(Namespace::Ordinary, name.name)
                    .and_then(|entry| {
                        let binding = self.bindings[entry.binding];
                        (binding.kind == BindingKind::Typedef).then_some(binding.ty)
                    });
                if ty.is_none() {
                    self.error(
                        SemanticErrorKind::UnknownTypedef,
                        name.source_vectors,
                        Some(name.name),
                        None,
                    );
                }
                self.values.push(ty.unwrap_or_else(|| self.types.unknown()));
            },
            | S::StructOrUnion(s) => {
                let kind = if s.struct_or_union == StructOrUnion::Struct {
                    TagKind::Struct
                } else {
                    TagKind::Union
                };
                let (tag, define) = self.tag(
                    kind,
                    s.identifier,
                    s.struct_declaration_list.is_some(),
                    force,
                    false,
                    s.source_vectors,
                );
                let ty = self.types.intern(TypeKind::Tag(tag));
                self.values.push(ty);
                if define && self.unmodeled_extension(s.attributes) {
                    self.types.tags[tag].tainted.set(true);
                }
                if define && let Some(list) = s.struct_declaration_list {
                    _ = self.defining.insert(tag, ());
                    let members = self.scratch.alloc(Collection::new());
                    self.work.push(Work::RecordMembers(tag, list, 0, members));
                }
            },
            | S::Enum(s) => {
                let (tag, define) = self.tag(
                    TagKind::Enum,
                    s.name,
                    s.enumeration_list.is_some(),
                    false,
                    s.underlying_type.is_some(),
                    s.source_vectors,
                );
                let ty = self.types.intern(TypeKind::Tag(tag));
                self.values.push(ty);
                if define && let Some(list) = s.enumeration_list {
                    _ = self.defining.insert(tag, ());
                    self.work.push(Work::EnumMembers(tag, list, 0, None));
                }
                if (define && self.unmodeled_extension(s.attributes)) || s.underlying_type.is_some()
                {
                    self.types.tags[tag].tainted.set(true);
                }
                if let Some(name) = s.underlying_type {
                    self.syntax_operand(super::SyntaxOperand::Type(name));
                }
            },
            | S::Extended(extended) => {
                if let super::ExtendedType::Atomic(name) = *extended {
                    self.work.push(Work::Atomic(source));
                    self.work.push(Work::TypeName(name));
                    return;
                }
                self.values.push(self.types.unknown());
                match *extended {
                    | super::ExtendedType::Atomic(name) =>
                        self.syntax_operand(super::SyntaxOperand::Type(name)),
                    | super::ExtendedType::Typeof { operand, .. } => self.syntax_operand(operand),
                    | super::ExtendedType::BitInt { width, .. } =>
                        self.work.push(Work::Expression(width)),
                    | _ => {},
                }
            },
            | _ => self.values.push(self.types.unknown()),
        }
    }

    /// Specifier extensions whose meaning this phase does not model:
    /// alignment, thread or constexpr storage, MSVC pointer modifiers and
    /// layout attributes. Other attributes, `__extension__` and x86-64
    /// calling conventions leave the declared type unchanged.
    /// C99: §4 paragraph 6, p. 7; PDF p. 19.
    pub(super) fn unmodeled_extension(
        &self,
        mut chain: Option<&'tu super::SpecifierExtension<'tu>>,
    ) -> bool {
        use super::SpecifierExtensionKind as K;
        while let Some(item) = chain {
            if match item.kind {
                | K::ExtensionMarker => false,
                | K::Attributes(attribute) => self.layout_attribute(attribute),
                | K::MsModifier(keyword) => !calling_convention(keyword),
                | K::Alignment(_) | K::ThreadLocal | K::Constexpr => true,
            } {
                return true;
            }
            chain = item.next;
        }
        false
    }

    /// GNU, standard-syntax vendor and MSVC attributes that change size,
    /// alignment or representation. Their arguments are not interpreted, so
    /// any such name, or an attribute that failed to parse, is conservative.
    /// GNU extension: GCC manual, "Attribute Syntax".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
    /// MSVC extension: Microsoft Learn, "declspec".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/declspec>
    pub(super) fn layout_attribute(&self, attribute: &super::AttributeSpecifier<'tu>) -> bool {
        attribute.recovered
            || attribute.tokens.iter().any(|token| {
                let name = self.context.string_cache.at(token.contents);
                let name = name
                    .strip_prefix("__")
                    .and_then(|n| n.strip_suffix("__"))
                    .unwrap_or(name);
                matches!(name, "align" | "packed" | "mode" | "ext_vector_type")
            })
    }

    /// Discover core type names inside opaque later-standard operands.
    /// C99: §6.7.6, p. 122; PDF p. 134; extensions remain unanalyzed.
    pub(super) fn syntax_operand(&mut self, operand: super::SyntaxOperand<'tu>) {
        match operand {
            | super::SyntaxOperand::Expression(expression) =>
                self.work.push(Work::Expression(expression)),
            | super::SyntaxOperand::Type(name) => {
                self.work.push(Work::DiscardType);
                self.work.push(Work::TypeName(name));
            },
        }
    }

    /// A tag-only declaration shadows outer tags; a reference reuses visible
    /// tags. C99: §6.7.2.3p1-9, pp. 106-107; PDF pp. 118-119.
    fn tag(
        &mut self,
        kind: TagKind,
        name: Option<Identifier>,
        body: bool,
        force: bool,
        allow_incomplete: bool,
        source: SourceVectors,
    ) -> (usize, bool) {
        let visible = name.and_then(|n| self.lookup(Namespace::Tag, n.name));
        if let Some(entry) = visible
            && ((!body && !force) || entry.scope == self.scope)
        {
            let tag = self.types.tags[entry.binding];
            if body || force {
                self.tag_declarations.push((entry.binding, self.scope));
            }
            if tag.kind != kind {
                self.error(
                    SemanticErrorKind::TagKindMismatch,
                    source,
                    name.map(|n| n.name),
                    Some(entry.name.source_vectors),
                );
                return if body {
                    (self.rejected_tag(kind, name), true)
                } else {
                    (entry.binding, false)
                };
            }
            // A nested list redefines a tag whose own list is still open: its
            // type is incomplete until the closing brace (§6.7.2.3p1 and p4).
            if body && (tag.complete.get() || self.defining.contains_key(&entry.binding)) {
                self.error(
                    SemanticErrorKind::TagRedefinition,
                    source,
                    name.map(|n| n.name),
                    Some(entry.name.source_vectors),
                );
                return (self.rejected_tag(kind, name), true);
            }
            return (entry.binding, body);
        }
        if kind == TagKind::Enum
            && !body
            && !allow_incomplete
            && !self.context.configuration.gnu_extensions()
        {
            self.error(
                SemanticErrorKind::IncompleteEnum,
                source,
                name.map(|n| n.name),
                None,
            );
        }
        let index = self.new_tag(kind, name);
        self.tag_declarations.push((index, self.scope));
        if let Some(name) = name {
            self.install(name, Namespace::Tag, index);
        }
        (index, body)
    }

    /// Creates a distinct incomplete structure, union or enumeration type.
    /// C99: §6.7.2.3 paragraphs 4-5, p. 106; PDF p. 118.
    fn new_tag(&mut self, kind: TagKind, name: Option<Identifier>) -> usize {
        let index = self.types.tags.len();
        let tag = self.types.tu.alloc(Tag {
            name: name.map(|n| n.name),
            kind,
            members: Cell::new(&[]),
            fields: Cell::new(&[]),
            layout: Cell::new(None),
            complete: Cell::new(false),
            tainted: Cell::new(false),
            contains_flexible: Cell::new(false),
            compatible: Cell::new(Scalar::Int),
        });
        self.types.tags.push(tag);
        index
    }

    /// A rejected definition still declares the tags, enumerators and
    /// members inside its list, so later uses of them do not cascade. Its
    /// uninstalled tag is unanalyzed: neither the rejected nor the original
    /// contents is a trustworthy type for the declaration.
    #[cold]
    #[inline(never)]
    fn rejected_tag(&mut self, kind: TagKind, name: Option<Identifier>) -> usize {
        let index = self.new_tag(kind, name);
        self.types.tags[index].tainted.set(true);
        index
    }

    /// Enforces the implementation's PTRDIFF_MAX-byte object-size limit for
    /// declared bounds and initializer-completed arrays.
    /// C99: §5.2.4.1p1, pp. 20-21; PDF pp. 32-33 (implementation limit);
    /// §6.5.6p9, pp. 83-84; PDF pp. 95-96 (representable pointer differences).
    pub(super) fn validate_array_size(
        &mut self,
        element: TypeId,
        count: i128,
        source: SourceVectors,
    ) -> bool {
        if self.types.layout(element).is_some_and(|layout| {
            i128::from(layout.size)
                .checked_mul(count)
                .is_none_or(|size| size > i128::from(i64::MAX))
        }) {
            self.error(SemanticErrorKind::ObjectTooLarge, source, None, None);
            false
        } else {
            true
        }
    }

    /// Checks `restrict` after typedef expansion as well as after pointer
    /// derivation. C99: §6.7.3p2, p. 108; PDF p. 120.
    pub(super) fn validate_qualifiers(&mut self, ty: TypeId, source: SourceVectors) {
        if !ty.qualifiers.contains(TypeQualifiers::RESTRICT) {
            return;
        }
        let valid = match self.types.nodes[ty.index] {
            | TypeKind::Unknown => true,
            | TypeKind::Pointer(target) =>
                !matches!(self.types.nodes[target.index], TypeKind::Function { .. }),
            | _ => false,
        };
        if !valid {
            self.error(SemanticErrorKind::InvalidRestrict, source, None, None);
        }
    }

    /// Declarator suffixes derive from the base inward, then grouped children.
    /// C99: §6.7.5p4-6, p. 115; PDF p. 127.
    pub(super) fn direct(
        &mut self,
        direct: &'tu DirectDeclarator<'tu>,
        source: SourceVectors,
        parameter: bool,
    ) {
        let base = self.take_type();
        match *direct {
            | DirectDeclarator::Parenthesized(p) =>
                self.work
                    .push(Work::Declarator(p.declarator, base, parameter)),
            | DirectDeclarator::Array {
                type_qualifiers,
                is_static,
                is_pointer,
                assignment_expression,
            } => {
                if !parameter && (!type_qualifiers.is_empty() || is_static) {
                    self.error(SemanticErrorKind::InvalidParameter, source, None, None);
                } else if !parameter && is_pointer {
                    self.error(SemanticErrorKind::InvalidStarBound, source, None, None);
                }
                if matches!(
                    self.types.nodes[base.index],
                    TypeKind::Scalar(Scalar::Void) | TypeKind::Function { .. }
                ) || matches!(self.types.nodes[base.index],TypeKind::Tag(id) if !self.types.tags[id].complete.get() && !self.types.tags[id].tainted.get())
                    || matches!(
                        self.types.nodes[base.index],
                        TypeKind::Array(_, ArrayBound::Incomplete)
                    )
                {
                    self.error(SemanticErrorKind::InvalidDerivedType, source, None, None);
                }
                // §6.7.2.1p2 forbids such elements; GCC accepts them.
                if let TypeKind::Tag(id) = self.types.nodes[base.index]
                    && self.types.tags[id].contains_flexible.get()
                    && !self.types.tags[id].tainted.get()
                {
                    self.flexible_extension(
                        "array of structures with a flexible array member",
                        source,
                    );
                }
                if let Some(expression) = assignment_expression {
                    self.work.push(Work::ArrayDone(
                        base,
                        type_qualifiers,
                        expression,
                        source,
                        self.semantic_errors,
                    ));
                    // A bound whose evaluation is exceptional is not an
                    // integer constant expression, so it is a variable length
                    // (§6.7.5.2p4); scopes that forbid one report it then.
                    self.work.push(Work::RuntimeBound(self.runtime_bound));
                    self.work.push(Work::Eval(expression));
                    self.work.push(Work::RuntimeBound(true));
                    self.work.push(Work::Expression(expression));
                } else if is_pointer && !parameter {
                    // Already diagnosed; no dependent variably modified error.
                    self.values.push(self.types.unknown());
                } else {
                    let bound = if is_pointer {
                        ArrayBound::Star
                    } else {
                        ArrayBound::Incomplete
                    };
                    self.values.push(
                        self.types
                            .intern(TypeKind::Array(base, bound))
                            .qualified(type_qualifiers),
                    );
                }
            },
            | DirectDeclarator::Function {
                parameter_list,
                is_variadic,
            } => {
                if matches!(
                    self.types.nodes[base.index],
                    TypeKind::Array(..) | TypeKind::Function { .. }
                ) {
                    self.error(SemanticErrorKind::InvalidDerivedType, source, None, None);
                }
                self.enter(ScopeKind::Prototype);
                let parameters = self.scratch.alloc(Collection::new());
                self.work.push(Work::FunctionParameters(
                    parameter_list,
                    0,
                    parameters,
                    base,
                    is_variadic,
                    source,
                    std::ptr::from_ref(direct).addr(),
                ));
            },
            | DirectDeclarator::KAndRStyleFunction { parameters } => {
                let ty = self.types.intern(TypeKind::Function {
                    result:     base,
                    parameters: &[],
                    prototype:  false,
                    variadic:   false,
                });
                self.values.push(ty);
                let int = self.types.scalar(Scalar::Int);
                let mut list = ArenaVec::new_in(self.types.tu);
                for &name in parameters {
                    list.push(Parameter {
                        name:          Some(name),
                        ty:            int,
                        array_minimum: None,
                    });
                }
                _ = self
                    .parameters
                    .insert(std::ptr::from_ref(direct).addr(), (list.leak(), 0));
            },
            | DirectDeclarator::Attributes(attribute) => {
                let ty = if self.layout_attribute(attribute) {
                    self.types.unknown()
                } else {
                    self.vector_attribute(base, attribute)
                };
                self.values.push(ty);
            },
            | DirectDeclarator::MsModifier(keyword, _) =>
                self.values.push(if calling_convention(keyword) {
                    base
                } else {
                    self.types.unknown()
                }),
            | _ => self.values.push(base),
        }
    }

    /// Parameter array qualifiers/static belong only to the outermost array
    /// derivation. Walk constructors in type-construction order, including
    /// parenthesized pointer derivations, without native recursion.
    /// C99: §6.7.5.2p1, p. 116; PDF p. 128.
    pub(super) fn validate_parameter_arrays(&mut self, declarator: Declarator<'tu>) {
        enum Step<'tu> {
            Declarator(Declarator<'tu>),
            Direct(DirectDeclarator<'tu>, SourceVectors),
        }
        let mut work = ArenaVec::new_in(self.scratch);
        let mut constrained = None;
        work.push(Step::Declarator(declarator));
        while let Some(step) = work.pop() {
            let (derives, next) = match step {
                | Step::Declarator(d) => {
                    for &direct in d.kind {
                        work.push(Step::Direct(direct, d.source_vectors));
                    }
                    (!d.pointer.levels.is_empty(), None)
                },
                | Step::Direct(DirectDeclarator::Parenthesized(p), _) => {
                    work.push(Step::Declarator(p.declarator));
                    (false, None)
                },
                | Step::Direct(
                    DirectDeclarator::Array {
                        type_qualifiers,
                        is_static,
                        ..
                    },
                    source,
                ) => (
                    true,
                    (!type_qualifiers.is_empty() || is_static).then_some(source),
                ),
                | Step::Direct(
                    DirectDeclarator::Function { .. } | DirectDeclarator::KAndRStyleFunction { .. },
                    _,
                ) => (true, None),
                | _ => (false, None),
            };
            if derives {
                if let Some(source) = constrained.take() {
                    self.error(SemanticErrorKind::InvalidParameter, source, None, None);
                }
                constrained = next;
            }
        }
    }

    /// Resolves storage class, linkage inheritance and duration independently.
    /// C99: §6.2.2p2-7, pp. 30-31; PDF pp. 42-43; §6.2.4, p. 32; PDF p. 44;
    /// §6.7.1, p. 98; PDF p. 110; §6.7.4p2-4, p. 112; PDF p. 124.
    pub(super) fn bind_declaration(
        &mut self,
        declarator: Declarator<'tu>,
        spec: DeclarationSpecifiers<'tu>,
        ty: TypeId,
        initialized: bool,
    ) {
        let Some(name) = declarator.identifier() else {
            return;
        };
        if self.old_parameter_mode {
            let ty = if self.validate_old_parameter(name, spec, ty, initialized) {
                ty
            } else {
                self.types.unknown()
            };
            let ty = match self.types.nodes[ty.index] {
                | TypeKind::Array(element, _) => self.types.intern(TypeKind::Pointer(element)),
                | TypeKind::Function { .. } => self.types.intern(TypeKind::Pointer(ty)),
                | _ => ty,
            };
            if spec
                .storage_class
                .is_some_and(|s| s != StorageClass::Register)
            {
                self.error(
                    SemanticErrorKind::InvalidParameter,
                    name.source_vectors,
                    Some(name.name),
                    None,
                );
            }
            self.bind(
                name,
                ty,
                BindingKind::Parameter,
                Linkage::None,
                Duration::Automatic,
                None,
            );
            self.record_vm(self.bindings.len() - 1);
            // K&R parameters take their storage from the declaration list.
            // C99 §6.5.3.2p1, p. 78; PDF p. 90.
            if spec.storage_class == Some(StorageClass::Register) {
                _ = self.register_bindings.insert(self.bindings.len() - 1, true);
            }
            return;
        }
        let file = self.scopes[self.scope].kind == ScopeKind::File;
        let typedef = spec.storage_class == Some(StorageClass::Typedef);
        let prior = self
            .lookup(Namespace::Ordinary, name.name)
            .map(|e| self.bindings[e.binding]);
        // An unanalyzed type, such as GNU `__typeof__` of a function, cannot
        // show whether it declares an object or a function, so it redeclares
        // the visible function rather than conflicting with its kind.
        let function = matches!(self.types.nodes[ty.index], TypeKind::Function { .. })
            || (!typedef
                && self.types.nodes[ty.index] == TypeKind::Unknown
                && prior.is_some_and(|p| p.kind == BindingKind::Function));
        if file
            && matches!(
                spec.storage_class,
                Some(StorageClass::Auto | StorageClass::Register)
            )
            && !(spec.storage_class == Some(StorageClass::Auto)
                && self.context.configuration.standard() >= CStandard::C23
                && self.types.unanalyzed(ty))
        {
            self.error(
                SemanticErrorKind::InvalidStorage,
                name.source_vectors,
                Some(name.name),
                None,
            );
        } else if !file
            && function
            && !typedef
            && matches!(
                spec.storage_class,
                Some(StorageClass::Auto | StorageClass::Register | StorageClass::Static)
            )
        {
            self.error(
                SemanticErrorKind::InvalidFunctionStorage,
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
        if spec.function_specifiers.is_inline
            && ((!function && !self.types.unanalyzed(ty))
                || typedef
                || self.context.string_cache.at(name.name) == "main")
        {
            self.error(
                SemanticErrorKind::InvalidInline,
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
        let kind = if typedef {
            BindingKind::Typedef
        } else if function {
            BindingKind::Function
        } else {
            BindingKind::Object
        };
        let linkage = if typedef {
            Linkage::None
        } else if file && spec.storage_class == Some(StorageClass::Static) {
            Linkage::Internal
        } else if spec.storage_class == Some(StorageClass::Extern)
            || (function && spec.storage_class.is_none())
        {
            prior
                .filter(|p| p.linkage != Linkage::None)
                .map_or(Linkage::External, |p| p.linkage)
        } else if file {
            Linkage::External
        } else {
            Linkage::None
        };
        let duration = if typedef || function {
            Duration::None
        } else if file
            || linkage != Linkage::None
            || spec.storage_class == Some(StorageClass::Static)
        {
            Duration::Static
        } else {
            Duration::Automatic
        };
        if kind == BindingKind::Object
            && !self.types.unanalyzed(ty)
            && ((matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void))
                && spec.storage_class != Some(StorageClass::Extern))
                || (linkage == Linkage::None
                    && !initialized
                    && (matches!(
                        self.types.nodes[ty.index],
                        TypeKind::Array(_, ArrayBound::Incomplete)
                    ) || matches!(self.types.nodes[ty.index],TypeKind::Tag(id) if !self.types.tags[id].complete.get()))))
        {
            self.error(
                SemanticErrorKind::IncompleteObject,
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
        if self.variably_modified(ty)
            && (file
                || linkage != Linkage::None
                || (duration == Duration::Static
                    && matches!(self.types.nodes[ty.index], TypeKind::Array(..))))
        {
            self.error(
                SemanticErrorKind::FileScopeVariableType,
                declarator.source_vectors,
                Some(name.name),
                None,
            );
        }
        self.bind(name, ty, kind, linkage, duration, None);
        self.record_declaration(self.bindings.len() - 1, spec, declarator, initialized);
        if spec.storage_class == Some(StorageClass::Register) {
            _ = self.register_bindings.insert(self.bindings.len() - 1, true);
        }
    }

    /// Merges visible compatible linked declarations, checks hidden ones for
    /// compatibility, and rejects same-scope no-linkage repeats. Incompatible
    /// occurrences do not replace accepted lookup bindings.
    /// C99: §6.7p3-4, p. 97; PDF p. 109; §6.2.7p2-4, p. 40; PDF p. 52. Repeated
    /// typedef exception: C11 §6.7p3, p. 108;
    /// PDF p. 126.
    pub(super) fn bind(
        &mut self,
        name: Identifier,
        mut ty: TypeId,
        kind: BindingKind,
        linkage: Linkage,
        duration: Duration,
        value: Option<Integer>,
    ) {
        let visible = self.lookup(Namespace::Ordinary, name.name);
        let previous = visible
            .filter(|p| p.scope == self.scope)
            .map(|p| p.binding)
            .or_else(|| {
                (linkage != Linkage::None)
                    .then(|| self.external.get(&name.name).copied())
                    .flatten()
            });
        // C99 §6.2.7p4: only a visible linked declaration supplies a
        // composite type; hidden linked declarations still check compatibility.
        let composite_prior = visible
            .filter(|p| {
                linkage != Linkage::None && self.bindings[p.binding].linkage != Linkage::None
            })
            .map(|p| p.binding);
        let mut compatible = true;
        for index in [
            previous,
            composite_prior.filter(|&index| Some(index) != previous),
        ]
        .into_iter()
        .flatten()
        {
            let old = self.bindings[index];
            let gnu_redeclaration = kind == BindingKind::Function
                && old.kind == kind
                && old.linkage == Linkage::External
                && linkage == Linkage::Internal
                && self.gnu_function_redefinable(name.name);
            if old.linkage != linkage
                && old.linkage != Linkage::None
                && linkage != Linkage::None
                && !gnu_redeclaration
            {
                self.error(
                    SemanticErrorKind::ConflictingLinkage,
                    name.source_vectors,
                    Some(name.name),
                    Some(old.name.source_vectors),
                );
            } else if linkage == Linkage::None || old.linkage == Linkage::None {
                // C11 §6.7p3 lets a typedef name be redefined to denote the
                // same type, unless it is variably modified; GCC and Clang
                // accept it earlier as a C11 extension, GNU modes included.
                let same_typedef = kind == BindingKind::Typedef
                    && old.kind == kind
                    && old.ty == ty
                    && !self.variably_modified(ty);
                if same_typedef {
                    if !self.tainted {
                        self.context.report_extension(
                            crate::configuration::Feature::TypedefRedefinition,
                            "typedef redefinition",
                            name.source_vectors,
                        );
                    }
                } else {
                    self.error(
                        SemanticErrorKind::DuplicateDeclaration,
                        name.source_vectors,
                        Some(name.name),
                        Some(old.name.source_vectors),
                    );
                }
            } else if old.kind != kind {
                self.error(
                    SemanticErrorKind::IncompatibleDeclaration,
                    name.source_vectors,
                    Some(name.name),
                    Some(old.name.source_vectors),
                );
                compatible = false;
                break;
            } else if let Some(composite) = self.types.composite(old.ty, ty) {
                if Some(index) == composite_prior {
                    ty = composite;
                }
            } else {
                self.error(
                    SemanticErrorKind::IncompatibleDeclaration,
                    name.source_vectors,
                    Some(name.name),
                    Some(old.name.source_vectors),
                );
                compatible = false;
                break;
            }
        }
        let index = self.bindings.len();
        self.bindings.push(Binding {
            name,
            ty,
            scope: self.scope,
            kind,
            linkage,
            duration,
            value,
        });
        // Retain the rejected occurrence without changing ordinary or linkage
        // lookup.
        if !compatible {
            return;
        }
        self.install(name, Namespace::Ordinary, index);
        if linkage != Linkage::None {
            _ = self.external.insert(name.name, index);
        }
    }

    /// C99: §6.7.5p3, pp. 114-115; PDF pp. 126-127; §6.7.5.2p2, p. 116; PDF p.
    /// 128.
    pub(super) fn variably_modified(&self, ty: TypeId) -> bool {
        self.types.variably_modified(ty)
    }

    /// Rejects non-integer bounds using the already-analyzed expression type,
    /// suppressing dependent constraints on unanalyzed types.
    /// C99: §6.7.5.2p1, p. 116; PDF p. 128.
    pub(super) fn non_integer_bound(&self, expression: &'tu super::Expression<'tu>) -> bool {
        let ty = self.expression_info(expression).ty;
        !self.types.unanalyzed(ty) && self.integer_type(ty).is_none()
    }

    /// An untagged structure or union specifier with no declarator, or with
    /// MSVC anonymous structures a tagged or typedef one, declares an
    /// anonymous member whose names join this record's namespace.
    /// C11: §6.7.2.1p13, p. 115; PDF p. 133; C99: §6.7p3, p. 97; PDF p. 109.
    pub(super) fn anonymous_member(
        &mut self,
        tag: usize,
        member: super::StructDeclaration<'tu>,
        base: TypeId,
    ) -> Option<Member> {
        let anonymous = match member.type_specifiers {
            | TypeSpecifiers::StructOrUnion(s) if s.identifier.is_none() =>
                s.struct_declaration_list.is_some(),
            | TypeSpecifiers::StructOrUnion(_) | TypeSpecifiers::TypedefName(_) => self
                .context
                .configuration
                .accepts(crate::configuration::Feature::MsAnonymousStructs),
            | _ => false,
        };
        let TypeKind::Tag(inner) = self.types.nodes[base.index] else {
            return None;
        };
        if !anonymous || self.types.tags[inner].kind == TagKind::Enum {
            return None;
        }
        for field in self.types.tags[inner].fields.get() {
            if let Some(previous) = self
                .member_names
                .insert((tag, field.name.name), field.name.source_vectors)
            {
                self.error(
                    SemanticErrorKind::DuplicateMember,
                    field.name.source_vectors,
                    Some(field.name.name),
                    Some(previous),
                );
            }
        }
        Some(Member {
            name:           None,
            source_vectors: member.source_vectors,
            invalid:        false,
            ty:             base,
            width:          None,
            offset:         0,
            bit_offset:     0,
            anonymous:      true,
        })
    }

    /// Reports a GNU flexible-array form through the extension policy.
    /// C99: §4p6, p. 7; PDF p. 19.
    pub(super) fn flexible_extension(&mut self, spelling: &str, source: SourceVectors) {
        if !self.tainted {
            self.context.report_extension(
                crate::configuration::Feature::FlexibleArrayExtensions,
                spelling,
                source,
            );
        }
    }

    /// System V packs non-straddling fields at the bit cursor. Microsoft
    /// reserves entire units, sharing only consecutive fields of equal size;
    /// a zero-width field affects layout only after a nonzero bit-field.
    /// C99: implementation-defined bit-field
    /// allocation §6.7.2.1p10-16, pp. 102-103; PDF pp. 114-115.
    pub(super) fn complete_record(&mut self, id: usize, members: &'tu [Member]) {
        let tag = self.types.tags[id];
        let mut output = ArenaVec::new_in(self.types.tu);
        let (mut bytes, mut alignment, mut bit_end) = (0_u64, 1_u64, 0_u64);
        let ms = self.types.target.ms_bitfields;
        let mut ms_unit: Option<(u64, u64, u64)> = None; // size, byte start, used bits
        let named = members.iter().filter(|m| m.initializable()).count();
        let mut too_large = false;
        for (index, &member) in members.iter().enumerate() {
            let source = member
                .name
                .map_or(member.source_vectors, |n| n.source_vectors);
            // C99 §6.7.2.1p2-4: a rejected declarator has already been
            // diagnosed; completion preserves it without dependent errors.
            if member.invalid {
                tag.tainted.set(true);
                output.push(member);
                continue;
            }
            // C99 permits a flexible array only as the last member of a
            // structure with another named member (§6.7.2.1p16). GCC also
            // accepts one in a union or as a structure's only member.
            let flexible = matches!(
                self.types.nodes[member.ty.index],
                TypeKind::Array(_, ArrayBound::Incomplete)
            ) && member.name.is_some()
                && (tag.kind == TagKind::Union || index + 1 == members.len());
            if flexible {
                tag.contains_flexible.set(true);
                let extension = if tag.kind == TagKind::Union {
                    Some("flexible array member in a union")
                } else if named < 2 {
                    Some("flexible array member in an otherwise empty structure")
                } else {
                    None
                };
                if let Some(spelling) = extension {
                    self.flexible_extension(spelling, source);
                }
            }
            // Nor may such a structure, or a union containing one, be a
            // structure member (§6.7.2.1p2); GCC accepts that too.
            if let TypeKind::Tag(nested) = self.types.nodes[member.ty.index]
                && self.types.tags[nested].contains_flexible.get()
            {
                if tag.kind == TagKind::Struct {
                    self.flexible_extension(
                        "structure with a flexible array member nested in a structure",
                        source,
                    );
                } else {
                    tag.contains_flexible.set(true);
                }
            }
            let layout = if flexible {
                if let TypeKind::Array(element, _) = self.types.nodes[member.ty.index] {
                    self.types.layout(element).map(|l| Layout { size: 0, ..l })
                } else {
                    None
                }
            } else {
                self.types.layout(member.ty)
            };
            let Some(layout) = layout else {
                if !self.types.unanalyzed(member.ty) && !self.variably_modified(member.ty) {
                    self.error(
                        SemanticErrorKind::InvalidMember,
                        source,
                        member.name.map(|n| n.name),
                        None,
                    );
                }
                tag.tainted.set(true);
                output.push(member);
                continue;
            };
            // The System V ABI ignores unnamed bit-fields when aligning a
            // structure or union; in a union one still occupies its bytes.
            let unnamed_bit_field = member.name.is_none() && member.width.is_some();
            if !(unnamed_bit_field || ms && tag.kind == TagKind::Union && member.width.is_some()) {
                alignment = alignment.max(layout.align);
            }
            let mut resolved = member;
            if tag.kind == TagKind::Union {
                bytes = bytes.max(match member.width {
                    | Some(0) if ms => 0,
                    | Some(_) if ms => layout.size,
                    | Some(width) if unnamed_bit_field => u64::from(width).div_ceil(8),
                    | _ => layout.size,
                });
            } else if let Some(width) = member.width {
                if ms {
                    // Microsoft ABI: a bit-field shares its predecessor's
                    // allocation unit only when the declared types have the
                    // same size and the unit has room.
                    let bits = layout.size * 8;
                    if width == 0 {
                        if ms_unit.take().is_some() {
                            alignment = alignment.max(layout.align);
                            bytes = align_up(bytes, layout.align).unwrap_or(bytes);
                        }
                        resolved.offset = bytes;
                    } else {
                        alignment = alignment.max(layout.align);
                        let unit = match ms_unit {
                            | Some((size, start, used))
                                if size == layout.size && used + u64::from(width) <= bits =>
                                (size, start, used),
                            | _ => {
                                let start = align_up(bytes, layout.align).unwrap_or(bytes);
                                bytes = start.saturating_add(layout.size);
                                (layout.size, start, 0)
                            },
                        };
                        resolved.offset = unit.1 + unit.2 / 8;
                        resolved.bit_offset = u32::try_from(unit.2 % 8).unwrap_or(0);
                        ms_unit = Some((unit.0, unit.1, unit.2 + u64::from(width)));
                    }
                } else {
                    // C99 §6.7.2.1p10: checked non-straddling allocation;
                    // §5.2.4.1: reject layouts beyond our representable cursor.
                    let placed = (|| {
                        let bits = layout.size.checked_mul(8)?;
                        let width = u64::from(width);
                        let start = if width == 0 {
                            align_up(bit_end, bits)?
                        } else {
                            let remaining = bits - bit_end.checked_rem(bits)?;
                            if width > remaining {
                                bit_end.checked_add(remaining)?
                            } else {
                                bit_end
                            }
                        };
                        Some((start, start.checked_add(width)?))
                    })();
                    let Some((start, end)) = placed else {
                        if !too_large {
                            self.error(
                                SemanticErrorKind::ObjectTooLarge,
                                source,
                                member.name.map(|n| n.name),
                                None,
                            );
                        }
                        too_large = true;
                        tag.tainted.set(true);
                        output.push(member);
                        continue;
                    };
                    resolved.offset = start / 8;
                    resolved.bit_offset = u32::try_from(start % 8).unwrap_or(0);
                    bit_end = end;
                    bytes = bytes.max(bit_end.div_ceil(8));
                }
            } else {
                ms_unit = None;
                let placed = align_up(bytes, layout.align).and_then(|offset| {
                    offset
                        .checked_add(layout.size)
                        .filter(|&end| end <= i64::MAX.unsigned_abs())
                        .map(|end| (offset, end))
                });
                let Some((offset, end)) = placed else {
                    // The record keeps the member but has no layout.
                    if !too_large {
                        self.error(
                            SemanticErrorKind::ObjectTooLarge,
                            source,
                            member.name.map(|n| n.name),
                            None,
                        );
                    }
                    too_large = true;
                    tag.tainted.set(true);
                    output.push(member);
                    continue;
                };
                resolved.offset = offset;
                bytes = end;
                bit_end = bytes.saturating_mul(8);
            }
            output.push(resolved);
        }
        _ = self.defining.remove(&id);
        tag.members.set(output.leak());
        let mut fields = ArenaVec::new_in(self.types.tu);
        for (index, member) in tag.members.get().iter().enumerate() {
            if let Some(name) = member.name {
                fields.push(Field {
                    name,
                    path: self.types.tu.alloc(FieldPath::new(index, None)),
                    ty: member.ty,
                    qualifiers: TypeQualifiers::empty(),
                    offset: member.offset,
                    bit_offset: member.bit_offset,
                    width: member.width,
                });
            } else if member.anonymous
                && let TypeKind::Tag(inner) = self.types.nodes[member.ty.index]
            {
                for field in self.types.tags[inner].fields.get() {
                    fields.push(Field {
                        path: self.types.tu.alloc(FieldPath::new(index, Some(field.path))),
                        qualifiers: field.qualifiers | member.ty.qualifiers,
                        offset: member.offset.saturating_add(field.offset),
                        ..*field
                    });
                }
            }
        }
        tag.fields.set(fields.leak());
        for (index, field) in tag.fields.get().iter().enumerate() {
            _ = self.member_indices.insert((id, field.name.name), index);
        }
        tag.complete.set(true);
        if !tag.tainted.get() {
            tag.layout
                .set(align_up(bytes, alignment).map(|size| Layout {
                    size,
                    align: alignment,
                }));
        }
    }
}
