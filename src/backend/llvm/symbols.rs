//! Module symbols as LLVM sees them: functions and globals with their
//! linkage, COMDAT, alignment, section, visibility and DLL storage,
//! aliases created once their targets exist, and the special
//! `llvm.global_ctors`, `llvm.global_dtors` and `llvm.compiler.used` (ELF) or
//! `llvm.used` (COFF) arrays, which match bundled Clang's output for the GNU
//! attributes. GNU extension: GCC manual, "Common Function Attributes" and
//! "Common Variable Attributes" (alias, constructor, destructor, section, used,
//! weak). <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>

use super::{
    EmitError,
    api::{
        Linkage,
        Session,
        Ty,
        Val,
    },
    function,
    global,
};
use crate::{
    configuration::LibraryBuiltins,
    diagnostics::format_in,
    ir::{
        DllStorage,
        Entity,
        InitKind,
        Linkage as IrLinkage,
        Module,
        Symbol,
        SymbolAttributes,
        Type,
        Visibility,
    },
    target::Target,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// LLVM's attribute index of the function itself.
const FUNCTION_INDEX: u32 = u32::MAX;

/// Declares every function and global, then every alias, returning their
/// values indexed by `FuncId` and `GlobalId`.
pub(super) fn declare<'id, 'a, 'm>(
    module: &'m Module<'_>,
    api: &Session<'id>,
    platform: Target,
    strict: bool,
    arena: &'a Bump,
) -> Result<(ArenaVec<'a, Val<'id>>, ArenaVec<'a, Val<'id>>), EmitError<'m>> {
    let mut functions = ArenaVec::new_in(arena);
    for (id, function) in module.functions() {
        let signature = module.signature(function.signature);
        let ty = function::signature_type(api, signature, arena);
        if function.attributes.alias.is_some()
            || module.inline_copy_target(id).is_some()
            || (function.inline_copy_of.is_some() && function.body.is_none())
        {
            // Placeholder until the alias or the external function exists;
            // nothing reads it before. A plain inline copy whose body did
            // not lower is never named.
            functions.push(api.null(api.ty(Type::Ptr)));
            continue;
        }
        let value = api.function(api.name(function.name), ty);
        api.add_attributes(value, &function::attributes(api, signature, arena), false);
        if module.returns_twice(id) {
            function::returns_twice(api, value, false);
        }
        // GNU `noinline`/`noipa` and `always_inline`, as Clang marks them.
        if let Some(keyword) = function.inlining.keyword() {
            api.attribute(value, FUNCTION_INDEX, keyword, 0, None, false);
        }
        if let Some(linkage) = linkage(function.linkage, function.body.is_some()) {
            api.linkage(value, linkage);
        }
        if function.linkage.comdat() && function.body.is_some() {
            api.own_comdat(value, api.name(function.name));
        }
        if let Some(align) = function.align {
            api.alignment(value, align.bytes());
        }
        if function.body.is_some() {
            library_builtins(api, value, module.library_builtins(), arena);
            null_pointer_validity(api, value, module);
            floating_point(api, value, module, strict);
        } else {
            math_without_errno(api, value, module, function.name);
        }
        sections(api, value, function.attributes);
        functions.push(value);
    }
    // A private inline copy becomes its external function's
    // `available_externally` body, and its uses name that function.
    for (id, function) in module.functions() {
        if let Some(external) = module.inline_copy_target(id) {
            let value = functions[external.index()];
            api.linkage(value, Linkage::AvailableExternally);
            // The copy's body becomes the external function's, which was
            // declared without one, so it takes the builtin controls here.
            library_builtins(api, value, module.library_builtins(), arena);
            null_pointer_validity(api, value, module);
            floating_point(api, value, module, strict);
            if let Some(keyword) = function.inlining.keyword() {
                api.attribute(value, FUNCTION_INDEX, keyword, 0, None, false);
            }
            functions[id.index()] = value;
        }
    }
    let mut globals = ArenaVec::new_in(arena);
    // Declare every global before constructing any relocated initializer.
    for (_, global) in module.globals() {
        let ty = global::global_type(api, global, arena)?;
        if global.attributes.alias.is_some() {
            globals.push(api.null(api.ty(Type::Ptr)));
            continue;
        }
        let value = api.global(api.name(global.name), ty);
        api.alignment(value, global.align.bytes());
        api.constant_global(value, global.constant);
        if let Some(linkage) = linkage(global.linkage, global.init.is_some()) {
            api.linkage(value, linkage);
        }
        if global.linkage.comdat() && global.init.is_some() {
            api.own_comdat(value, api.name(global.name));
        }
        if global.thread_local {
            // In an ELF executable Clang marks a definition `dso_local`, so
            // LLVM gives it the local-exec model and a declaration
            // initial-exec. LLVM-C cannot mark a global `dso_local`, so a
            // definition (weak ones included, as Clang marks them
            // `dso_local` too) requests local-exec directly. Code for a
            // shared library requests the `-ftls-model`, general-dynamic by
            // default, which LLVM refines to local-dynamic for an internal
            // or hidden global, as for Clang's.
            api.thread_local(
                value,
                api.thread_local_model(platform, global.init.is_some()),
            );
        }
        sections(api, value, global.attributes);
        globals.push(value);
    }
    // Verified alias targets are definitions, never aliases themselves.
    for (id, function) in module.functions() {
        if let Some(target) = function.attributes.alias {
            let ty = function::signature_type(api, module.signature(function.signature), arena);
            let value = alias(
                api,
                function.name,
                ty,
                target,
                function.linkage,
                &functions,
                &globals,
            );
            visibility(api, value, function.attributes);
            functions[id.index()] = value;
        }
    }
    for (id, global) in module.globals() {
        if let Some(target) = global.attributes.alias {
            let value = if global.thread_local {
                // A verified thread-local alias names a thread-local
                // definition, whose model it shares, as in Clang.
                let Symbol::Global(aliasee) = target else {
                    unreachable!("a thread-local alias names a global")
                };
                let value = api.thread_local_alias(
                    global.name,
                    global.size,
                    globals[aliasee.index()],
                    api.thread_local_model(platform, true),
                );
                alias_linkage(api, value, global.linkage);
                value
            } else {
                let ty = api.array(api.ty(Type::I8), global.size);
                alias(
                    api,
                    global.name,
                    ty,
                    target,
                    global.linkage,
                    &functions,
                    &globals,
                )
            };
            visibility(api, value, global.attributes);
            globals[id.index()] = value;
        }
    }
    Ok((functions, globals))
}

/// The LLVM linkage of a symbol, when it differs from LLVM's external
/// default. An undefined internal function stays an external declaration.
fn linkage(linkage: IrLinkage, defined: bool) -> Option<Linkage> {
    match (linkage, defined) {
        | (IrLinkage::External, _)
        | (
            IrLinkage::Internal | IrLinkage::SelectAny | IrLinkage::LinkOnce | IrLinkage::Common,
            false,
        ) => None,
        | (IrLinkage::Common, true) => Some(Linkage::Common),
        | (IrLinkage::Internal, true) => Some(Linkage::Internal),
        | (IrLinkage::Weak, true) => Some(Linkage::Weak),
        | (IrLinkage::Weak, false) => Some(Linkage::ExternWeak),
        | (IrLinkage::SelectAny, true) => Some(Linkage::WeakOdr),
        | (IrLinkage::LinkOnce, true) => Some(Linkage::LinkOnceOdr),
    }
}

fn sections<'id>(api: &Session<'id>, value: Val<'id>, attributes: SymbolAttributes<'_>) {
    if let Some(section) = attributes.section {
        api.section(value, section);
    }
    visibility(api, value, attributes);
}

/// ELF visibility and Windows DLL storage, which aliases share with the
/// definitions they name.
fn visibility<'id>(api: &Session<'id>, value: Val<'id>, attributes: SymbolAttributes<'_>) {
    match attributes.visibility {
        | Visibility::Default => {},
        | Visibility::Hidden => api.hidden(value, false),
        | Visibility::Protected => api.hidden(value, true),
    }
    match attributes.dll {
        | DllStorage::Default => {},
        | DllStorage::Import => api.dll_storage(value, false),
        | DllStorage::Export => api.dll_storage(value, true),
    }
}

fn alias<'id>(
    api: &Session<'id>,
    name: &str,
    ty: Ty<'id>,
    target: Symbol,
    linkage: IrLinkage,
    functions: &[Val<'id>],
    globals: &[Val<'id>],
) -> Val<'id> {
    let aliasee = match target {
        | Symbol::Function(func) => functions[func.index()],
        | Symbol::Global(global) => globals[global.index()],
    };
    let value = api.alias(api.name(name), ty, aliasee);
    alias_linkage(api, value, linkage);
    value
}

/// An alias is a definition, so `weak` gives it weak linkage.
fn alias_linkage<'id>(api: &Session<'id>, value: Val<'id>, linkage: IrLinkage) {
    match linkage {
        // Only a global without an alias is common.
        | IrLinkage::External | IrLinkage::Common => {},
        | IrLinkage::Internal => api.linkage(value, Linkage::Internal),
        | IrLinkage::Weak => api.linkage(value, Linkage::Weak),
        | IrLinkage::SelectAny => api.linkage(value, Linkage::WeakOdr),
        | IrLinkage::LinkOnce => api.linkage(value, Linkage::LinkOnceOdr),
    }
}

/// Emits `llvm.global_ctors` and `llvm.global_dtors` entries in
/// registration order with null associated data, and the retained symbols
/// of GNU `used`, as Clang does: on ELF in `llvm.compiler.used`, which keeps
/// them through compilation without asking the linker to keep their
/// sections (that is GNU `retain`), and on COFF in `llvm.used`.
pub(super) fn special_arrays<'id>(
    module: &Module<'_>,
    api: &Session<'id>,
    target: Target,
    arena: &Bump,
    functions: &[Val<'id>],
    globals: &[Val<'id>],
) {
    let ptr = api.ty(Type::Ptr);
    let entry = api.structure(&[api.ty(Type::I32), ptr, ptr], false);
    for (kind, name) in [
        (InitKind::Constructor, c"llvm.global_ctors"),
        (InitKind::Destructor, c"llvm.global_dtors"),
    ] {
        let mut entries = ArenaVec::new_in(arena);
        for init in module
            .init_functions()
            .iter()
            .filter(|init| init.kind == kind)
        {
            entries.push(api.const_record(&[
                api.bits(Type::I32, u128::from(init.priority)),
                functions[init.func.index()],
                api.null(ptr),
            ]));
        }
        if !entries.is_empty() {
            let array = api.global(name, api.array(entry, entries.len() as u64));
            api.linkage(array, Linkage::Appending);
            api.initializer(array, api.const_array(entry, &entries));
        }
    }
    let mut used = ArenaVec::new_in(arena);
    for (id, function) in module.functions() {
        if function.attributes.used {
            used.push(functions[id.index()]);
        }
    }
    for (id, global) in module.globals() {
        if global.attributes.used {
            used.push(globals[id.index()]);
        }
    }
    if !used.is_empty() {
        let name = match target {
            | Target::LinuxGnu | Target::LinuxMusl | Target::Wasm32Unknown | Target::Wasm32Wasi =>
                c"llvm.compiler.used",
            | Target::WindowsGnu | Target::WindowsMsvc => c"llvm.used",
        };
        let array = api.global(name, api.array(ptr, used.len() as u64));
        api.linkage(array, Linkage::Appending);
        api.initializer(array, api.const_array(ptr, &used));
        api.section(array, "llvm.metadata");
    }
}

/// Marks a function in which LLVM must not assume that a dereferenced
/// pointer is non-null, as Clang marks every function under
/// `-fno-delete-null-pointer-checks`.
fn null_pointer_validity<'id>(api: &Session<'id>, function: Val<'id>, module: &Module<'_>) {
    if module.null_pointer_is_valid() {
        api.attribute(
            function,
            FUNCTION_INDEX,
            "null_pointer_is_valid",
            0,
            None,
            false,
        );
    }
}

/// Marks a function whose floating operations keep their exceptions, as
/// Clang marks every function under `-ffp-exception-behavior=strict`; its
/// floating operations are LLVM's constrained intrinsics, and its calls
/// are marked too (`function.rs`). Otherwise marks what the program gives
/// up of IEC 60559 semantics, as Clang marks each function it defines:
/// `no-signed-zeros-fp-math` under `-fno-signed-zeros` and
/// `no-trapping-math` under `-fno-trapping-math`, which LLVM's code
/// generator reads; the operations carry the rest as fast-math flags.
fn floating_point<'id>(api: &Session<'id>, function: Val<'id>, module: &Module<'_>, strict: bool) {
    if strict {
        api.attribute(function, FUNCTION_INDEX, "strictfp", 0, None, false);
        return;
    }
    let flags = module.fast_math();
    if flags.no_signed_zeros {
        api.string_attribute(function, "no-signed-zeros-fp-math", "true");
    }
    if flags.no_trapping {
        api.string_attribute(function, "no-trapping-math", "true");
    }
}

/// Marks a declared math function that computes without effects but for
/// `errno` as reading and writing no memory under `-fno-math-errno`, as
/// Clang does (it calls the ones LLVM has intrinsics for through them), so
/// LLVM may treat a call as an operation: compute `sqrt` with an
/// instruction, hoist, vectorize or remove it. Not where the program
/// takes the function's name for its own (`-fno-builtin`, a freestanding
/// program).
fn math_without_errno<'id>(
    api: &Session<'id>,
    function: Val<'id>,
    module: &Module<'_>,
    name: &str,
) {
    if !module.fast_math().no_math_errno || !crate::builtins::computes_without_errno(name) {
        return;
    }
    let builtin = match module.library_builtins() {
        | LibraryBuiltins::All => true,
        | LibraryBuiltins::None => false,
        | LibraryBuiltins::Except(names) => !names.contains(&name),
    };
    if builtin {
        for keyword in ["nounwind", "willreturn"] {
            api.attribute(function, FUNCTION_INDEX, keyword, 0, None, false);
        }
        // `memory(none)`: LLVM encodes no effect on any location as 0.
        api.attribute(function, FUNCTION_INDEX, "memory", 0, None, false);
    }
}

/// Marks a function in which LLVM must not treat library functions as
/// builtins, as Clang marks every function under `-fno-builtin`
/// (`no-builtins`) and `-fno-builtin-NAME` (`no-builtin-NAME`); LLVM's
/// library-call simplification and loop-idiom recognition read them.
fn library_builtins<'id>(
    api: &Session<'id>,
    function: Val<'id>,
    builtins: LibraryBuiltins<'_>,
    arena: &Bump,
) {
    match builtins {
        | LibraryBuiltins::All => {},
        | LibraryBuiltins::None => api.string_attribute(function, "no-builtins", ""),
        | LibraryBuiltins::Except(names) =>
            for name in names {
                api.string_attribute(function, format_in!(arena, "no-builtin-{name}"), "");
            },
    }
}
