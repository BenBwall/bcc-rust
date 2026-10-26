//! Microsoft C overload symbols follow Clang's Microsoft ABI, including the
//! C-linkage marker, written parameter qualifiers and shared backreferences.
//! Type traversal uses an explicit stack, including nested vendor templates.
//! C99: §6.2.2p2, p. 30; PDF p. 42 (extended linkage names).
//! <https://clang.llvm.org/docs/AttributeReference.html#overloadable>
//! <https://github.com/llvm/llvm-project/blob/main/clang/lib/AST/MicrosoftMangle.cpp>

use super::super::{
    Analyzer,
    ArrayBound,
    BindingKind,
    FunctionExtension,
    Scalar,
    TagKind,
    TypeId,
    TypeKind,
    WrittenFunction,
};
use crate::{
    translation_phases::parsing::declaration_syntax::TypeQualifiers,
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaString,
            ArenaVec,
        },
        string_cache::StringCacheId,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn mangle_msvc_function(
        &mut self,
        name: StringCacheId,
        ty: TypeId,
        extension: FunctionExtension<'tu>,
        internal: bool,
    ) -> StringCacheId {
        let mut output = ArenaString::new_in(self.scratch);
        output.push('?');
        output.push_str(self.context.string_cache.at(name));
        output.push_str(if internal { "@@YA" } else { "@@$$J0YA" });
        let mut names = ArenaVec::new_in(self.scratch);
        names.push(Name::Source(name));
        let mut arguments = ArenaVec::new_in(self.scratch);
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(Part::Signature(
            ty,
            Some(extension),
            WrittenAt::new(extension.written_functions),
        ));
        while let Some(part) = work.pop() {
            match part {
                | Part::Text(text) => output.push_str(text),
                | Part::Signature(ty, extension, written) => {
                    let TypeKind::Function {
                        result,
                        parameters,
                        prototype,
                        variadic,
                    } = self.types.nodes[ty.index]
                    else {
                        continue;
                    };
                    work.push(Part::Text(if variadic {
                        "ZZ"
                    } else if parameters.is_empty() && prototype {
                        "XZ"
                    } else {
                        "@Z"
                    }));
                    for (index, &parameter) in parameters.iter().enumerate().rev() {
                        if let Some(size) = extension.and_then(|ext| {
                            ext.sizes.iter().find(|size| size.index as usize == index)
                        }) {
                            work.push(Part::Size(size.mode, size.dynamic));
                        }
                        let declared = written
                            .parameters()
                            .and_then(|parameters| parameters.get(index))
                            .or_else(|| {
                                extension.and_then(|ext| ext.declared_parameters.get(index))
                            });
                        let mut ty = declared.map_or(parameter, |parameter| parameter.ty);
                        let original = declared.map_or(ty, |parameter| parameter.original);
                        let key =
                            if let TypeKind::Array(element, _) = self.types.nodes[original.index] {
                                ty = ty.qualified(TypeQualifiers::CONST);
                                Argument::Array(element.with_alignment(0))
                            } else {
                                Argument::Type(original.with_alignment(0))
                            };
                        let written = declared.map_or_else(WrittenAt::empty, |parameter| {
                            let mut written = WrittenAt::new(parameter.written_functions);
                            if matches!(self.types.nodes[original.index], TypeKind::Function { .. })
                            {
                                // C99 §6.7.5.3p8: adjustment inserts a pointer
                                // that has no corresponding written derivation.
                                written.depth = None;
                            }
                            written
                        });
                        work.push(Part::Argument(ty, key, written));
                    }
                    work.push(Part::Type(result, Mode::Result, written.next()));
                },
                | Part::Argument(ty, key, written) => {
                    if let Some(index) = arguments
                        .iter()
                        .position(|&previous| self.same_msvc_argument(previous, key))
                    {
                        output.push(char::from(
                            b'0' + u8::try_from(index).expect("ten backreferences"),
                        ));
                    } else {
                        work.push(Part::RegisterArgument(key, output.len()));
                        work.push(Part::Type(ty, Mode::Drop, written));
                    }
                },
                | Part::RegisterArgument(key, start) => {
                    if output.len() - start > 1
                        && arguments.len() < 10
                        && !arguments
                            .iter()
                            .any(|&previous| self.same_msvc_argument(previous, key))
                    {
                        arguments.push(key);
                    }
                },
                | Part::Size(mode, dynamic) => {
                    let key = Argument::Size(mode, dynamic);
                    if let Some(index) = arguments
                        .iter()
                        .position(|&previous| self.same_msvc_argument(previous, key))
                    {
                        output.push(char::from(
                            b'0' + u8::try_from(index).expect("ten backreferences"),
                        ));
                    } else {
                        let name = crate::diagnostics::format_in!(
                            self.scratch,
                            "__pass_{}object_size{mode}",
                            if dynamic { "dynamic_" } else { "" }
                        );
                        let name = self.context.string_cache.intern(name);
                        output.push_str("W4");
                        source_name(
                            &mut output,
                            &mut names,
                            name,
                            self.context.string_cache.at(name),
                        );
                        let namespace = self.context.string_cache.intern("__clang");
                        source_name(&mut output, &mut names, namespace, "__clang");
                        output.push('@');
                        if arguments.len() < 10 {
                            arguments.push(key);
                        }
                    }
                },
                | Part::Type(ty, mode, written) => {
                    let kind = self.types.nodes[ty.index];
                    let pointer = matches!(kind, TypeKind::Pointer(_));
                    if let TypeKind::Array(_, _) = kind {
                        output.push_str(match mode {
                            | Mode::Mangle => "A",
                            | Mode::Escape | Mode::Result => "$$B",
                            | Mode::Drop => "",
                        });
                    } else {
                        match mode {
                            | Mode::Mangle => {
                                if matches!(kind, TypeKind::Function { .. }) {
                                    output.push_str("6A");
                                    work.push(Part::Signature(ty, None, written));
                                    continue;
                                }
                                output.push(cv(ty.qualifiers));
                            },
                            | Mode::Escape if !pointer && !ty.qualifiers.is_empty() => {
                                output.push_str("$$C");
                                output.push(cv(ty.qualifiers));
                            },
                            | Mode::Result
                                if (!pointer && !ty.qualifiers.is_empty())
                                    || matches!(
                                        kind,
                                        TypeKind::Tag(_)
                                            | TypeKind::Atomic(_)
                                            | TypeKind::Vector { .. }
                                            | TypeKind::Scalar(
                                                Scalar::ComplexFloat
                                                    | Scalar::ComplexDouble
                                                    | Scalar::ComplexLongDouble
                                                    | Scalar::ComplexFloat128
                                            )
                                    ) =>
                            {
                                output.push('?');
                                output.push(cv(ty.qualifiers));
                            },
                            | _ => {},
                        }
                    }
                    match kind {
                        | TypeKind::Scalar(scalar) => {
                            let complex = match scalar {
                                | Scalar::ComplexFloat => Some(Scalar::Float),
                                | Scalar::ComplexDouble => Some(Scalar::Double),
                                | Scalar::ComplexLongDouble => Some(Scalar::LongDouble),
                                | Scalar::ComplexFloat128 => Some(Scalar::Float128),
                                | _ => None,
                            };
                            if let Some(real) = complex {
                                let inner = self.types.scalar(real);
                                work.push(Part::Vendor(
                                    ty.unqualified().with_alignment(0),
                                    inner,
                                    "_Complex",
                                    None,
                                    WrittenAt::empty(),
                                ));
                            } else {
                                output.push_str(scalar_code(scalar));
                            }
                        },
                        | TypeKind::Nullptr => output.push_str("$$T"),
                        | TypeKind::Pointer(inner) => {
                            output.push(
                                match (
                                    ty.qualifiers.contains(TypeQualifiers::CONST),
                                    ty.qualifiers.contains(TypeQualifiers::VOLATILE),
                                ) {
                                    | (false, false) => 'P',
                                    | (true, false) => 'Q',
                                    | (false, true) => 'R',
                                    | (true, true) => 'S',
                                },
                            );
                            if !matches!(self.types.nodes[inner.index], TypeKind::Function { .. }) {
                                output.push('E');
                            }
                            if ty.qualifiers.contains(TypeQualifiers::RESTRICT) {
                                output.push('I');
                            }
                            work.push(Part::Type(inner, Mode::Mangle, written.next()));
                        },
                        | TypeKind::Array(_, _) => {
                            let mut inner = ty;
                            let mut written = written;
                            let mut bounds = ArenaVec::new_in(self.scratch);
                            while let TypeKind::Array(element, bound) =
                                self.types.nodes[inner.index]
                            {
                                bounds.push(match bound {
                                    | ArrayBound::Constant(size) => size,
                                    | _ => 0,
                                });
                                inner = element;
                                written = written.next();
                            }
                            output.push('Y');
                            number(
                                &mut output,
                                u64::try_from(bounds.len()).expect("array dimensions fit"),
                            );
                            for &bound in &bounds {
                                number(&mut output, bound);
                            }
                            work.push(Part::Type(inner, Mode::Escape, written));
                        },
                        | TypeKind::Function { .. } => {
                            output.push_str("$$A6A");
                            work.push(Part::Signature(ty, None, written));
                        },
                        | TypeKind::Tag(tag) => {
                            output.push_str(match self.types.tags[tag].kind {
                                | TagKind::Struct => "U",
                                | TagKind::Union => "T",
                                | TagKind::Enum => "W4",
                            });
                            self.msvc_tag_name(&mut output, &mut names, tag);
                        },
                        | TypeKind::Atomic(inner) => work.push(Part::Vendor(
                            ty.unqualified().with_alignment(0),
                            inner,
                            "_Atomic",
                            None,
                            written.next(),
                        )),
                        | TypeKind::Vector { element, count, .. } => {
                            let width = self.types.layout(ty).map_or(0, |layout| layout.size * 8);
                            let intrinsic = match self.types.nodes[element.index] {
                                | TypeKind::Scalar(Scalar::LongLong) if width == 64 =>
                                    Some(("T", "__m64")),
                                | TypeKind::Scalar(
                                    Scalar::Float | Scalar::Double | Scalar::LongLong,
                                ) if width >= 128 => {
                                    let (kind, suffix) = match self.types.nodes[element.index] {
                                        | TypeKind::Scalar(Scalar::Double) => ("U", "d"),
                                        | TypeKind::Scalar(Scalar::LongLong) => ("T", "i"),
                                        | _ => ("T", ""),
                                    };
                                    let name = crate::diagnostics::format_in!(
                                        self.scratch,
                                        "__m{width}{suffix}"
                                    );
                                    Some((kind, name))
                                },
                                | _ => None,
                            };
                            if let Some((kind, name)) = intrinsic {
                                output.push_str(kind);
                                let id = self.context.string_cache.intern(name);
                                source_name(&mut output, &mut names, id, name);
                                output.push('@');
                            } else {
                                work.push(Part::Vendor(
                                    ty.unqualified().with_alignment(0),
                                    element,
                                    "__vector",
                                    Some(count),
                                    WrittenAt::empty(),
                                ));
                            }
                        },
                        | TypeKind::Unknown | TypeKind::Unmodeled(_) => {},
                    }
                },
                | Part::Vendor(ty, inner, name, count, written) => {
                    output.push(if count.is_some() { 'T' } else { 'U' });
                    if let Some(index) = names
                        .iter()
                        .position(|&previous| matches!(previous, Name::Vendor(previous) if self.same_abi_type(previous, ty)))
                    {
                        output.push(char::from(
                            b'0' + u8::try_from(index).expect("ten backreferences"),
                        ));
                        output.push('@');
                    } else {
                        output.push_str("?$");
                        output.push_str(name);
                        output.push('@');
                        let saved_names =
                            std::mem::replace(&mut names, ArenaVec::new_in(self.scratch));
                        let saved_arguments =
                            std::mem::replace(&mut arguments, ArenaVec::new_in(self.scratch));
                        work.push(Part::EndVendor(ty, saved_names, saved_arguments));
                        if let Some(count) = count {
                            work.push(Part::Number(count));
                            work.push(Part::Text("$0"));
                        }
                        work.push(Part::Type(inner, Mode::Escape, written));
                    }
                },
                | Part::Number(value) => number(&mut output, value),
                | Part::EndVendor(ty, saved_names, saved_arguments) => {
                    output.push('@');
                    names = saved_names;
                    arguments = saved_arguments;
                    if names.len() < 10 {
                        names.push(Name::Vendor(ty));
                    }
                    let namespace = self.context.string_cache.intern("__clang");
                    source_name(&mut output, &mut names, namespace, "__clang");
                    output.push('@');
                },
            }
        }
        self.context.string_cache.intern(output.as_str())
    }

    fn same_msvc_argument(&self, left: Argument, right: Argument) -> bool {
        match (left, right) {
            | (Argument::Type(a), Argument::Type(b)) | (Argument::Array(a), Argument::Array(b)) =>
                self.same_abi_type(a, b),
            | _ => left == right,
        }
    }

    fn msvc_tag_name(
        &mut self,
        output: &mut ArenaString<'_>,
        names: &mut ArenaVec<'_, Name>,
        tag: usize,
    ) {
        let name = self.types.tags[tag]
            .name
            .or_else(|| self.function_extensions.typedef_names.get(&tag).copied());
        let name = name.unwrap_or_else(|| {
            let object = self.bindings.iter().find(|binding| {
                binding.kind == BindingKind::Object
                    && matches!(self.types.nodes[binding.ty.index], TypeKind::Tag(id) if id == tag)
            });
            let text = if let Some(object) = object {
                crate::diagnostics::format_in!(
                    self.scratch,
                    "<unnamed-type-{}>",
                    self.context.string_cache.at(object.name.name)
                )
            } else {
                crate::diagnostics::format_in!(self.scratch, "<unnamed-type-{tag}>")
            };
            self.context.string_cache.intern(text)
        });
        source_name(output, names, name, self.context.string_cache.at(name));
        let scope = self
            .tag_declarations
            .iter()
            .find_map(|&(id, scope)| (id == tag).then_some(scope))
            .unwrap_or(0);
        let mut ancestor = Some(scope);
        while let Some(scope) = ancestor {
            if let Some(function) = self.function_records.iter().find(|f| f.scope == scope) {
                if let Some(binding) = function.binding {
                    let ordinal = self.local_tag_ordinal(tag, scope, name);
                    output.push('?');
                    number(
                        output,
                        u64::try_from(ordinal).expect("local tag ordinal fits") + 2,
                    );
                    output.push('?');
                    let identity = self.function_identity(binding);
                    if let Some(extension) = self
                        .function_extensions
                        .function(binding)
                        .filter(|e| e.overloadable)
                    {
                        let abi_name = self
                            .function_extensions
                            .abi_names
                            .get(&identity)
                            .copied()
                            .unwrap_or(extension.abi_name);
                        output.push_str(self.context.string_cache.at(abi_name));
                    } else {
                        output.push('?');
                        output.push_str(
                            self.context
                                .string_cache
                                .at(self.bindings[binding].name.name),
                        );
                        output.push_str("@@9");
                    }
                }
                break;
            }
            ancestor = self.scopes[scope].parent;
        }
        output.push('@');
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    target_pointer_width = "64",
    expect(
        variant_size_differences,
        reason = "Vendor names require canonical type keys; source names use compact interned IDs."
    )
)]
enum Name {
    Source(StringCacheId),
    Vendor(TypeId),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Argument {
    Type(TypeId),
    Array(TypeId),
    Size(u8, bool),
}
#[derive(Clone, Copy)]
enum Mode {
    Drop,
    Mangle,
    Escape,
    Result,
}
enum Part<'s, 'tu> {
    Text(&'static str),
    Type(TypeId, Mode, WrittenAt<'tu>),
    Signature(TypeId, Option<FunctionExtension<'tu>>, WrittenAt<'tu>),
    Argument(TypeId, Argument, WrittenAt<'tu>),
    RegisterArgument(Argument, usize),
    Size(u8, bool),
    Vendor(TypeId, TypeId, &'static str, Option<u64>, WrittenAt<'tu>),
    Number(u64),
    EndVendor(TypeId, ArenaVec<'s, Name>, ArenaVec<'s, Argument>),
}

#[derive(Clone, Copy)]
struct WrittenAt<'tu> {
    functions: ArenaList<'tu, WrittenFunction<'tu>>,
    depth:     Option<usize>,
}
impl<'tu> WrittenAt<'tu> {
    fn new(functions: ArenaList<'tu, WrittenFunction<'tu>>) -> Self {
        Self {
            functions,
            depth: Some(0),
        }
    }

    fn empty() -> Self {
        Self::new(ArenaList::empty())
    }

    fn next(self) -> Self {
        Self {
            depth: Some(self.depth.map_or(0, |depth| depth + 1)),
            ..self
        }
    }

    fn parameters(self) -> Option<&'tu [super::super::Parameter<'tu>]> {
        self.functions
            .iter()
            .find(|function| Some(function.depth) == self.depth)
            .map(|function| function.parameters)
    }
}
fn source_name(
    output: &mut ArenaString<'_>,
    names: &mut ArenaVec<'_, Name>,
    id: StringCacheId,
    text: &str,
) {
    if let Some(index) = names
        .iter()
        .position(|&previous| previous == Name::Source(id))
    {
        output.push(char::from(
            b'0' + u8::try_from(index).expect("ten backreferences"),
        ));
    } else {
        output.push_str(text);
        output.push('@');
        if names.len() < 10 {
            names.push(Name::Source(id));
        }
    }
}
fn cv(qualifiers: TypeQualifiers) -> char {
    match (
        qualifiers.contains(TypeQualifiers::CONST),
        qualifiers.contains(TypeQualifiers::VOLATILE),
    ) {
        | (false, false) => 'A',
        | (true, false) => 'B',
        | (false, true) => 'C',
        | (true, true) => 'D',
    }
}
fn number(output: &mut ArenaString<'_>, value: u64) {
    if value == 0 {
        output.push_str("A@");
    } else if value <= 10 {
        output.push(char::from(
            b'0' + u8::try_from(value - 1).expect("small number"),
        ));
    } else {
        let mut shift = 60;
        while shift > 0 && (value >> shift) == 0 {
            shift -= 4;
        }
        loop {
            output.push(char::from(
                b'A' + u8::try_from((value >> shift) & 15).expect("hex digit"),
            ));
            if shift == 0 {
                break;
            }
            shift -= 4;
        }
        output.push('@');
    }
}
fn scalar_code(scalar: Scalar) -> &'static str {
    match scalar {
        | Scalar::Void => "X",
        | Scalar::Bool => "_N",
        | Scalar::Char => "D",
        | Scalar::SignedChar => "C",
        | Scalar::UnsignedChar => "E",
        | Scalar::Short => "F",
        | Scalar::UnsignedShort => "G",
        | Scalar::Int => "H",
        | Scalar::UnsignedInt => "I",
        | Scalar::Long => "J",
        | Scalar::UnsignedLong => "K",
        | Scalar::LongLong => "_J",
        | Scalar::UnsignedLongLong => "_K",
        | Scalar::Int128 => "_L",
        | Scalar::UnsignedInt128 => "_M",
        | Scalar::Float => "M",
        | Scalar::Double => "N",
        | Scalar::LongDouble => "O",
        | Scalar::Float128 => "_T",
        | Scalar::ComplexFloat
        | Scalar::ComplexDouble
        | Scalar::ComplexLongDouble
        | Scalar::ComplexFloat128 => unreachable!("complex types use vendor templates"),
    }
}
