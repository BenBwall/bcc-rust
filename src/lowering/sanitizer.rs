//! `UndefinedBehaviorSanitizer`: the checks `-fsanitize=undefined` asks for,
//! written as lowering meets each operation they guard. The options' model
//! is `target/sanitizers.rs`.
//!
//! A check is ordinary IR, so every back end runs it as written: the
//! condition that the operation would be undefined, a branch on it, and on
//! that branch either the `trap` instruction (`-fsanitize-trap`) or a call of
//! the run-time library's `__ubsan_handle_*` function with the check's
//! static data and the operands, after which the program goes on
//! (`-fsanitize-recover`, the default) or does not (the `_abort` handlers).
//! The checked operation itself is then computed without any promise its
//! undefined case would allow: a checked signed `+` has no `nsw`.
//!
//! For `int f(int a) { return a + 1; }` under GCC's `-fsanitize=undefined`:
//!
//! ```text
//! v1 = iadd.i32 v0, 1                      ; wraps
//! ...  v4 = icmp slt (a ^ v1) & (1 ^ v1), 0 ; the signs say it overflowed
//! brif v4, block1, block2
//! block1:
//! call @__ubsan_handle_add_overflow(@.ubsan.data, zext a, 1)
//! jump block2
//! ```
//!
//! The data (`.ubsan.data`) is a source location and a type descriptor in the
//! layout both libraries share (compiler-rt's `ubsan_handlers.h`); the
//! library writes to the location to report each place once, so it is
//! writable. Which column a report names, how a type is spelled, and which
//! library is linked follow the compiler bcc answers as (`sites.rs`,
//! `data.rs`, `Hardening::link_flags`).
//!
//! Read [`FunctionLowerer::check_undefined`], then `sites.rs` for where a
//! report points, `data.rs` for the static data and the operands' encoding,
//! and the checks by kind: `arithmetic.rs` (overflow, shifts, division),
//! `memory.rs` (null, alignment, array bounds, pointer overflow) and
//! `values.rs` (`_Bool` loads, floating conversions, variable length array
//! bounds, `__builtin_clz` and `__builtin_unreachable`).
//!
//! GCC manual, "Instrumentation Options"; Clang,
//! "`UndefinedBehaviorSanitizer`"; compiler-rt `lib/ubsan/ubsan_handlers.h` for
//! the run-time interface. C99: §3.4.3, p. 4; PDF p. 16 (undefined behavior,
//! which each check names by its clause).

// Checks by kind
mod arithmetic;
mod memory;
mod values;

// Reports
mod data;
mod sites;

pub(super) use data::{
    Field,
    Handler,
    Style,
};
pub(super) use sites::{
    Parent,
    Root,
    operator_site,
};

use super::{
    FunctionLowerer,
    ssa::{
        Op,
        Terminator,
    },
};
use crate::{
    ir::{
        Align,
        FuncId,
        GlobalId,
        InstData,
        Opcode,
        Type,
        Value,
    },
    target::sanitizers::{
        UbChecks,
        UndefinedSanitizer,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::Expression,
        semantic_analysis::TypeId,
    },
    util::bump::{
        ArenaMap,
        Bump,
    },
};

impl<'s> FunctionLowerer<'_, '_, '_, '_, 's> {
    /// Checks that `failed` is false. When it is true the program reaches
    /// `check`'s undefined behavior at `site`: it traps if the options say
    /// so, and otherwise calls the library with what `report` gives, built
    /// only on that path, then goes on or ends as the options say.
    pub(super) fn check_undefined(
        &mut self,
        check: UbChecks,
        failed: Value,
        site: SourceVectors,
        report: impl FnOnce(&mut Self) -> Report<'s>,
    ) {
        if self.draft.constant(failed) == Some(0) {
            return;
        }
        let options = self.unit.sanitizer.options;
        let fail = self.draft.create_block(&[]);
        let next = self.draft.create_block(&[]);
        self.draft.brif(failed, fail, next);
        self.draft.seal(fail);
        self.draft.switch_to(fail);
        if options.trap.intersects(check) {
            _ = self.draft.push(
                Op::Inst(InstData::Nullary {
                    opcode: Opcode::Trap,
                    ty:     Type::Ptr,
                }),
                None,
            );
            self.draft.terminate(Terminator::Unreachable);
        } else {
            let report = report(self);
            let recover = options.recover.intersects(check) && !report.handler.always_ends();
            let data_site = match report.site {
                | DataSite::Check => Some(site),
                | DataSite::Given(given) => given,
            };
            let data = self.unit.check_data(data_site, report.fields);
            let address = self.inst(InstData::GlobalAddr { global: data }, Type::Ptr);
            let handler = self
                .unit
                .ub_handler(report.handler, report.arguments.len(), recover);
            let mut arguments = bumpalo_vec(self.scratch, report.arguments.len() + 1);
            arguments.push(address);
            arguments.extend_from_slice(report.arguments);
            let arguments = self.scratch.alloc_slice_copy(&arguments);
            _ = self.draft.push(Op::Call(handler, arguments), None);
            if recover {
                self.draft.jump(next, &[]);
            } else {
                self.draft.terminate(Terminator::Unreachable);
            }
        }
        self.draft.seal(next);
        self.draft.switch_to(next);
    }

    /// Whether `check` runs in this function: the options ask for it and
    /// no `no_sanitize` leaves it out.
    pub(super) fn checks(&self, check: UbChecks) -> bool {
        self.unit
            .sanitizer
            .options
            .checks
            .difference(self.ub.disabled)
            .intersects(check)
    }

    /// Whether the reports follow GCC's conventions rather than Clang's.
    pub(super) fn gcc_reports(&self) -> bool {
        self.unit.sanitizer.options.gcc
    }
}

/// What a failed check passes the library: the handler, the fields of its
/// static data after the source location, and the operands, already encoded
/// as the library reads them ([`FunctionLowerer::value_handle`]).
pub(super) struct Report<'s> {
    pub(super) handler:   Handler,
    pub(super) fields:    &'s [Field],
    pub(super) arguments: &'s [Value],
    /// The source location the data starts with.
    pub(super) site:      DataSite,
}

/// The source location a check's static data starts with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DataSite {
    /// Where the check failed.
    Check,
    /// Another place, or none (`None`), where the data starts with the place
    /// an attribute was written.
    Given(Option<SourceVectors>),
}

/// The unit's sanitizer options and the static data and handlers made so
/// far, which the unit's checks share.
pub(super) struct UnitSanitizer<'a> {
    pub(super) options: UndefinedSanitizer,
    /// The name of each source file, by index.
    files:              ArenaMap<'a, u32, GlobalId>,
    /// Each type descriptor, by its kind, information and name.
    descriptors:        ArenaMap<'a, (u16, u16, &'a str), GlobalId>,
    /// Each handler declared, by name.
    handlers:           ArenaMap<'a, &'a str, FuncId>,
}

impl<'a> UnitSanitizer<'a> {
    pub(super) fn new(options: UndefinedSanitizer, tables: &'a Bump) -> Self {
        Self {
            options,
            files: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            descriptors: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            handlers: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
        }
    }
}

/// What the checks of one function remember while it is lowered.
pub(super) struct FunctionSanitizer<'s, 'tu> {
    /// The expression whose operator or conversion lowering is completing,
    /// whose checks report at its site.
    pub(super) site:      Option<&'tu Expression<'tu>>,
    /// Where the statement or declaration being lowered starts, which GCC
    /// reports a conversion of a name at.
    pub(super) statement: Option<SourceVectors>,
    /// GCC's report sites that are not the operator's: the statement an
    /// expression's value is assigned or passed in (`sites.rs`), by the
    /// expression's address.
    pub(super) roots:     ArenaMap<'s, usize, (SourceVectors, Root)>,
    /// What encloses an expression where that changes its checks, by the
    /// expression's address.
    pub(super) parents:   ArenaMap<'s, usize, Parent<'tu>>,
    /// Addresses within an object lowering placed, which are not null, and
    /// how aligned each is known to be.
    pub(super) known:     ArenaMap<'s, Value, Align>,
    /// For GCC, the places reached through a member of a record a pointer
    /// points to, with that pointer and the record's type: GCC checks an
    /// access there as an access of that member (`memory.rs`).
    pub(super) members:   ArenaMap<'s, Value, (Value, TypeId)>,
    /// The alignment an access of a member's place needs, by place.
    pub(super) required:  ArenaMap<'s, Value, Align>,
    /// The elements whose index was checked against their array's bounds.
    pub(super) bounded:   ArenaMap<'s, Value, ()>,
    /// The checks the function's `no_sanitize` leaves out.
    pub(super) disabled:  UbChecks,
}

impl<'s> FunctionSanitizer<'s, '_> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            site:      None,
            statement: None,
            roots:     ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            parents:   ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            known:     ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            members:   ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            required:  ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            bounded:   ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            disabled:  UbChecks::empty(),
        }
    }
}

/// An empty vector of `capacity` values in `scratch`.
fn bumpalo_vec(scratch: &Bump, capacity: usize) -> crate::util::bump::ArenaVec<'_, Value> {
    crate::util::bump::ArenaVec::with_capacity_in(capacity, scratch)
}
