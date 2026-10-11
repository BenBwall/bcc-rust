//! Phase-7 semantic types, interning, compatibility and object layout.
//! C99: §6.2.5, pp. 33-37; PDF pp. 45-49; §6.2.7, pp. 40-41;
//! PDF pp. 52-53; §6.7.2-§6.7.6, pp. 99-124; PDF pp. 111-136.
//! Expression constraints and initializer completion use this canonical graph.

use std::cell::Cell;

use rustc_hash::FxBuildHasher;

pub(crate) use crate::target::{
    Layout,
    Scalar,
    TargetLayout,
};
use crate::{
    translation_phases::parsing::declaration_syntax::TypeQualifiers,
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl<'tu, 's> TypeInterner<'tu, 's> {
    pub(crate) fn intern(&mut self, kind: TypeKind<'tu>) -> TypeId {
        if let Some(&ty) = self.keys.get(&kind) {
            return ty;
        }
        let ty = TypeId {
            index:      self.nodes.len(),
            qualifiers: TypeQualifiers::empty(),
        };
        let variably_modified = match kind {
            | TypeKind::Array(_, ArrayBound::Variable | ArrayBound::Star) => true,
            // C11 §6.7.6p3: derived types retain variably modified state.
            | TypeKind::Array(next, _) | TypeKind::Pointer(next) | TypeKind::Atomic(next) =>
                self.variably_modified[next.index],
            | TypeKind::Function { result, .. } => self.variably_modified[result.index],
            | _ => false,
        };
        // C99 §6.2.5p20,22: inherit the immutable tail from the immediate
        // element, so shared array derivations are never rescanned.
        #[cfg(test)]
        self.classification_step();
        let tail = if let TypeKind::Array(element, bound) = kind {
            let element_tail = self.array_tails[element.index];
            ArrayTail {
                incomplete: element_tail.incomplete || bound == ArrayBound::Incomplete,
                ..element_tail
            }
        } else {
            ArrayTail {
                index:      ty.index,
                incomplete: false,
            }
        };
        self.array_tails.push(tail);
        self.variably_modified.push(variably_modified);
        self.nodes.push(kind);
        _ = self.keys.insert(kind, ty);
        ty
    }

    /// Compatibility and composite construction use an explicit postorder
    /// stack. C99: §6.2.7p1-4, p. 40; PDF p. 52; §6.7.5.1p2, p.
    /// 115; PDF p. 127; §6.7.5.2p6, p. 117; PDF p. 129; §6.7.5.3p15, pp.
    /// 119-120; PDF pp. 131-132.
    #[expect(
        clippy::many_single_char_names,
        reason = "Paired type derivations use conventional left/right algebra names."
    )]
    pub(crate) fn composite(&mut self, left: TypeId, right: TypeId) -> Option<TypeId> {
        enum Work<'a> {
            Pair(TypeId, TypeId),
            Pointer(TypeQualifiers),
            Array(ArrayBound, TypeQualifiers),
            Function(&'a [TypeId], bool, bool, TypeQualifiers),
            Atomic(TypeQualifiers),
        }
        let mut work = ArenaVec::new_in(self.scratch);
        let mut values = ArenaVec::new_in(self.scratch);
        work.push(Work::Pair(left, right));
        while let Some(item) = work.pop() {
            match item {
                | Work::Pair(a, b) => {
                    if a == b {
                        values.push(a);
                        continue;
                    }
                    let (ak, bk) = (self.nodes[a.index], self.nodes[b.index]);
                    if matches!(ak, TypeKind::Unknown)
                        || matches!(ak, TypeKind::Tag(id) if self.tags[id].tainted.get())
                    {
                        values.push(b);
                        continue;
                    }
                    if matches!(bk, TypeKind::Unknown)
                        || matches!(bk, TypeKind::Tag(id) if self.tags[id].tainted.get())
                    {
                        values.push(a);
                        continue;
                    }
                    if a.qualifiers != b.qualifiers {
                        return None;
                    }
                    match (ak, bk) {
                        | (
                            TypeKind::Vector {
                                element: x,
                                count: n,
                                ..
                            },
                            TypeKind::Vector {
                                element: y,
                                count: m,
                                ..
                            },
                        ) if x == y && n == m => {
                            values.push(a);
                        },
                        | (TypeKind::Tag(id), TypeKind::Scalar(scalar))
                        | (TypeKind::Scalar(scalar), TypeKind::Tag(id))
                            if self.tags[id].kind == TagKind::Enum
                                && self.tags[id].compatible.get() == scalar =>
                        {
                            values.push(if matches!(ak, TypeKind::Tag(_)) { a } else { b });
                        },
                        | (TypeKind::Pointer(x), TypeKind::Pointer(y)) => {
                            work.push(Work::Pointer(a.qualifiers));
                            work.push(Work::Pair(x, y));
                        },
                        | (TypeKind::Atomic(x), TypeKind::Atomic(y)) => {
                            work.push(Work::Atomic(a.qualifiers));
                            work.push(Work::Pair(x, y));
                        },
                        | (TypeKind::Array(x, ab), TypeKind::Array(y, bb)) => {
                            if matches!((ab,bb), (ArrayBound::Constant(n),ArrayBound::Constant(m)) if n != m)
                            {
                                return None;
                            }
                            let bound = if matches!(ab, ArrayBound::Constant(_)) {
                                ab
                            } else {
                                bb
                            };
                            work.push(Work::Array(bound, a.qualifiers));
                            work.push(Work::Pair(x, y));
                        },
                        | (
                            TypeKind::Function {
                                result: x,
                                parameters: ap,
                                prototype: aproto,
                                variadic: av,
                            },
                            TypeKind::Function {
                                result: y,
                                parameters: bp,
                                prototype: bproto,
                                variadic: bv,
                            },
                        ) => {
                            if aproto && bproto && (ap.len() != bp.len() || av != bv) {
                                return None;
                            }
                            let params = if aproto || (!bproto && !ap.is_empty()) {
                                ap
                            } else {
                                bp
                            };
                            let old_params = if aproto { bp } else { ap };
                            if aproto != bproto
                                && !old_params.is_empty()
                                && params.len() != old_params.len()
                            {
                                return None;
                            }
                            if aproto != bproto
                                && (av || bv || params.iter().any(|p| !self.promotion_stable(*p)))
                            {
                                return None;
                            }
                            work.push(Work::Function(
                                params,
                                aproto || bproto,
                                av || bv,
                                a.qualifiers,
                            ));
                            if (aproto && bproto) || (aproto != bproto && !old_params.is_empty()) {
                                for (&a, &b) in ap.iter().zip(bp).rev() {
                                    work.push(Work::Pair(a.unqualified(), b.unqualified()));
                                }
                            } else {
                                for &p in params.iter().rev() {
                                    work.push(Work::Pair(p, p));
                                }
                            }
                            work.push(Work::Pair(x, y));
                        },
                        | _ => return None,
                    }
                },
                | Work::Pointer(q) => {
                    let x = values.pop()?;
                    values.push(self.intern(TypeKind::Pointer(x)).qualified(q));
                },
                | Work::Atomic(q) => {
                    let x = values.pop()?;
                    values.push(self.intern(TypeKind::Atomic(x)).qualified(q));
                },
                | Work::Array(b, q) => {
                    let x = values.pop()?;
                    values.push(self.intern(TypeKind::Array(x, b)).qualified(q));
                },
                | Work::Function(p, prototype, variadic, q) => {
                    let start = values.len().checked_sub(p.len() + 1)?;
                    let result = values[start];
                    let parameters = self.tu.alloc_slice_copy(&values[start + 1..]);
                    values.truncate(start);
                    values.push(
                        self.intern(TypeKind::Function {
                            result,
                            parameters,
                            prototype,
                            variadic,
                        })
                        .qualified(q),
                    );
                },
            }
        }
        values.pop()
    }

    /// C99: §6.5.3.4p2-4, p. 80; PDF p. 92.
    pub(crate) fn layout(&self, ty: TypeId) -> Option<Layout> {
        layout(&self.nodes, &self.tags, &self.target, ty)
    }

    /// Array alignment depends on the element, regardless of its extent.
    /// C11: §6.5.3.4p3, p. 90; PDF p. 108 (an extension in C99).
    pub(crate) fn alignment(&self, ty: TypeId) -> Option<u64> {
        let ty = TypeId {
            index: self.array_tails[ty.index].index,
            ..ty
        };
        self.layout(ty).map(|layout| layout.align)
    }

    /// Remove top-level qualification, including qualification carried by
    /// array elements, without changing pointed-to types or atomic identity.
    /// C99: §6.7.3p8, p. 109; PDF p. 121.
    /// C23: §6.7.3.6p5, p. 118; PDF p. 131 (`typeof_unqual` caller).
    pub(crate) fn unqualified_array(&mut self, mut ty: TypeId) -> TypeId {
        let mut bounds = ArenaVec::new_in(self.scratch);
        while let TypeKind::Array(element, bound) = self.nodes[ty.index] {
            bounds.push(bound);
            ty = element;
        }
        ty = ty.unqualified();
        while let Some(bound) = bounds.pop() {
            ty = self.intern(TypeKind::Array(ty, bound));
        }
        ty
    }

    /// Removes atomicity as required by lvalue value conversion.
    /// C11: §6.3.2.1 paragraph 2, p. 54; PDF p. 72.
    pub(crate) fn non_atomic(&self, ty: TypeId) -> TypeId {
        match self.nodes[ty.index] {
            | TypeKind::Atomic(value) => value.qualified(ty.qualifiers),
            | _ => ty,
        }
    }

    pub(crate) fn new(tu: &'tu Bump, scratch: &'s Bump, target: &TargetLayout) -> Self {
        let mut result = Self {
            #[cfg(test)]
            steps: Cell::new(0),
            nodes: ArenaVec::new_in(tu),
            tags: ArenaVec::new_in(tu),
            keys: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            variably_modified: ArenaVec::new_in(scratch),
            array_tails: ArenaVec::new_in(scratch),
            target: *target,
            tu,
            scratch,
        };
        _ = result.intern(TypeKind::Unknown);
        result
    }

    pub(crate) fn finish(self) -> Types<'tu> {
        Types {
            nodes:  self.nodes.leak(),
            tags:   self.tags.leak(),
            target: self.target,
        }
    }

    pub(crate) fn scalar(&mut self, scalar: Scalar) -> TypeId {
        self.intern(TypeKind::Scalar(scalar))
    }

    pub(crate) fn unknown(&self) -> TypeId {
        self.keys[&TypeKind::Unknown]
    }

    /// A variable or `[*]` array derivation reached through arrays, pointers
    /// atomic wrappers or function results; constant time for shared typedef
    /// graphs. C99: §6.7.5p3, pp. 114-115; PDF pp. 126-127; §6.7.5.2p2, p. 116;
    /// PDF
    /// p. 128. C11: §6.7.6p3, p. 129; PDF p. 147.
    pub(crate) fn variably_modified(&self, ty: TypeId) -> bool {
        self.variably_modified[ty.index]
    }

    /// Unmodeled extension layout suppresses dependent constraints.
    /// C99: array derivations §6.2.5p20, pp. 35-36; PDF pp. 47-48.
    pub(crate) fn unanalyzed(&self, ty: TypeId) -> bool {
        #[cfg(test)]
        self.classification_step();
        match self.nodes[self.array_tails[ty.index].index] {
            | TypeKind::Unknown => true,
            | TypeKind::Tag(id) => self.tags[id].tainted.get(),
            | _ => false,
        }
    }

    /// Array bounds are immutable, but a terminal tag can complete later.
    /// C99: §6.2.5p22, p. 36; PDF p. 48; §6.7.5.2p1, p. 116; PDF p. 128.
    pub(crate) fn complete_object(&self, ty: TypeId) -> bool {
        #[cfg(test)]
        self.classification_step();
        let tail = self.array_tails[ty.index];
        !tail.incomplete
            && match self.nodes[tail.index] {
                | TypeKind::Tag(id) => self.tags[id].complete.get(),
                | TypeKind::Scalar(Scalar::Void)
                | TypeKind::Function { .. }
                | TypeKind::Unknown => false,
                | _ => true,
            }
    }

    #[cfg(test)]
    pub(super) fn classification_step(&self) {
        self.steps.set(self.steps.get() + 1);
    }

    /// C99: §6.7.5.3p15, pp. 119-120; PDF pp. 131-132.
    fn promotion_stable(&self, ty: TypeId) -> bool {
        !matches!(
            self.nodes[ty.index],
            TypeKind::Scalar(
                Scalar::Bool
                    | Scalar::Char
                    | Scalar::SignedChar
                    | Scalar::UnsignedChar
                    | Scalar::Short
                    | Scalar::UnsignedShort
                    | Scalar::Float
            )
        )
    }
}

/// Working hash-cons table; only its immutable graph escapes into `'tu`.
pub(crate) struct TypeInterner<'tu, 's> {
    /// Type nodes examined by completeness and recovery queries in tests.
    #[cfg(test)]
    pub(super) steps:  Cell<usize>,
    pub(crate) nodes:  ArenaVec<'tu, TypeKind<'tu>>,
    pub(crate) tags:   ArenaVec<'tu, &'tu Tag<'tu>>,
    keys:              ArenaMap<'s, TypeKind<'tu>, TypeId>,
    /// Variably modified nodes, decided once from immediate children.
    variably_modified: ArenaVec<'s, bool>,
    array_tails:       ArenaVec<'s, ArrayTail>,
    pub(crate) target: TargetLayout,
    pub(crate) tu:     &'tu Bump,
    scratch:           &'s Bump,
}

/// Canonical unqualified type identity plus its compact qualifier set.
/// C99: §6.2.5p26, p. 36; PDF p. 48; §6.7.3, pp. 108-109; PDF pp. 120-121.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TypeId {
    pub(crate) index:      usize,
    pub(crate) qualifiers: TypeQualifiers,
}

/// Hash-consing key. Child identities keep hashing bounded by immediate arity.
/// C99: derived types §6.2.5p20, pp. 35-36; PDF pp. 47-48.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TypeKind<'tu> {
    Unknown,
    Scalar(Scalar),
    Pointer(TypeId),
    Array(TypeId, ArrayBound),
    Function {
        result:     TypeId,
        parameters: &'tu [TypeId],
        prototype:  bool,
        variadic:   bool,
    },
    Tag(usize),
    /// An atomic type is distinct from its corresponding non-atomic type.
    /// C11: §6.2.5 paragraph 27, p. 43; PDF p. 61.
    Atomic(TypeId),
    /// GCC Vector Extensions: arithmetic element, lane count and ABI alignment.
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    Vector {
        element: TypeId,
        count:   u64,
        align:   u64,
    },
}

/// Durable type graph, retained after semantic working storage is released.
#[derive(Debug)]
pub(crate) struct Types<'tu> {
    pub(crate) nodes:  &'tu [TypeKind<'tu>],
    pub(crate) tags:   &'tu [&'tu Tag<'tu>],
    pub(crate) target: TargetLayout,
}

/// Retained nominal type; completion mutates only its member/layout cells.
/// C99: §6.7.2.3p4-8, pp. 106-107; PDF pp. 118-119.
#[derive(Debug)]
pub(crate) struct Tag<'tu> {
    pub(crate) name:              Option<StringCacheId>,
    pub(crate) kind:              TagKind,
    pub(crate) members:           Cell<&'tu [Member]>,
    /// The member namespace in declaration order, set at completion.
    pub(crate) fields:            Cell<&'tu [Field<'tu>]>,
    pub(crate) layout:            Cell<Option<Layout>>,
    pub(crate) complete:          Cell<bool>,
    pub(crate) tainted:           Cell<bool>,
    pub(crate) contains_flexible: Cell<bool>,
    /// The implementation-defined compatible integer type fixed at enum
    /// completion.
    /// C99: §6.7.2.2 paragraph 4, p. 105; PDF p. 117.
    pub(crate) compatible:        Cell<Scalar>,
}

/// A record member including ABI byte/bit offset and source bit-field width.
/// An anonymous member is an unnamed structure or union member whose own
/// members belong to the containing record (C11 §6.7.2.1p13, p. 115; PDF
/// p. 133; a GNU and MSVC extension before C11).
/// C99: §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Member {
    pub(crate) name:           Option<super::Identifier>,
    /// Declarator provenance, including unnamed bit-fields.
    pub(crate) source_vectors: super::SourceVectors,
    /// A rejected declarator must not produce dependent completion errors.
    pub(crate) invalid:        bool,
    pub(crate) ty:             TypeId,
    pub(crate) offset:         u64,
    pub(crate) bit_offset:     u32,
    pub(crate) width:          Option<u32>,
    pub(crate) anonymous:      bool,
}

/// One name in a record's member namespace, including the names an anonymous
/// member contributes, resolved to the member that declares it. A backend
/// can lower member access from the byte offset without walking `path`.
/// C11: §6.7.2.1p13, p. 115; PDF p. 133. C99: §6.5.2.3p3, p. 73; PDF p. 85.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Field<'tu> {
    pub(crate) name:       super::Identifier,
    /// Member indices from this record through each anonymous member.
    pub(crate) path:       &'tu FieldPath<'tu>,
    /// The declaring member's type.
    pub(crate) ty:         TypeId,
    /// Qualifiers of the anonymous members on the path.
    pub(crate) qualifiers: TypeQualifiers,
    /// Byte offset from the start of this record.
    pub(crate) offset:     u64,
    pub(crate) bit_offset: u32,
    pub(crate) width:      Option<u32>,
}

/// A shared path from a containing record to a declaring member. Prepending
/// an anonymous member retains its child's tail without copying the indices.
/// C11: §6.7.2.1p13, p. 115; PDF p. 133.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FieldPath<'tu> {
    pub(crate) index: usize,
    pub(crate) next:  Option<&'tu Self>,
    length:           usize,
}

/// Function parameter name and declared (adjusted) type.
/// C99: §6.7.5.3p7-8, p. 119; PDF p. 131.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Parameter {
    pub(crate) name:          Option<super::Identifier>,
    pub(crate) ty:            TypeId,
    pub(crate) array_minimum: Option<ArrayBound>,
}

/// Array completeness and run-time extent; `Star` is prototype-scope `[*]`.
/// C99: §6.7.5.2, pp. 116-118; PDF pp. 128-130.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ArrayBound {
    Incomplete,
    Constant(u64),
    Variable,
    Star,
}

/// Nominal tag kind. Members and labels never share the ordinary namespace.
/// C99: §6.2.3, p. 31; PDF p. 43; §6.7.2.3, pp. 106-107; PDF pp. 118-119.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TagKind {
    Struct,
    Union,
    Enum,
}

/// Immutable array derivations stop at a scalar, pointer, function or tag.
/// Tag completion and recovery taint are queried live at that terminal node.
/// C99: §6.2.5p20, pp. 35-36; PDF pp. 47-48; §6.2.5p22, p. 36; PDF p. 48.
#[derive(Clone, Copy)]
struct ArrayTail {
    index:      usize,
    incomplete: bool,
}

/// Iteratively strips array derivations and multiplies their extents; the
/// retained graph and the working interner share this one definition.
/// C99: §6.5.3.4p2-4, p. 80; PDF p. 92.
fn layout(
    nodes: &[TypeKind<'_>],
    tags: &[&Tag<'_>],
    target: &TargetLayout,
    mut ty: TypeId,
) -> Option<Layout> {
    let mut count = 1_u64;
    let mut atomic = false;
    let layout = loop {
        match nodes[ty.index] {
            | TypeKind::Scalar(scalar) => break target.scalar(scalar)?,
            | TypeKind::Vector {
                element,
                count,
                align,
            } => {
                let TypeKind::Scalar(scalar) = nodes[element.index] else {
                    return None;
                };
                break Layout {
                    size: target
                        .scalar(scalar)?
                        .size
                        .checked_mul(count)?
                        .checked_next_power_of_two()?,
                    align,
                };
            },
            | TypeKind::Pointer(_) => break target.pointer,
            | TypeKind::Tag(id) => break tags[id].layout.get()?,
            | TypeKind::Array(element, ArrayBound::Constant(n)) => {
                count = count.checked_mul(n)?;
                ty = element;
            },
            | TypeKind::Atomic(value) => {
                atomic = true;
                ty = value;
            },
            | _ => return None,
        }
    };
    let layout = if atomic && layout.size <= 16 {
        let size = layout.size.next_power_of_two();
        Layout {
            size,
            align: layout.align.max(size),
        }
    } else {
        layout
    };
    Some(Layout {
        size: layout.size.checked_mul(count)?,
        ..layout
    })
}

pub(crate) fn align_up(value: u64, align: u64) -> Option<u64> {
    value.checked_add(align - 1).map(|n| n & !(align - 1))
}

impl TypeId {
    /// C99: §6.7.3p9, p. 109; PDF p. 121.
    pub(crate) fn unqualified(self) -> Self {
        Self {
            qualifiers: TypeQualifiers::empty(),
            ..self
        }
    }

    /// C99: §6.7.3p9, p. 109; PDF p. 121.
    pub(crate) fn qualified(self, qualifiers: TypeQualifiers) -> Self {
        Self {
            qualifiers: self.qualifiers | qualifiers,
            ..self
        }
    }
}

impl Member {
    /// Unnamed bit-fields do not participate in initialization; anonymous
    /// structure/union members contribute their members to the containing
    /// record.
    /// C99: §6.7.8 paragraph 9, p. 126; PDF p. 138.
    /// C11: §6.7.2.1 paragraph 13, p. 115; PDF p. 133.
    pub(crate) const fn initializable(&self) -> bool {
        self.name.is_some() || self.anonymous
    }
}

impl<'tu> FieldPath<'tu> {
    /// C11: §6.7.2.1p13, p. 115; PDF p. 133.
    pub(crate) fn new(index: usize, next: Option<&'tu Self>) -> Self {
        Self {
            index,
            next,
            length: next.map_or(1, |path| path.length + 1),
        }
    }

    /// C11: §6.7.2.1p13, p. 115; PDF p. 133.
    pub(crate) const fn len(&self) -> usize {
        self.length
    }

    /// Walks the member indices in current-object order without recursion.
    /// C99: §6.7.8p18, p. 127; PDF p. 139; C11: §6.7.2.1p13,
    /// p. 115; PDF p. 133.
    pub(crate) fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(self), |path| path.next).map(|path| path.index)
    }
}

impl Types<'_> {
    /// C99: §6.5.3.4p2-4, p. 80; PDF p. 92.
    pub(crate) fn layout(&self, ty: TypeId) -> Option<Layout> {
        layout(self.nodes, self.tags, &self.target, ty)
    }
}
