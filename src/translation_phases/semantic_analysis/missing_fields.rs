//! GCC's and Clang's `-Wmissing-field-initializers`: a braced initializer
//! that names some of a structure's members and leaves the rest zero, as
//! C99 §6.7.8p21 defines. Both compilers exempt `{ 0 }`, `{ }` and a list with
//! designators; GCC's exemption extends to the lists nested in a designated
//! element. A list that elides inner braces is left alone at its level: the
//! walk here reads the syntax against the types and does not model elision,
//! preferring a missing warning to a false one.
//! C99: §6.7.8p21, p. 127; PDF p. 139.

use super::{
    Analyzer,
    SemanticErrorKind,
    expressions::ValueCategory,
    types::{
        ArrayBound,
        Member,
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticFormat,
        DiagnosticGroup,
    },
    diagnostics::format_in,
    translation_phases::parsing::{
        declaration_syntax::{
            BracedInitializerList,
            DesignatorType,
            InitializerElement,
            InitializerType,
        },
        syntax::{
            Expression,
            ExpressionType as E,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Walks a whole initializer's braced lists; each warning points at its
    /// list's closing brace.
    /// C99: §6.7.8p21, p. 127; PDF p. 139.
    pub(super) fn check_missing_fields(
        &mut self,
        list: &'tu BracedInitializerList<'tu>,
        ty: TypeId,
    ) {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((list, ty, false));
        while let Some((list, ty, designated)) = work.pop() {
            match self.types.nodes[self.types.non_atomic(ty).index] {
                | TypeKind::Array(element, _) => {
                    let designators = list.elements.iter().any(|e| e.designation.is_some());
                    for item in list.elements {
                        match item.initializer.kind {
                            | InitializerType::InitializerList(inner) =>
                                work.push((inner, element, designated || designators)),
                            | InitializerType::AssignmentExpression(_) =>
                                if self.aggregate(element) {
                                    break;
                                },
                        }
                    }
                },
                | TypeKind::Tag(id) => {
                    let tag = self.types.tags[id];
                    let members = tag.members.get();
                    if tag.kind == TagKind::Union {
                        Self::walk_union(list, members, designated, &mut work);
                    } else if tag.kind == TagKind::Struct {
                        self.walk_struct(list, ty, members, designated, &mut work);
                    }
                },
                | _ => {},
            }
        }
    }

    fn walk_union(
        list: &'tu BracedInitializerList<'tu>,
        members: &'tu [Member],
        designated: bool,
        work: &mut ArenaVec<'_, (&'tu BracedInitializerList<'tu>, TypeId, bool)>,
    ) {
        let Some(item) = list.elements.as_slice().first() else {
            return;
        };
        let member = match Self::named_member(item, members) {
            | Some(found) => found.map(|index| &members[index]),
            | None => members.iter().find(|member| Self::initializable(member)),
        };
        if let (Some(member), InitializerType::InitializerList(inner)) =
            (member, item.initializer.kind)
        {
            work.push((inner, member.ty, designated || item.designation.is_some()));
        }
    }

    fn walk_struct(
        &mut self,
        list: &'tu BracedInitializerList<'tu>,
        ty: TypeId,
        members: &'tu [Member],
        designated: bool,
        work: &mut ArenaVec<'_, (&'tu BracedInitializerList<'tu>, TypeId, bool)>,
    ) {
        let mut designators = false;
        let mut next = 0;
        for item in list.elements {
            match Self::named_member(item, members) {
                | Some(Some(index)) => {
                    designators = true;
                    next = index;
                },
                // A nested or an array designator: the rest of this list is
                // not read.
                | Some(None) => return,
                | None => {},
            }
            while members
                .get(next)
                .is_some_and(|member| !Self::initializable(member))
            {
                next += 1;
            }
            let Some(member) = members.get(next) else {
                return;
            };
            match item.initializer.kind {
                | InitializerType::InitializerList(inner) =>
                    work.push((inner, member.ty, designated || designators)),
                | InitializerType::AssignmentExpression(e) =>
                    if self.aggregate(member.ty) && !self.initializes_whole(e) {
                        return;
                    },
            }
            next += 1;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        if designators || (gcc && designated) || list.elements.is_empty() || self.zero_idiom(list) {
            return;
        }
        let Some(member) = members[next.min(members.len())..]
            .iter()
            .find(|member| self.needs_initializer(member))
        else {
            return;
        };
        let member = *member;
        let closing = list
            .closing_brace_source_vectors
            .or(list.opening_brace_source_vectors);
        if let Some(place) = closing {
            let name = self.missing_field_wording(ty, &member);
            self.spelled_group_diagnostic(
                DiagnosticGroup::MissingFieldInitializers,
                SemanticErrorKind::MissingFieldInitializer,
                place,
                Some(name),
            );
        }
    }

    /// The member a lone `.name` designator names, `Some(None)` for any other
    /// designator, and `None` for an element without one.
    #[expect(
        clippy::option_option,
        reason = "No designator and an unreadable one are different answers."
    )]
    fn named_member(item: &InitializerElement<'tu>, members: &[Member]) -> Option<Option<usize>> {
        let designation = item.designation?;
        let designators = designation.designators.as_slice();
        if let [designator] = designators
            && let DesignatorType::Field(name) | DesignatorType::GnuField(name) = designator.kind
        {
            return Some(
                members
                    .iter()
                    .position(|member| member.name.is_some_and(|m| m.name == name.name)),
            );
        }
        Some(None)
    }

    /// Whether an initializer list's element can land on `member`: an unnamed
    /// bit-field takes none. C99: §6.7.8p9, p. 126; PDF p. 138.
    const fn initializable(member: &Member) -> bool {
        member.name.is_some() || member.anonymous
    }

    /// Whether leaving `member` out deserves the warning: not a flexible or
    /// zero-length array member, which holds nothing to initialize.
    fn needs_initializer(&self, member: &Member) -> bool {
        Self::initializable(member)
            && !matches!(
                self.types.nodes[member.ty.index],
                TypeKind::Array(_, ArrayBound::Incomplete | ArrayBound::Constant(0))
            )
    }

    /// Whether the expression initializes an aggregate whole, as a string
    /// does a character array and a structure value its member: otherwise
    /// the initializer elides braces.
    fn initializes_whole(&self, e: &'tu Expression<'tu>) -> bool {
        let mut e = e;
        while let E::Parenthesized { expression } = e.kind {
            e = expression;
        }
        if matches!(e.kind, E::StringLiteral(_)) {
            return true;
        }
        let info = self.expression_info(e);
        info.category != ValueCategory::Rvalue
            && matches!(self.types.nodes[info.ty.index], TypeKind::Tag(_))
    }

    /// `{ 0 }`: Clang wants the literal, and GCC any constant zero, a null
    /// pointer's cast included.
    fn zero_idiom(&self, list: &'tu BracedInitializerList<'tu>) -> bool {
        let [item] = list.elements.as_slice() else {
            return false;
        };
        let InitializerType::AssignmentExpression(mut e) = item.initializer.kind else {
            return false;
        };
        if item.designation.is_some() {
            return false;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        loop {
            match e.kind {
                | E::Parenthesized { expression } => e = expression,
                | E::Cast {
                    operand_expression, ..
                } if gcc => e = operand_expression,
                | _ => break,
            }
        }
        if !gcc && !matches!(e.kind, E::Constant(_)) {
            return false;
        }
        self.expression_info(e)
            .integer
            .is_some_and(super::Integer::is_zero)
    }

    /// Each compiler's words, for `ty`'s `member`.
    fn missing_field_wording(
        &mut self,
        ty: TypeId,
        member: &Member,
    ) -> crate::util::string_cache::StringCacheId {
        let field = member.name.map_or("({anonymous})", |name| {
            self.context.string_cache.at(name.name)
        });
        let text = if self.context.configuration.diagnostic_format() == DiagnosticFormat::Gcc {
            let named = match self.types.nodes[ty.index] {
                | TypeKind::Tag(id) => self
                    .function_extensions
                    .typedef_names
                    .get(&id)
                    .map(|&name| self.context.string_cache.at(name)),
                | _ => None,
            };
            let record = named
                .or_else(|| self.type_spelling(ty.unqualified(), true))
                .unwrap_or("struct <anonymous>");
            format_in!(
                self.scratch,
                "missing initializer for field '{field}' of '{record}'"
            )
        } else {
            format_in!(self.scratch, "missing field '{field}' initializer")
        };
        self.context.string_cache.intern(text)
    }
}
