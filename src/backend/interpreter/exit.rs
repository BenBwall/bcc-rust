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
//! registered and before one a destructor registers. A handler registered
//! while the handlers run is called next, and a handler that calls `exit`
//! continues the list with the new status. Then the host writes out its
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
    host::Host,
    trap::{
        Fault,
        Trap,
        UbKind,
        Unsupported,
    },
    value::RuntimeValue,
};
use crate::{
    ir::{
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
#[expect(
    variant_size_differences,
    reason = "A short list of handlers; boxing the argument would allocate."
)]
enum Handler {
    /// `atexit` or `at_quick_exit`: called without arguments.
    Plain(FuncId),
    /// `on_exit`: called with the exit status and this argument.
    WithStatus(FuncId, RuntimeValue),
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
    /// Runs the exit function `name` with the call's arguments: the
    /// registration's result, or the program's ending.
    pub(super) fn exit_function(
        &mut self,
        name: &str,
    ) -> Result<Result<Option<RuntimeValue>, Termination>, Fault> {
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
                    Handler::WithStatus(self.handler(argument(0), &[Type::I32, Type::Ptr])?, argument(1))
                } else {
                    if types != [Type::Ptr] {
                        return Err(UbKind::SignatureMismatch.into());
                    }
                    Handler::Plain(self.handler(argument(0), &[])?)
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
    /// and which returns nothing.
    fn handler(&self, pointer: RuntimeValue, parameters: &[Type]) -> Result<FuncId, Fault> {
        let func = self.memory.function_at(pointer)?;
        let func = self.module.function_definition(func);
        let signature = self.module.function_signature(func);
        if self.module.function(func).body.is_none() {
            return Err(Fault::Unsupported(Unsupported::HostExitHandler));
        }
        if signature.params != parameters || signature.result.is_some() {
            return Err(UbKind::SignatureMismatch.into());
        }
        Ok(func)
    }

    /// How the program is ending.
    pub(super) const fn ending(&self) -> Ending {
        self.exit.ending
    }

    /// Ends a program that returned from `main` or called an exit
    /// function: runs the handlers its ending calls, and the destructors on
    /// a normal end, keeping its termination unless a handler exits again
    /// or aborts.
    pub(super) fn finish(
        &mut self,
        destructors: &[FuncId],
        ended: Outcome,
        host: &mut dyn Host,
    ) -> Result<Outcome, Trap> {
        let mut termination = ended.termination;
        let mut last = ended;
        // `exit` ends the program from inside its frames; their automatic
        // objects no longer exist when the handlers run.
        self.unwind();
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
            match handler {
                | Handler::Destructors => {
                    for &func in destructors {
                        self.enter_function(func)?;
                        last = self.run(host)?;
                        if !matches!(last.termination, Termination::Returned(_)) {
                            return Ok(last);
                        }
                    }
                    continue;
                },
                | Handler::Plain(func) => self.enter_handler(func, &[])?,
                | Handler::WithStatus(func, argument) => self.enter_handler(
                    func,
                    &[
                        RuntimeValue::int(Type::I32, u128::from(status.cast_unsigned())),
                        argument,
                    ],
                )?,
            }
            last = self.run(host)?;
            match last.termination {
                | Termination::Returned(_) => {},
                // glibc resumes the list after the handler, with the new
                // status, or switches to the quick list or stops.
                | Termination::Exited(_) => {
                    termination = last.termination;
                    self.unwind();
                },
                | Termination::Aborted => return Ok(last),
            }
        }
        Ok(Outcome {
            termination,
            ..last
        })
    }

    /// Enters the exit handler `func` with `args`.
    fn enter_handler(&mut self, func: FuncId, args: &[RuntimeValue]) -> Result<(), Trap> {
        let module = self.module;
        self.enter(module.function(func).name, args)
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
