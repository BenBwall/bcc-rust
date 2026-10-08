//! Phase-7 semantic types, interning, compatibility and object layout.
//! C99: §6.2.5, pp. 33-37; PDF pp. 45-49; §6.2.7, pp. 40-41;
//! PDF pp. 52-53; §6.7.2-§6.7.6, pp. 99-124; PDF pp. 111-136.
//! Expression typing and initializer completion belong to later stages.

use std::cell::Cell;

use rustc_hash::FxBuildHasher;

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

/// Canonical unqualified type identity plus its compact qualifier set.
/// C99: §6.2.5p26, p. 37; PDF p. 49; §6.7.3, pp. 108-109; PDF pp. 120-121.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TypeId {
    pub(crate) index:      usize,
    pub(crate) qualifiers: TypeQualifiers,
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

pub(crate) use crate::target::{
    Layout,
    Scalar,
    TargetLayout,
};

/// Array completeness and run-time extent; `Star` is prototype-scope `[*]`.
/// C99: §6.7.5.2, pp. 116-118; PDF pp. 128-130.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ArrayBound {
    Incomplete,
    Constant(u64),
    Variable,
    Star,
}

/// Function parameter name and declared (adjusted) type.
/// C99: §6.7.5.3p7-8, p. 119; PDF p. 131.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Parameter {
    pub(crate) name:          Option<super::Identifier>,
    pub(crate) ty:            TypeId,
    pub(crate) array_minimum: Option<ArrayBound>,
}

/// Nominal tag kind. Members and labels never share the ordinary namespace.
/// C99: §6.2.3, p. 31; PDF p. 43; §6.7.2.3, pp. 106-107; PDF pp. 118-119.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TagKind {
    Struct,
    Union,
    Enum,
}

/// A record member including ABI byte/bit offset and source bit-field width.
/// C99: §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Member {
    pub(crate) name:       Option<super::Identifier>,
    pub(crate) ty:         TypeId,
    pub(crate) offset:     u64,
    pub(crate) bit_offset: u32,
    pub(crate) width:      Option<u32>,
}

/// Retained nominal type; completion mutates only its member/layout cells.
/// C99: §6.7.2.3p4-8, pp. 106-107; PDF pp. 118-119.
#[derive(Debug)]
pub(crate) struct Tag<'tu> {
    pub(crate) name:              Option<StringCacheId>,
    pub(crate) kind:              TagKind,
    pub(crate) members:           Cell<&'tu [Member]>,
    pub(crate) layout:            Cell<Option<Layout>>,
    pub(crate) complete:          Cell<bool>,
    pub(crate) tainted:           Cell<bool>,
    pub(crate) contains_flexible: Cell<bool>,
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
}

/// Durable type graph, retained after semantic working storage is released.
#[derive(Debug)]
pub(crate) struct Types<'tu> {
    pub(crate) nodes:  &'tu [TypeKind<'tu>],
    pub(crate) tags:   &'tu [&'tu Tag<'tu>],
    pub(crate) target: TargetLayout,
}

impl Types<'_> {
    /// Iteratively strips array derivations and multiplies their extents.
    /// C99: §6.5.3.4p2-4, p. 80; PDF p. 92.
    pub(crate) fn layout(&self, mut ty: TypeId) -> Option<Layout> {
        let mut count = 1_u64;
        let layout = loop {
            match self.nodes[ty.index] {
                | TypeKind::Scalar(scalar) => break self.target.scalar(scalar)?,
                | TypeKind::Pointer(_) => break self.target.pointer,
                | TypeKind::Tag(id) => break self.tags[id].layout.get()?,
                | TypeKind::Array(element, ArrayBound::Constant(n)) => {
                    count = count.checked_mul(n)?;
                    ty = element;
                },
                | _ => return None,
            }
        };
        Some(Layout {
            size: layout.size.checked_mul(count)?,
            ..layout
        })
    }
}

/// Working hash-cons table; only its immutable graph escapes into `'tu`.
pub(crate) struct TypeInterner<'tu, 's> {
    pub(crate) nodes:  ArenaVec<'tu, TypeKind<'tu>>,
    pub(crate) tags:   ArenaVec<'tu, &'tu Tag<'tu>>,
    keys:              ArenaMap<'s, TypeKind<'tu>, TypeId>,
    pub(crate) target: TargetLayout,
    pub(crate) tu:     &'tu Bump,
    scratch:           &'s Bump,
}

impl<'tu, 's> TypeInterner<'tu, 's> {
    pub(crate) fn new(tu: &'tu Bump, scratch: &'s Bump) -> Self {
        let mut result = Self {
            nodes: ArenaVec::new_in(tu),
            tags: ArenaVec::new_in(tu),
            keys: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            target: TargetLayout::LP64,
            tu,
            scratch,
        };
        _ = result.intern(TypeKind::Unknown);
        result
    }

    pub(crate) fn intern(&mut self, kind: TypeKind<'tu>) -> TypeId {
        if let Some(&ty) = self.keys.get(&kind) {
            return ty;
        }
        let ty = TypeId {
            index:      self.nodes.len(),
            qualifiers: TypeQualifiers::empty(),
        };
        self.nodes.push(kind);
        _ = self.keys.insert(kind, ty);
        ty
    }

    pub(crate) fn scalar(&mut self, scalar: Scalar) -> TypeId {
        self.intern(TypeKind::Scalar(scalar))
    }

    pub(crate) fn unknown(&self) -> TypeId {
        self.keys[&TypeKind::Unknown]
    }

    /// Unmodeled extension layout suppresses dependent constraints.
    pub(crate) fn unanalyzed(&self, mut ty: TypeId) -> bool {
        loop {
            match self.nodes[ty.index] {
                | TypeKind::Unknown => return true,
                | TypeKind::Tag(id) => return self.tags[id].tainted.get(),
                | TypeKind::Array(element, _) => ty = element,
                | _ => return false,
            }
        }
    }

    /// C99: §6.5.3.4p2-4, p. 80; PDF p. 92.
    pub(crate) fn layout(&self, mut ty: TypeId) -> Option<Layout> {
        let mut count = 1_u64;
        let layout = loop {
            match self.nodes[ty.index] {
                | TypeKind::Scalar(s) => break self.target.scalar(s)?,
                | TypeKind::Pointer(_) => break self.target.pointer,
                | TypeKind::Tag(id) => break self.tags[id].layout.get()?,
                | TypeKind::Array(element, ArrayBound::Constant(n)) => {
                    count = count.checked_mul(n)?;
                    ty = element;
                },
                | _ => return None,
            }
        };
        Some(Layout {
            size: layout.size.checked_mul(count)?,
            ..layout
        })
    }

    pub(crate) fn finish(self) -> Types<'tu> {
        Types {
            nodes:  self.nodes.leak(),
            tags:   self.tags.leak(),
            target: self.target,
        }
    }

    /// Compatibility and composite construction use an explicit postorder
    /// stack. C99: §6.2.7p1-4, pp. 40-41; PDF pp. 52-53; §6.7.5.1p2, p.
    /// 115; PDF p. 127; §6.7.5.2p6, p. 117; PDF p. 129; §6.7.5.3p15, p.
    /// 119; PDF p. 131.
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
                        | (TypeKind::Tag(id), TypeKind::Scalar(Scalar::Int))
                        | (TypeKind::Scalar(Scalar::Int), TypeKind::Tag(id))
                            if self.tags[id].kind == TagKind::Enum =>
                        {
                            values.push(if matches!(ak, TypeKind::Tag(_)) { a } else { b });
                        },
                        | (TypeKind::Pointer(x), TypeKind::Pointer(y)) => {
                            work.push(Work::Pointer(a.qualifiers));
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

pub(crate) fn align_up(value: u64, align: u64) -> Option<u64> {
    value.checked_add(align - 1).map(|n| n & !(align - 1))
}
