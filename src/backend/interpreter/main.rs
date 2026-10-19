//! Hosted main entry, with argv and its strings stored in writable memory.
//! C99: §5.1.2.2.1 paragraphs 1-2, p. 12; PDF p. 24.

use super::{
    EntryError,
    Machine,
    ObjectKind,
    RuntimeValue,
    Trap,
};
use crate::ir::{
    Align,
    Symbol,
    Type,
};

impl Machine<'_, '_, '_> {
    /// Builds the arguments of C main without relaxing the explicit-argument
    /// contract of the general interpreter entry point.
    /// C99: §5.1.2.2.1 paragraph 2, p. 12; PDF p. 24.
    pub(super) fn enter_main(&mut self) -> Result<(), Trap> {
        let Some(Symbol::Function(func)) = self.module.symbol("main") else {
            return Err(Trap::BadEntry(EntryError::NotFound));
        };
        let params = self.module.function_signature(func).params;
        if params.is_empty() {
            return self.enter("main", &[]);
        }
        if params != [Type::I32, Type::Ptr] {
            return Err(Trap::BadEntry(EntryError::Arguments));
        }
        let allocate = |memory: &mut super::Memory<'_>, size, align| {
            memory
                .allocate(ObjectKind::Global, size, align)
                .map(RuntimeValue::Ptr)
                .map_err(|_| Trap::BadEntry(EntryError::GlobalTooLarge))
        };
        let name = allocate(&mut self.memory, 1, Align::BYTE)?;
        // Memory bases are at least 16-aligned; byte alignment also permits
        // writing the individual entries without an infallible constructor.
        let align = Align::BYTE;
        let argv = allocate(&mut self.memory, 16, align)?;
        self.memory
            .store(Type::I8, RuntimeValue::Int(0), name, Align::BYTE)
            .and_then(|()| self.memory.store(Type::Ptr, name, argv, align))
            .and_then(|()| {
                let end = self.memory.ptr_add(argv, RuntimeValue::Int(8), true);
                self.memory.store(Type::Ptr, RuntimeValue::NULL, end, align)
            })
            .map_err(|_| Trap::BadEntry(EntryError::GlobalTooLarge))?;
        self.enter("main", &[RuntimeValue::Int(1), argv])
    }
}
