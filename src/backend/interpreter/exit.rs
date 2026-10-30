//! How a program ends: the handlers `atexit`, `on_exit` and
//! `at_quick_exit` register, and `exit`, `quick_exit`, `_Exit` and `_exit`.
//!
//! The machine runs these itself, as it runs `qsort`'s comparisons, since a
//! handler is an ordinary interpreted function. It keeps glibc's lists
//! (`stdlib/exit.c`): a normal end, after `main` returns or the program
//! calls `exit`, calls the `atexit` and `on_exit` handlers in the reverse
//! order of their registration, an `on_exit` handler with the exit status
//! and its argument. The module's destructors run where glibc's dynamic
//! linker registered its own handler, before `main` and its constructors
//! could register any, so they run after every handler the program
//! registered. A handler registered while the handlers run is called next;
//! one a destructor registers with `atexit` runs from crtbegin's
//! `__cxa_finalize`, between the destructors without a priority and the
//! prioritized ones. A handler or destructor that
//! calls `exit` continues the list with the new status (a destructor ends
//! the destructors). The handlers run on top of the frames that called
//! `exit`, whose automatic objects stay live. Then the host writes out its
//! streams. `quick_exit` calls the `at_quick_exit` handlers the same way
//! and ends without writing them out; `_Exit` and `_exit` end at once. A
//! handler is checked when it is registered: it must be a function the
//! program defines, of the type its registration takes.
//!
//! C99: §7.20.4.2-§7.20.4.4, pp. 315-316; PDF pp. 327-328. C11:
//! §7.22.4.3 and §7.22.4.7 (`at_quick_exit`, `quick_exit`). POSIX
//! (`_exit`); glibc (`on_exit`).

use super::{
    Machine,
    Outcome,
    Termination,
    callbacks::Callback,
    host::Host,
    startup::Destructors,
    trap::{
        Fault,
        Location,
        Trap,
        UbKind,
        Unsupported,
    },
    value::RuntimeValue,
};
use crate::{
    ir::{
        Block,
        Entity as _,
        FuncId,
        Type,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The functions this module runs.
pub(crate) const FUNCTIONS: [&str; 7] = [
    "exit",
    "atexit",
    "on_exit",
    "at_quick_exit",
    "quick_exit",
    "_Exit",
    "_exit",
];

/// A registered exit handler.
#[derive(Clone, Copy, Debug)]
enum Handler {
    /// `atexit` or `at_quick_exit`: called without arguments, but for the
    /// static chain of a nested function a trampoline names.
    Plain(FuncId, Option<RuntimeValue>),
    /// `on_exit`: called with the exit status and this argument, and the
    /// static chain if any.
    WithStatus(FuncId, RuntimeValue, Option<RuntimeValue>),
    /// The module's destructors, which glibc's dynamic linker runs from the
    /// handler it registers first.
    Destructors,
}

/// How a program that called one of the exit functions is ending.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ending {
    /// `exit`, or a return from `main`: the handlers, the destructors, and
    /// the streams written out.
    Normal,
    /// `quick_exit`: the `at_quick_exit` handlers, and nothing written out.
    Quick,
    /// `_Exit` or `_exit`: nothing more.
    Immediate,
}

/// The handlers the program registered and how it is ending.
pub(super) struct Exit<'a> {
    handlers: ArenaVec<'a, Handler>,
    quick:    ArenaVec<'a, Handler>,
    ending:   Ending,
}

impl Machine<'_, '_, '_> {
    /// Runs the exit function `name`, declared with the result type
    /// `result`, with the call's arguments: the registration's result, or
    /// the program's ending. A call through an incompatible declaration is
    /// undefined.
    pub(super) fn exit_function(
        &mut self,
        name: &str,
        result: Option<Type>,
    ) -> Result<Result<Option<RuntimeValue>, Termination>, Fault> {
        let registers = matches!(name, "atexit" | "on_exit" | "at_quick_exit");
        if result != registers.then_some(Type::I32) {
            return Err(UbKind::SignatureMismatch.into());
        }
        let argument = |index: usize| {
            self.arguments
                .get(index)
                .copied()
                .unwrap_or(RuntimeValue::Poison)
        };
        let types: &[Type] = &self.argument_types;
        let ending = match name {
            | "exit" => Ending::Normal,
            | "quick_exit" => Ending::Quick,
            | "_Exit" | "_exit" => Ending::Immediate,
            | _ => {
                let handler = if name == "on_exit" {
                    if types != [Type::Ptr, Type::Ptr] {
                        return Err(UbKind::SignatureMismatch.into());
                    }
                    let (func, chain) = self.handler(argument(0), &[Type::I32, Type::Ptr])?;
                    Handler::WithStatus(func, argument(1), chain)
                } else {
                    if types != [Type::Ptr] {
                        return Err(UbKind::SignatureMismatch.into());
                    }
                    let (func, chain) = self.handler(argument(0), &[])?;
                    Handler::Plain(func, chain)
                };
                if name == "at_quick_exit" {
                    self.exit.quick.push(handler);
                } else {
                    self.exit.handlers.push(handler);
                }
                return Ok(Ok(Some(RuntimeValue::int(Type::I32, 0))));
            },
        };
        if types != [Type::I32] {
            return Err(UbKind::SignatureMismatch.into());
        }
        let status = argument(0)
            .as_int()
            .ok_or(Fault::Ub(UbKind::PoisonInHostCall))?;
        self.exit.ending = ending;
        #[expect(clippy::cast_possible_truncation, reason = "The status is an int.")]
        Ok(Err(Termination::Exited(status as u32 as i32)))
    }

    /// The function `pointer` names, which a registration makes an exit
    /// handler: one the program defines, whose parameters are `parameters`
    /// and which returns nothing, with the static chain a trampoline passes
    /// it.
    fn handler(
        &self,
        pointer: RuntimeValue,
        parameters: &[Type],
    ) -> Result<(FuncId, Option<RuntimeValue>), Fault> {
        let (func, chain) = self.callee(pointer)?;
        let signature = self.module.function_signature(func);
        if self.module.function(func).body.is_none() {
            return Err(Fault::Unsupported(Unsupported::HostExitHandler));
        }
        let own = signature.params.len() - usize::from(chain.is_some());
        if signature.params[..own] != *parameters || signature.result.is_some() {
            return Err(UbKind::SignatureMismatch.into());
        }
        Ok((func, chain))
    }

    /// How the program is ending.
    pub(super) const fn ending(&self) -> Ending {
        self.exit.ending
    }

    /// Ends a program that returned from `main` or called an exit
    /// function: runs the handlers its ending calls, and the destructors on
    /// a normal end, keeping its termination unless a handler exits again
    /// or aborts. `exit` does not return, so the frames that called it, and
    /// their automatic objects, stay live while the handlers run on top of
    /// them; after a return from `main` only its caller's are left.
    pub(super) fn finish(
        &mut self,
        destructors: &Destructors<'_>,
        ended: Outcome,
        host: &mut dyn Host,
    ) -> Result<Outcome, Trap> {
        let mut termination = ended.termination;
        loop {
            let handler = match self.exit.ending {
                | Ending::Normal => self.exit.handlers.pop(),
                | Ending::Quick => self.exit.quick.pop(),
                | Ending::Immediate => None,
            };
            let Some(handler) = handler else {
                break;
            };
            let status = match termination {
                | Termination::Exited(status) => status,
                #[expect(clippy::cast_possible_truncation, reason = "`main` returns an int.")]
                | Termination::Returned(Some(RuntimeValue::Int(bits))) => bits as u32 as i32,
                | _ => 0,
            };
            let ended = match handler {
                // A destructor that exits ends the destructors, and glibc
                // goes on with the handlers registered since.
                | Handler::Destructors => self.run_destructors(destructors, host)?,
                | Handler::Plain(func, chain) => self.call_handler(func, chain.as_slice(), host)?,
                | Handler::WithStatus(func, argument, chain) => {
                    let status = RuntimeValue::int(Type::I32, u128::from(status.cast_unsigned()));
                    let args = [status, argument, chain.unwrap_or(RuntimeValue::Poison)];
                    self.call_handler(func, &args[..2 + usize::from(chain.is_some())], host)?
                },
            };
            match ended {
                // glibc resumes the list after the handler, with the new
                // status, or switches to the quick list or stops.
                | Some(exited @ Termination::Exited(_)) => termination = exited,
                | Some(Termination::Aborted) => {
                    termination = Termination::Aborted;
                    break;
                },
                | None | Some(Termination::Returned(_)) => {},
            }
        }
        Ok(Outcome {
            termination,
            steps: self.steps,
            #[cfg(test)]
            memory_cells: self.memory.backing_cells(),
        })
    }

    /// Runs the destructors as glibc's `_dl_fini` runs the program's
    /// `.fini_array`, backwards: those without a priority, then crtbegin's
    /// `__do_global_dtors_aux`, whose `__cxa_finalize` calls the `atexit`
    /// handlers they registered (newest first; `on_exit` handlers wait for
    /// the list), then the prioritized ones. A destructor or handler that
    /// exits ends the destructors: `None`, or how the program ended.
    fn run_destructors(
        &mut self,
        destructors: &Destructors<'_>,
        host: &mut dyn Host,
    ) -> Result<Option<Termination>, Trap> {
        for (index, &func) in destructors.functions.iter().enumerate() {
            if index == destructors.unprioritized
                && let Some(ended) = self.finalize(host)?
            {
                return Ok(Some(ended));
            }
            if let Some(ended) = self.call_handler(func, &[], host)? {
                return Ok(Some(ended));
            }
        }
        if destructors.unprioritized == destructors.functions.len() {
            return self.finalize(host);
        }
        Ok(None)
    }

    /// `__cxa_finalize` for the program: calls every `atexit` handler still
    /// registered, newest first, including those they register.
    fn finalize(&mut self, host: &mut dyn Host) -> Result<Option<Termination>, Trap> {
        while let Some(index) = self
            .exit
            .handlers
            .iter()
            .rposition(|handler| matches!(handler, Handler::Plain(..)))
        {
            let Handler::Plain(func, chain) = self.exit.handlers.remove(index) else {
                unreachable!("found a plain handler");
            };
            if let Some(ended) = self.call_handler(func, chain.as_slice(), host)? {
                return Ok(Some(ended));
            }
        }
        Ok(None)
    }

    /// Calls the exit handler or destructor `func` with `args` on top of the
    /// frames still live, and runs it to its return: `None` then, or how
    /// the program ended inside it.
    fn call_handler(
        &mut self,
        func: FuncId,
        args: &[RuntimeValue],
        host: &mut dyn Host,
    ) -> Result<Option<Termination>, Trap> {
        let module = self.module;
        let func = module.function_definition(func);
        let floor = self.frames.len();
        let outer = self.callback.replace(Callback::Waiting(floor));
        let result = self.enter(module.function(func).name, args).and_then(|()| {
            loop {
                if let Some(Callback::Returned(_)) = self.callback {
                    break Ok(None);
                }
                if self.frames.len() <= floor {
                    // A `longjmp` left the handler for a frame below it.
                    let location = self.entry_location(func);
                    break Err(Trap::Unsupported(Unsupported::CallbackEscape, location));
                }
                if let Some(termination) = self.step(host)? {
                    break Ok(Some(termination));
                }
            }
        });
        self.callback = outer;
        result
    }

    /// The first instruction of the defined function `func`.
    fn entry_location(&self, func: FuncId) -> Location {
        let body = self
            .module
            .function(func)
            .body
            .as_ref()
            .expect("exit handlers are defined");
        Location {
            func,
            inst: body.block_insts(Block::new(0))[0],
        }
    }
}

impl<'a> Exit<'a> {
    /// No handlers yet but the destructors, ending normally.
    pub(super) fn new_in(arena: &'a Bump) -> Self {
        let mut handlers = ArenaVec::new_in(arena);
        handlers.push(Handler::Destructors);
        Self {
            handlers,
            quick: ArenaVec::new_in(arena),
            ending: Ending::Normal,
        }
    }
}
