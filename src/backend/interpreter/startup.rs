//! Constructors before hosted main and destructors after it. Constructors run
//! in increasing priority, ties in registration order; destructors in
//! decreasing priority, ties in reverse registration order. This is the order
//! the ELF `.init_array`/`.fini_array` and MinGW `.ctors`/`.dtors` runtimes
//! give the entries LLVM emits for bundled Clang. Destructors run after main
//! returns and after `exit`, among the exit handlers (`exit.rs`), but not
//! after `abort`, `quick_exit` or `_Exit`.
//! C99: §5.1.2.2.3, p. 13; PDF p. 25; §7.20.4.3p2-5, pp. 315-316;
//! PDF pp. 327-328. GNU extension: GCC manual, "Common Function Attributes",
//! constructor/destructor.
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>

use super::{
    Machine,
    Trap,
};
use crate::{
    ir::{
        FuncId,
        InitKind,
        Module,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The destructors in run order, the first `unprioritized` of them those
/// without a priority (65535), which the ELF `.fini_array` runs before
/// crtbegin's `__do_global_dtors_aux` and the prioritized ones after it.
pub(super) struct Destructors<'a> {
    pub(super) functions:     ArenaVec<'a, FuncId>,
    pub(super) unprioritized: usize,
}

/// The constructors and the destructors of `module`, each in run order.
pub(super) fn order<'a>(
    module: &Module<'_>,
    arena: &'a Bump,
) -> (ArenaVec<'a, FuncId>, Destructors<'a>) {
    let mut constructors = ArenaVec::new_in(arena);
    let mut destructors = ArenaVec::new_in(arena);
    for init in module.init_functions() {
        match init.kind {
            | InitKind::Constructor => constructors.push((init.priority, init.func)),
            | InitKind::Destructor => destructors.push((init.priority, init.func)),
        }
    }
    // Stable sorts keep registration order among equal constructor
    // priorities; reversing first leaves equal destructor priorities in
    // reverse registration order.
    constructors.sort_by_key(|&(priority, _)| priority);
    destructors.reverse();
    destructors.sort_by_key(|&(priority, _)| std::cmp::Reverse(priority));
    let functions = |list: ArenaVec<'_, (u16, FuncId)>| {
        let mut result = ArenaVec::with_capacity_in(list.len(), arena);
        result.extend(list.iter().map(|&(_, func)| func));
        result
    };
    let unprioritized = destructors
        .iter()
        .take_while(|&&(priority, _)| priority == u16::MAX)
        .count();
    (
        functions(constructors),
        Destructors {
            functions: functions(destructors),
            unprioritized,
        },
    )
}

impl Machine<'_, '_, '_> {
    /// Enters a parameterless constructor or destructor, or the definition
    /// an alias names.
    pub(super) fn enter_function(&mut self, func: FuncId) -> Result<(), Trap> {
        let module = self.module;
        self.enter(module.function(module.function_definition(func)).name, &[])
    }
}
