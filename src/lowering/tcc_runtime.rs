//! TCC's runtime library under `--tcc-mode`: the functions of `libtcc1.a`
//! that bcc's headers declare, written in `tcc_runtime/libtcc1.c`. TCC links
//! that archive into every program, so a TCC program may take the address
//! of `atomic_signal_fence`, which GCC and Clang leave to libatomic. bcc has
//! no such archive. Instead the middle end compiles the C source with bcc
//! for the program's target when a unit references one of these functions
//! without defining it (`pipeline::middle_end`), and [`link`] moves the body
//! into the unit as a weak definition: every unit that needs one carries it,
//! with no extra file on any link line, and a program's own strong
//! definition, in any unit, still replaces it, as one replaces a member of
//! `libtcc1.a`.
//! TCC extension: TCC's `lib/atomic.S` and `include/stdatomic.h`.
//! C11: §7.17.4, pp. 278-279; PDF pp. 296-297; §7.17.8, pp. 285-286;
//! PDF pp. 303-304 (the functions as the standard declares them).

use crate::{
    ir::{
        Body,
        Function,
        InstData,
        Linkage,
        Module,
        Symbol,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The runtime's C source.
pub(crate) const SOURCE: &str = include_str!("tcc_runtime/libtcc1.c");

/// Whether `module` references a function of the runtime without defining
/// it.
pub(crate) fn needed(module: &Module<'_>) -> bool {
    module.functions().any(|(_, function)| wanted(function))
}

/// Defines each runtime function `module` references with the body
/// `runtime` gives it, as a weak definition, moving the body out of
/// `runtime`. A declaration whose signature differs from the runtime's (a
/// program's own `atomic_signal_fence` of another type) stays a reference.
/// The error names what a function needed that linking cannot carry, a bug
/// of bcc.
pub(crate) fn link<'ir>(
    module: &mut Module<'ir>,
    runtime: &mut Module<'ir>,
    scratch: &Bump,
) -> Result<(), &'static str> {
    let mut targets = ArenaVec::new_in(scratch);
    targets.extend(
        module
            .functions()
            .filter(|(_, function)| wanted(function))
            .map(|(id, _)| id),
    );
    for target in targets {
        let declared = module.signature(module.function(target).signature);
        let Some(Symbol::Function(source)) = runtime.symbol(module.function(target).name) else {
            return Err("a TCC runtime function is missing");
        };
        let defined = runtime.signature(runtime.function(source).signature);
        if (declared.params, declared.result, declared.variadic)
            != (defined.params, defined.result, defined.variadic)
        {
            continue;
        }
        let Some(body) = runtime.function_mut(source).body.take() else {
            return Err("a TCC runtime function has no body");
        };
        if refers_to_module_items(&body) {
            return Err("a TCC runtime function refers to a module item");
        }
        let function = module.function_mut(target);
        function.linkage = Linkage::Weak;
        function.body = Some(body);
    }
    Ok(())
}

/// The names of the runtime's functions, as `libtcc1.c` defines them.
const NAMES: [&str; 6] = [
    "atomic_thread_fence",
    "atomic_signal_fence",
    "atomic_flag_test_and_set",
    "atomic_flag_test_and_set_explicit",
    "atomic_flag_clear",
    "atomic_flag_clear_explicit",
];

/// Whether `function` is an external reference to a runtime function.
fn wanted(function: &Function<'_>) -> bool {
    function.body.is_none()
        && function.linkage == Linkage::External
        && NAMES.contains(&function.name)
}

/// Whether `body` calls a function or names a global or assembly, which
/// would need entities of the runtime's module.
fn refers_to_module_items(body: &Body<'_>) -> bool {
    body.blocks().any(|block| {
        body.block_insts(block).iter().any(|&inst| {
            matches!(
                body.inst(inst),
                InstData::Call { .. }
                    | InstData::CallIndirect { .. }
                    | InstData::GlobalAddr { .. }
                    | InstData::FuncAddr { .. }
                    | InstData::InlineAsm { .. }
            )
        })
    })
}
