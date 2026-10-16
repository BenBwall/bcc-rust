//! What bcc does with each attribute name. Every name bundled Clang knows
//! for C on x86-64 targets has a rule; other names are unknown and warned
//! about, as Clang's `-Wunknown-attributes` does. A rule says whether the
//! attribute's meaning is implemented, has no code-generation meaning to
//! honor, is dropped with a warning because dropping it cannot change a
//! program's behavior, or is refused because dropping it would.
//! `language-standards.md` ("Attribute status") documents the same table.
//! GNU extension: GCC manual, "Attribute Syntax" and the attribute pages.
//! <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
//! Clang extension: Clang Attributes Reference.
//! <https://clang.llvm.org/docs/AttributeReference.html>
//! C23: §6.7.13.2p1-3, pp. 142-143; PDF pp. 155-156 (standard attributes
//! and implementation-defined vendor prefixes).

use super::super::UnsupportedAttribute;
use crate::translation_phases::parsing::AttributeSyntax;

/// The treatment of one attribute name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::translation_phases::semantic_analysis) enum Rule {
    /// Layout and type attributes: semantic type construction owns them.
    Representation,
    /// GNU `aligned`: a representation attribute, and the code alignment of
    /// a function.
    Aligned,
    /// GNU `transparent_union`: type construction makes a qualifying union
    /// transparent (`transparent.rs`), or warns that it ignores the
    /// attribute.
    TransparentUnion,
    Cleanup,
    Constructor,
    Destructor,
    Alias,
    Weak,
    Section,
    Used,
    Visibility,
    /// GNU `returns_twice`: an entity fact that lowering projects onto the
    /// IR function, as the external-symbol contract does for `setjmp`.
    ReturnsTwice,
    /// A calling convention, checked against the target's default.
    Convention(Convention),
    /// Implemented elsewhere or without a code-generation meaning that bcc
    /// must honor: hints, diagnostics and optimization promises.
    Accepted,
    /// Bcc drops its code-generation effect; dropping it cannot change the
    /// behavior of a program linked into one executable, so this is a
    /// warning, as Clang's `-Wignored-attributes`.
    Ignored,
    /// Dropping its meaning could change behavior: lowering refuses it.
    Unsupported(UnsupportedAttribute),
    /// `ms_struct` or `gcc_struct`: accepted when it names the target's
    /// own record layout.
    RecordLayout {
        microsoft: bool,
    },
}

/// Calling-convention attributes. `Default` names the C convention x86-64
/// already uses (bundled Clang ignores `cdecl` and `regparm` there);
/// `X86` names a 32-bit convention that Clang ignores on x86-64 with a
/// warning outside Windows; `Microsoft` and `SystemV` select an x86-64
/// convention explicitly; `Other` is never the C default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::translation_phases::semantic_analysis) enum Convention {
    Default,
    X86,
    /// `pascal`, which Clang ignores with a warning on every x86-64 target.
    Pascal,
    Microsoft,
    SystemV,
    Other,
}

/// The rule for `name` in `namespace`, or `None` when the name is unknown.
pub(in crate::translation_phases::semantic_analysis) fn rule(
    syntax: AttributeSyntax,
    namespace: Option<&str>,
    name: &str,
) -> Option<Rule> {
    match (syntax, namespace) {
        | (AttributeSyntax::Standard, None) => standard(name),
        | (AttributeSyntax::Msvc, None) => declspec(name),
        | (_, None | Some("gnu" | "clang")) => gnu(name),
        | _ => None,
    }
}

/// C23 standard attributes have no code-generation meaning to honor.
/// C23: §6.7.13.1-§6.7.13.8, pp. 142-150; PDF pp. 155-163.
fn standard(name: &str) -> Option<Rule> {
    matches!(
        name,
        "deprecated"
            | "fallthrough"
            | "maybe_unused"
            | "nodiscard"
            | "noreturn"
            | "Noreturn"
            | "unsequenced"
            | "reproducible"
    )
    .then_some(Rule::Accepted)
}

/// MSVC `__declspec` modifiers.
/// MSVC extension: Microsoft Learn, "declspec".
/// <https://learn.microsoft.com/en-us/cpp/cpp/declspec>
fn declspec(name: &str) -> Option<Rule> {
    Some(match name {
        | "align" => Rule::Representation,
        | "allocate" => Rule::Section,
        | "naked" => Rule::Unsupported(UnsupportedAttribute::Naked),
        | "dllimport"
        | "dllexport"
        | "noreturn"
        | "nothrow"
        | "selectany"
        | "noinline"
        | "deprecated"
        | "restrict"
        | "noalias"
        | "novtable"
        | "uuid"
        | "property"
        | "safebuffers"
        | "code_seg"
        | "allocator"
        | "empty_bases"
        | "jitintrinsic"
        | "process"
        | "appdomain"
        | "guard"
        | "spectre"
        | "intrin_type"
        | "no_sanitize_address" => Rule::Accepted,
        | _ => return gnu(name),
    })
}

/// GNU and Clang names, including Clang's `[[gnu::...]]`/`[[clang::...]]`.
fn gnu(name: &str) -> Option<Rule> {
    use UnsupportedAttribute as U;
    Some(match name {
        | "packed" | "mode" | "vector_size" | "ext_vector_type" | "matrix_type" | "align" =>
            Rule::Representation,
        | "aligned" => Rule::Aligned,
        | "cleanup" => Rule::Cleanup,
        | "constructor" => Rule::Constructor,
        | "destructor" => Rule::Destructor,
        | "alias" => Rule::Alias,
        | "weak" => Rule::Weak,
        | "section" => Rule::Section,
        | "used" => Rule::Used,
        | "visibility" => Rule::Visibility,
        | "cdecl" | "regparm" => Rule::Convention(Convention::Default),
        | "stdcall" | "fastcall" | "thiscall" => Rule::Convention(Convention::X86),
        | "pascal" => Rule::Convention(Convention::Pascal),
        | "ms_abi" => Rule::Convention(Convention::Microsoft),
        | "sysv_abi" => Rule::Convention(Convention::SystemV),
        | "vectorcall"
        | "regcall"
        | "preserve_most"
        | "preserve_all"
        | "preserve_none"
        | "swiftcall"
        | "swiftasynccall"
        | "intel_ocl_bicc"
        | "pcs"
        | "aarch64_vector_pcs"
        | "aarch64_sve_pcs"
        | "riscv_vector_cc"
        | "no_caller_saved_registers"
        | "no_callee_saved_registers"
        | "interrupt" => Rule::Convention(Convention::Other),
        | "ms_struct" => Rule::RecordLayout { microsoft: true },
        | "gcc_struct" => Rule::RecordLayout { microsoft: false },
        | "ifunc" => Rule::Unsupported(U::Ifunc),
        | "weakref" => Rule::Unsupported(U::Weakref),
        | "naked" => Rule::Unsupported(U::Naked),
        | "returns_twice" => Rule::ReturnsTwice,
        | "overloadable" => Rule::Unsupported(U::Overloadable),
        | "cpu_dispatch" | "cpu_specific" | "target_version" =>
            Rule::Unsupported(U::Multiversioning),
        | "pass_object_size" | "pass_dynamic_object_size" => Rule::Unsupported(U::PassObjectSize),
        | "transparent_union" => Rule::TransparentUnion,
        // Dropped with a warning: link-time or multi-version effects that a
        // single executable's behavior does not depend on.
        | "common" | "target" | "target_clones" | "weak_import" | "copy" => Rule::Ignored,
        | "access"
        | "alloc_align"
        | "alloc_size"
        | "always_inline"
        | "annotate"
        | "annotate_type"
        | "artificial"
        | "assume"
        | "assume_aligned"
        | "availability"
        | "btf_decl_tag"
        | "btf_type_tag"
        | "callback"
        | "capability"
        | "cf_check"
        | "cold"
        | "const"
        | "consumable"
        | "convergent"
        | "counted_by"
        | "counted_by_or_null"
        | "deprecated"
        | "designated_init"
        | "diagnose_if"
        | "disable_tail_calls"
        | "dllexport"
        | "dllimport"
        | "enable_if"
        | "enforce_tcb"
        | "enforce_tcb_leaf"
        | "enum_extensibility"
        | "error"
        | "warning"
        | "exclude_from_explicit_instantiation"
        | "externally_visible"
        | "fallthrough"
        | "flag_enum"
        | "flatten"
        | "format"
        | "format_arg"
        | "force_align_arg_pointer"
        | "function_return"
        | "gnu_inline"
        | "guarded_by"
        | "pt_guarded_by"
        | "acquired_after"
        | "acquired_before"
        | "requires_capability"
        | "requires_shared_capability"
        | "acquire_capability"
        | "acquire_shared_capability"
        | "release_capability"
        | "release_shared_capability"
        | "release_generic_capability"
        | "try_acquire_capability"
        | "try_acquire_shared_capability"
        | "exclusive_locks_required"
        | "shared_locks_required"
        | "locks_excluded"
        | "lock_returned"
        | "assert_capability"
        | "assert_shared_capability"
        | "no_thread_safety_analysis"
        | "lockable"
        | "scoped_lockable"
        // MSVC `__declspec(thread)`, also in the GNU spelling MinGW's
        // `__declspec` macro produces: declaration analysis gives the object
        // thread storage duration (`Analyzer::thread_storage`).
        | "thread"
        | "hot"
        | "indirect_branch"
        | "indirect_return"
        | "leaf"
        | "likely"
        | "unlikely"
        | "malloc"
        | "maybe_unused"
        | "may_alias"
        | "min_vector_width"
        | "minsize"
        | "ms_hook_prologue"
        | "musttail"
        | "no_builtin"
        | "no_icf"
        | "no_instrument_function"
        | "no_profile_instrument_function"
        | "no_reorder"
        | "no_sanitize"
        | "no_sanitize_address"
        | "no_address_safety_analysis"
        | "no_sanitize_thread"
        | "no_sanitize_undefined"
        | "no_sanitize_memory"
        | "no_sanitize_coverage"
        | "no_split_stack"
        | "no_stack_limit"
        | "no_stack_protector"
        | "no_speculative_load_hardening"
        | "noclone"
        | "nocommon"
        | "nocf_check"
        | "nodebug"
        | "nodiscard"
        | "noduplicate"
        | "noescape"
        | "noinline"
        | "noipa"
        | "nomerge"
        | "nonnull"
        | "nonstring"
        | "noplt"
        | "noreturn"
        | "nothrow"
        | "not_tail_called"
        | "nouwtable"
        | "null_terminated_string_arg"
        | "optimize"
        | "optnone"
        | "patchable_function_entry"
        | "preserve_access_index"
        | "preserve_static_offset"
        | "pure"
        | "randomize_layout"
        | "no_randomize_layout"
        | "reproducible"
        | "unsequenced"
        | "retain"
        | "returns_nonnull"
        | "selectany"
        | "sentinel"
        | "simd"
        | "sized_by"
        | "sized_by_or_null"
        | "speculative_load_hardening"
        | "stack_protect"
        | "standalone_debug"
        | "strict_flex_array"
        | "tls_model"
        | "uninitialized"
        | "unused"
        | "warn_if_not_aligned"
        | "warn_unused"
        | "warn_unused_result"
        | "xray_always_instrument"
        | "xray_never_instrument"
        | "xray_log_args"
        | "zero_call_used_regs"
        | "fentry_name"
        | "nonblocking"
        | "nonallocating"
        | "blocking"
        | "allocating"
        | "noderef"
        | "lifetimebound"
        | "unavailable" => Rule::Accepted,
        | _ => return None,
    })
}
