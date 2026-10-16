//! Entity and symbol facts from validated GNU attributes: weak linkage,
//! aliases, sections, retention, function code alignment, constructors and
//! destructors, and the attributes whose meaning code generation cannot
//! honor. Semantic analysis validates each attribute once; lowering reads
//! these records and never parses attribute tokens.
//! GNU extension: GCC manual, "Common Function Attributes" and "Common
//! Variable Attributes".
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Variable-Attributes.html>
//! C99: §6.2.2p2, p. 30; PDF p. 42 (every declaration of a linked
//! identifier denotes one entity, so their attributes merge).

use super::correspondence::EntityId;
use crate::{
    translation_phases::SourceVectors,
    util::bump::ArenaMap,
};

/// Retained entity attributes, constructor/destructor registrations and
/// capability refusals.
#[derive(Debug)]
pub(crate) struct SymbolFacts<'tu> {
    pub(in crate::translation_phases::semantic_analysis) entities:
        ArenaMap<'tu, EntityId, EntityAttributes<'tu>>,
    /// Constructor and destructor definitions in definition order.
    pub(crate) init_functions: &'tu [InitFunctionFacts],
    /// Every alias declaration, resolved to the definition it names.
    pub(crate) aliases: &'tu [AliasFacts],
    /// Accepted attributes and conventions whose meaning code generation
    /// does not implement; lowering refuses each one at its provenance.
    pub(crate) refusals: &'tu [AttributeRefusal],
}

impl<'tu> SymbolFacts<'tu> {
    /// The merged attributes of an entity, if any declaration had one.
    pub(crate) fn entity(&self, entity: EntityId) -> Option<&EntityAttributes<'tu>> {
        self.entities.get(&entity)
    }
}

/// Attributes merged over every declaration of one entity.
#[derive(Debug, Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent attribute fact that lowering reads on its own."
)]
pub(crate) struct EntityAttributes<'tu> {
    /// GNU `weak`: a definition yields to a strong one at link time; an
    /// undefined declaration may stay undefined, with a null address.
    pub(crate) weak:               bool,
    /// GNU `section("name")`, from the declaration that wrote it.
    pub(crate) section:            Option<&'tu str>,
    /// GNU `used`: emit the definition even if nothing references it.
    pub(crate) used:               bool,
    /// GNU `aligned` on a function: the alignment of its code, in bytes.
    pub(crate) code_alignment:     u32,
    /// GNU `visibility`, from the declaration that wrote it.
    pub(crate) visibility:         Option<Visibility>,
    /// Calls may return a second time, after a `longjmp`: GNU
    /// `returns_twice`, or a linked function that the external-symbol
    /// contract (`builtins::external_contract`) lists, as `_setjmp` is.
    /// Lowering marks the IR function, which the optimizer and back ends
    /// read. C99: §7.13.2.1 paragraph 4, p. 244; PDF p. 256.
    pub(crate) returns_twice:      bool,
    /// GNU `nonnull`: zero-based parameter positions, or None when absent.
    /// An empty slice applies to every pointer parameter. These facts are
    /// diagnostic only.
    pub(crate) nonnull:            Option<&'tu [usize]>,
    /// A declaration of the object specifies its alignment, by GNU
    /// `aligned` or `_Alignas`, rather than a typedef: that alignment is the
    /// object's own, which the System V rule for large arrays does not raise
    /// (as in Clang, `ASTContext::getDeclAlign`).
    /// C11: §6.7.5p6, p. 127; PDF p. 145.
    pub(crate) declared_alignment: bool,
    /// GNU `noinline`/`noipa` or `always_inline` on a function.
    pub(crate) inlining:           Inlining,
    /// Microsoft `selectany` on an external object: its definitions in
    /// different translation units are one, and the linker keeps any.
    pub(crate) select_any:     bool,
}

/// A function's inline-control request. `Never` wins over `Always` when
/// declarations write both.
/// GNU extension: GCC manual, "Common Function Attributes", noinline,
/// noipa, `always_inline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Inlining {
    #[default]
    Default,
    Never,
    Always,
}

/// ELF symbol visibility; COFF targets ignore it as Clang does.
/// GNU extension: GCC manual, "Common Function Attributes", visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Visibility {
    Default,
    Hidden,
    Protected,
}

/// A declaration that GNU `alias("target")` makes another name for the
/// definition `target`. Chains of aliases are resolved to the final
/// definition.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AliasFacts {
    pub(crate) binding: usize,
    pub(crate) target:  usize,
    pub(crate) source:  SourceVectors,
}

/// A function to run before (`Constructor`) or after (`Destructor`) the
/// program's entry, with GCC's default priority 65535 when none is written.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InitFunctionFacts {
    pub(crate) kind:     InitFunctionKind,
    pub(crate) priority: u16,
    /// The function definition's binding.
    pub(crate) binding:  usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitFunctionKind {
    Constructor,
    Destructor,
}

/// An attribute that semantic analysis accepted but whose meaning bcc's
/// code generation does not implement. Dropping it would change behavior,
/// so lowering refuses the translation unit at `source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AttributeRefusal {
    pub(crate) capability: UnsupportedAttribute,
    pub(crate) source:     SourceVectors,
    /// The function definition whose body wrote it, or None outside any
    /// body. Lowering reports a refusal only when the module emits its
    /// owner, so an unreachable body refuses nothing.
    pub(crate) owner:      Option<usize>,
}

/// The unsupported attribute meanings, each named in lowering diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnsupportedAttribute {
    /// A calling convention other than the target's default C convention.
    CallingConvention,
    /// GNU `ifunc`: a symbol resolved by a function at load time.
    Ifunc,
    /// GNU `weakref`: a weak reference to another symbol.
    Weakref,
    /// GNU/MSVC `naked`: a body without prologue or epilogue.
    Naked,
    /// Clang `cpu_dispatch`/`cpu_specific`/`target_version` multiversioning.
    Multiversioning,
    /// `ms_struct` or `gcc_struct` naming the other target's record layout.
    RecordLayout,
    /// A constructor or destructor function that takes parameters.
    InitFunctionParameters,
    /// A section name that cannot be carried to the object file.
    SectionName,
    /// An alias whose target or alias name carries a GNU assembler label.
    AliasAssemblerLabel,
    /// An alias whose thread storage duration differs from its target's.
    AliasThreadLocal,
    /// Clang `address_space`: on x86-64, address spaces 256-258 address
    /// memory through the `gs`, `fs` and `ss` segments.
    AddressSpace,
    /// Clang's Swift parameter attributes, such as `swift_async_context`,
    /// which passes its parameter in `r14` under the ordinary C convention.
    SwiftParameter,
    /// Clang `internal_linkage`: an external declaration given internal
    /// linkage.
    InternalLinkage,
}

impl UnsupportedAttribute {
    /// The construct as a lowering diagnostic names it.
    pub(crate) const fn description(self) -> &'static str {
        match self {
            | Self::CallingConvention => "calling conventions",
            | Self::Ifunc => "the `ifunc` attribute",
            | Self::Weakref => "the `weakref` attribute",
            | Self::Naked => "naked functions",
            | Self::Multiversioning => "function multiversioning attributes",
            | Self::RecordLayout => "this record layout attribute",
            | Self::InitFunctionParameters => "constructors and destructors with parameters",
            | Self::SectionName => "this section name",
            | Self::AliasAssemblerLabel => "aliases involving assembler labels",
            | Self::AliasThreadLocal => "aliases between thread-local and other objects",
            | Self::AddressSpace => "the `address_space` attribute",
            | Self::SwiftParameter => "Swift parameter attributes such as `swift_async_context`",
            | Self::InternalLinkage => "the `internal_linkage` attribute",
        }
    }
}
