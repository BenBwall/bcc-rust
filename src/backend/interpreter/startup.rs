//! Constructors before hosted main and destructors after it. Constructors run
//! in increasing priority, ties in registration order; destructors in
//! decreasing priority, ties in reverse registration order. This is the order
//! the ELF `.init_array`/`.fini_array` and MinGW `.ctors`/`.dtors` runtimes
//! give the entries LLVM emits for bundled Clang. Destructors run after main
//! returns and after `exit`, but not after `abort`.
//! C99: §5.1.2.2.3, p. 13; PDF p. 25; §7.20.4.3p2-5, pp. 315-316;
//! PDF pp. 327-328. GNU extension: GCC manual, "Common Function Attributes",
//! constructor/destructor.
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>

use super::{
    Host,
    Machine,
    Outcome,
    Termination,
    Trap,
};
use crate::{
    ir::{
        FuncId,
        InitKind,
        Module,
        Symbol,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The constructors and the destructors of `module`, each in run order.
pub(super) fn order<'a>(
    module: &Module<'_>,
    arena: &'a Bump,
) -> (ArenaVec<'a, FuncId>, ArenaVec<'a, FuncId>) {
    let mut constructors = ArenaVec::new_in(arena);
    let mut destructors = ArenaVec::new_in(arena);
    for init in module.init_functions() {
        match init.kind {
            | InitKind::Constructor => constructors.push((init.priority, init.func)),
            | InitKind::Destructor => destructors.push((init.priority, init.func)),
        }
    }
    // Stable sorts keep registration order among equal priorities.
    constructors.sort_by_key(|&(priority, _)| priority);
    destructors.reverse();
    destructors.sort_by_key(|&(priority, _)| std::cmp::Reverse(priority));
    let functions = |list: ArenaVec<'_, (u16, FuncId)>| {
        let mut result = ArenaVec::with_capacity_in(list.len(), arena);
        result.extend(list.iter().map(|&(_, func)| func));
        result
    };
    (functions(constructors), functions(destructors))
}

impl Machine<'_, '_, '_> {
    /// Enters a parameterless constructor or destructor, or the definition
    /// an alias names.
    pub(super) fn enter_function(&mut self, func: FuncId) -> Result<(), Trap> {
        let module = self.module;
        let func = match module.function(func).attributes.alias {
            | Some(Symbol::Function(target)) => target,
            | _ => func,
        };
        self.enter(module.function(func).name, &[])
    }

    /// Runs `destructors` after the program ended with `ended`, keeping its
    /// termination unless a destructor exits or aborts.
    pub(super) fn finish(
        &mut self,
        destructors: &[FuncId],
        ended: Outcome,
        host: &mut dyn Host,
    ) -> Result<Outcome, Trap> {
        let mut last = ended;
        // `exit` ends the program from inside its frames; their automatic
        // objects no longer exist when destructors run.
        self.unwind();
        for &func in destructors {
            self.enter_function(func)?;
            last = self.run(host)?;
            if !matches!(last.termination, Termination::Returned(_)) {
                return Ok(last);
            }
        }
        Ok(Outcome {
            termination: ended.termination,
            ..last
        })
    }
}
