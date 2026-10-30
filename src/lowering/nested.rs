//! GNU nested functions: a function defined in a block, which sees the
//! automatic objects of the functions around it.
//!
//! Each nested function is an internal IR function whose last fixed
//! parameter is its static chain ([`crate::ir::Signature::chain`]): the
//! address of the chain record of the function that defines it. A function
//! with nested definitions builds its record on entry: word 0 holds its own
//! static chain (null at file scope), word 1 the address of its trampoline
//! area (or null), and the later words the address of each of its objects a
//! nested function uses, so a use reaches an object any number of functions
//! out by following the first words. Definitions are lowered innermost first
//! ([`definition_order`]), so a record's layout, its trampolines and the
//! objects it points to, which then live in memory, are known before the
//! function that builds it is lowered. A direct call passes the record as the
//! chain.
//!
//! Taking a nested function's address gives a trampoline in the frame of the
//! function that defines it, as GCC builds one, which calls the nested
//! function with that function's record. The definer makes the trampolines
//! that functions inside it use on entry, once per activation, in its
//! trampoline area; one only the definer itself uses is a slot of its own,
//! made where the definer takes the address.
//!
//! For `int f(int n) { int g(void) { return n; } return g(); }`, `g` reads
//! `n` through word 2 of `f`'s record, and `f`'s call of `g` passes the
//! record as `g`'s last argument.
//!
//! GNU extension: GCC manual, "Nested Functions".
//! <https://gcc.gnu.org/onlinedocs/gcc/Nested-Functions.html>
//! C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42 (block scope);
//! §6.2.4 paragraph 5, p. 32; PDF p. 44 (automatic lifetime).

use super::{
    Construct,
    FunctionLowerer,
    LoweringError,
    Unit,
    module_items::at,
};
use crate::{
    ir::{
        AggregateParam,
        Align,
        ExtendedParam,
        FuncId,
        InstData,
        InstFlags,
        Opcode,
        SigId,
        Signature,
        TRAMPOLINE_SIZE,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::FunctionDefinition,
        semantic_analysis::{
            BindingKind,
            FunctionRecord,
            Linkage,
            ScopeKind,
            SemanticTranslationUnit,
        },
    },
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// The function definitions of a unit in the order they are lowered:
/// deepest-nested first, so that every nested function is lowered before
/// the function whose record it uses, and otherwise in `records`' order.
pub(super) fn definition_order<'a, 'tu>(
    unit: &Unit<'_, 'tu, '_>,
    records: &[(&'tu FunctionRecord<'tu>, Option<FuncId>)],
    arena: &'a Bump,
) -> ArenaVec<'a, (&'tu FunctionRecord<'tu>, Option<FuncId>)> {
    let mut ordered = ArenaVec::with_capacity_in(records.len(), arena);
    ordered.extend_from_slice(records);
    ordered.sort_by_key(|(record, _)| std::cmp::Reverse(unit.function_depth(record.scope)));
    ordered
}

/// What lowering learns about a unit's nested functions as it lowers them.
pub(super) struct NestedFunctions<'a> {
    /// The IR function of each nested definition, by its binding.
    pub(super) functions: ArenaMap<'a, usize, FuncId>,
    /// The same, by the scope and name of the definition, which a GNU
    /// `auto` declaration before it shares.
    by_name:              ArenaMap<'a, (usize, StringCacheId), FuncId>,
    /// The function scope that defines each nested function.
    definers:             ArenaMap<'a, FuncId, usize>,
    /// The function scopes with nested definitions, which build a record.
    pub(super) parents:   ArenaMap<'a, usize, ()>,
    /// The record word of each object nested functions use, by the
    /// function scope it belongs to and its binding.
    words:                ArenaMap<'a, (usize, usize), u32>,
    /// Each function scope's captured objects and record trampolines.
    lists:                ArenaMap<'a, usize, Lists>,
    /// The captured objects, `(binding, word, next)`, linked per scope.
    captures:             ArenaVec<'a, (usize, u32, u32)>,
    /// The record trampolines, `(function, next)`, linked per scope.
    trampolines:          ArenaVec<'a, (FuncId, u32)>,
    /// The index of each record trampoline in its scope's area.
    trampoline_indexes:   ArenaMap<'a, (usize, FuncId), u32>,
    /// The binding of each function definition at file scope, by its
    /// function scope.
    outermost:            ArenaMap<'a, usize, usize>,
}

/// A function scope's captured objects as `(word, binding)` and its
/// record trampolines as `(index, function)`.
type Layout<'s> = (ArenaVec<'s, (u32, usize)>, ArenaVec<'s, (u32, FuncId)>);

/// The heads and lengths of one function scope's lists in
/// [`NestedFunctions`].
#[derive(Clone, Copy)]
struct Lists {
    captures:         u32,
    capture_count:    u32,
    trampolines:      u32,
    trampoline_count: u32,
}

impl Lists {
    const EMPTY: Self = Self {
        captures:         u32::MAX,
        capture_count:    0,
        trampolines:      u32::MAX,
        trampoline_count: 0,
    };
}

impl<'a> NestedFunctions<'a> {
    pub(super) fn new(arena: &'a Bump, sema: &SemanticTranslationUnit<'_>) -> Self {
        let mut outermost = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
        for record in sema.functions {
            if let Some(binding) = record.binding {
                _ = outermost.insert(record.scope, binding);
            }
        }
        Self {
            functions: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            by_name: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            definers: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            parents: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            words: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            lists: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            captures: ArenaVec::new_in(arena),
            trampolines: ArenaVec::new_in(arena),
            trampoline_indexes: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            outermost,
        }
    }

    /// The function scope that defines `func`, if it is a nested function.
    pub(super) fn definer(&self, func: FuncId) -> Option<usize> {
        self.definers.get(&func).copied()
    }

    /// The record word that holds the address of `binding`, an object of
    /// the function scope `owner`, assigned on first use after the chain
    /// and trampoline-area words.
    fn word(&mut self, owner: usize, binding: usize) -> u32 {
        if let Some(&word) = self.words.get(&(owner, binding)) {
            return word;
        }
        let lists = self.lists.entry(owner).or_insert(Lists::EMPTY);
        let word = lists.capture_count + 2;
        let index = u32::try_from(self.captures.len()).expect("captures fit u32");
        self.captures.push((binding, word, lists.captures));
        lists.captures = index;
        lists.capture_count += 1;
        _ = self.words.insert((owner, binding), word);
        word
    }

    /// The index in `owner`'s trampoline area of the trampoline of `func`,
    /// assigned on first use.
    fn trampoline(&mut self, owner: usize, func: FuncId) -> u32 {
        if let Some(&index) = self.trampoline_indexes.get(&(owner, func)) {
            return index;
        }
        let lists = self.lists.entry(owner).or_insert(Lists::EMPTY);
        let index = lists.trampoline_count;
        let link = u32::try_from(self.trampolines.len()).expect("trampolines fit u32");
        self.trampolines.push((func, lists.trampolines));
        lists.trampolines = link;
        lists.trampoline_count += 1;
        _ = self.trampoline_indexes.insert((owner, func), index);
        index
    }

    /// `owner`'s captured objects as `(word, binding)` and its record
    /// trampolines as `(index, function)`, in `arena`.
    fn layout<'s>(&self, owner: usize, arena: &'s Bump) -> Layout<'s> {
        let lists = self.lists.get(&owner).copied().unwrap_or(Lists::EMPTY);
        let mut captures = ArenaVec::with_capacity_in(lists.capture_count as usize, arena);
        let mut next = lists.captures;
        while let Some(&(binding, word, link)) = self.captures.get(next as usize) {
            captures.push((word, binding));
            next = link;
        }
        let mut trampolines = ArenaVec::with_capacity_in(lists.trampoline_count as usize, arena);
        let mut next = lists.trampolines;
        while let Some(&(func, link)) = self.trampolines.get(next as usize) {
            trampolines.push((self.trampoline_indexes[&(owner, func)], func));
            next = link;
        }
        (captures, trampolines)
    }
}

impl<'tu> Unit<'_, 'tu, '_> {
    /// The function scope around `scope`, itself if it is one; `None` at
    /// file scope.
    pub(super) fn function_scope(&self, mut scope: usize) -> Option<usize> {
        loop {
            let data = self.sema.scopes[scope];
            match data.kind {
                | ScopeKind::Function => return Some(scope),
                | ScopeKind::File => return None,
                | ScopeKind::Block | ScopeKind::Prototype => scope = data.parent?,
            }
        }
    }

    /// The function scope around the function scope `scope`, if it belongs
    /// to a nested function.
    pub(super) fn enclosing_function(&self, scope: usize) -> Option<usize> {
        self.function_scope(self.sema.scopes[scope].parent?)
    }

    /// How many functions enclose the function scope `scope`.
    fn function_depth(&self, scope: usize) -> usize {
        let mut depth = 0;
        let mut scope = scope;
        while let Some(outer) = self.enclosing_function(scope) {
            depth += 1;
            scope = outer;
        }
        depth
    }

    /// Declares the nested function `record`: an internal function whose
    /// signature is its type's with the static chain last, named as GCC
    /// names it (`name.N`).
    pub(super) fn declare_nested(
        &mut self,
        record: &FunctionRecord<'tu>,
        binding: usize,
    ) -> Result<FuncId, LoweringError> {
        let source = record.syntax.source_vectors;
        if let Some(&func) = self.nested.functions.get(&binding) {
            return Ok(func);
        }
        let definer = self
            .enclosing_function(record.scope)
            .ok_or_else(|| LoweringError::missing("a nested function has no definer", source))?;
        // MSVC's stack probe `__chkstk` clobbers `%r10`, where the static
        // chain arrives, before the prologue has saved it.
        if self.target == crate::target::Target::WindowsMsvc {
            return Err(LoweringError::unsupported(
                Construct::NestedChainTarget,
                source,
            ));
        }
        // The bounds of a variably modified type are its definer's, which
        // GCC evaluates where the definition is.
        if self
            .sema
            .types
            .variably_modified(self.sema.bindings[binding].ty)
        {
            return Err(LoweringError::unsupported(
                Construct::NestedVariablyModified,
                source,
            ));
        }
        let unchained = self
            .signature(self.sema.bindings[binding].ty)
            .map_err(|kind| at(kind, source))?;
        let signature = self.chained(unchained);
        let name = self
            .context
            .string_cache
            .at(self.sema.bindings[binding].name.name);
        let base = crate::diagnostics::format_in!(self.tables, "{name}.0");
        let name = self.unique_name(base);
        let func = self
            .module
            .declare_function(name, signature, crate::ir::Linkage::Internal);
        _ = self.nested.functions.insert(binding, func);
        let data = self.sema.bindings[binding];
        _ = self
            .nested
            .by_name
            .insert((data.scope, data.name.name), func);
        _ = self.nested.definers.insert(func, definer);
        _ = self.nested.parents.insert(definer, ());
        Ok(func)
    }

    /// `sig` with a static chain added after its fixed parameters; the
    /// descriptions of the variadic arguments after it move up one.
    pub(super) fn chained(&mut self, sig: SigId) -> SigId {
        let signature = self.module.signature(sig);
        let fixed = signature.fixed_count();
        let shift = |index: u32| index + u32::from(index as usize >= fixed);
        let mut params = ArenaVec::with_capacity_in(signature.params.len() + 1, self.tables);
        params.extend_from_slice(&signature.params[..fixed]);
        params.push(Type::Ptr);
        params.extend_from_slice(&signature.params[fixed..]);
        let aggregates = self
            .tables
            .alloc_slice_fill_iter(signature.aggregates.iter().map(|param| AggregateParam {
                index: shift(param.index),
                abi:   param.abi,
            }));
        let extensions = self
            .tables
            .alloc_slice_fill_iter(signature.extensions.iter().map(|param| ExtendedParam {
                index:     shift(param.index),
                extension: param.extension,
            }));
        let noundef = self
            .tables
            .alloc_slice_fill_iter(signature.noundef.iter().map(|&index| shift(index)));
        let chained = Signature {
            params: &params,
            variadic_fixed: signature.variadic_fixed.map(|fixed| fixed + 1),
            aggregates,
            extensions,
            noundef,
            chain: true,
            ..signature
        };
        self.module.intern(chained)
    }

    /// The nested function `binding` names: its definition's, or that of
    /// the definition a GNU `auto` declaration before it declares.
    pub(super) fn nested_function(&self, binding: usize) -> Option<FuncId> {
        if let Some(&func) = self.nested.functions.get(&binding) {
            return Some(func);
        }
        let data = self.sema.bindings[binding];
        if data.kind != BindingKind::Function || data.linkage != Linkage::None {
            return None;
        }
        self.nested
            .by_name
            .get(&(data.scope, data.name.name))
            .copied()
    }

    /// Whether `record` is a nested function inside a reachable function
    /// at file scope, whose body may reach it however deeply it is
    /// nested.
    pub(super) fn nested_reachable(&self, record: &FunctionRecord<'tu>) -> bool {
        let mut scope = record.scope;
        let mut nested = false;
        while let Some(outer) = self.enclosing_function(scope) {
            nested = true;
            scope = outer;
        }
        nested
            && self
                .nested
                .outermost
                .get(&scope)
                .is_some_and(|binding| self.reachable.contains(binding))
    }

    /// Whether the function scope `scope` builds a chain record: it
    /// defines a nested function.
    fn builds_record(&self, scope: usize) -> bool {
        self.nested.parents.contains_key(&scope)
    }
}

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Builds the function's chain record on entry, after its parameters:
    /// its own static chain, its trampoline area, made here once per
    /// activation, and the address of each object its nested functions use,
    /// which lives in memory.
    pub(super) fn build_chain_record(&mut self, chain: Option<Value>) -> Result<(), LoweringError> {
        let scope = self.record.scope;
        let source = self.record.syntax.source_vectors;
        if !self.unit.builds_record(scope) {
            return Ok(());
        }
        let (objects, trampolines) = self.unit.nested.layout(scope, self.scratch);
        let words = 2 + objects.len() as u64;
        let record = self.fresh_object(8 * words, WORD, source)?;
        let chain = chain.unwrap_or_else(|| self.nullary(Opcode::Null));
        self.store_pointer(chain, record);
        let area = if trampolines.is_empty() {
            self.nullary(Opcode::Null)
        } else {
            let area = self.fresh_object(
                TRAMPOLINE_SIZE * trampolines.len() as u64,
                Align::TRAMPOLINE,
                source,
            )?;
            for &(index, func) in &trampolines {
                let storage = self.offset_address(area, TRAMPOLINE_SIZE * u64::from(index));
                self.make_trampoline(storage, func, record);
            }
            area
        };
        let at = self.offset_address(record, 8);
        self.store_pointer(area, at);
        for &(word, binding) in &objects {
            let source = self.unit.sema.bindings[binding].name.source_vectors;
            // A parameter is in place already, where it arrived or in a
            // copy; any other object gets its slot now.
            let local = match self.locals.get(&binding).copied() {
                | Some(local) => local,
                | None => self.local(binding, source)?,
            };
            let address = match local {
                | super::work::Local::Slot(slot) =>
                    self.inst(InstData::StackAddr { slot }, Type::Ptr),
                | super::work::Local::Address(address) => address,
                | super::work::Local::Variable(_) =>
                    return Err(LoweringError::missing(
                        "a captured object is not in memory",
                        source,
                    )),
            };
            let at = self.offset_address(record, 8 * u64::from(word));
            self.store_pointer(address, at);
        }
        self.chain_record = Some(record);
        Ok(())
    }

    /// Checks a nested definition where its definer reaches it: the bounds
    /// of a variably modified type there are the definer's, which GCC
    /// evaluates at the definition and lowering does not yet.
    pub(super) fn nested_definition(
        &self,
        definition: &FunctionDefinition<'_>,
    ) -> Result<(), LoweringError> {
        let variably_modified = self
            .unit
            .sema
            .correspondence
            .declaration(definition.declarator)
            .and_then(|fact| fact.binding)
            .is_some_and(|binding| {
                self.unit
                    .sema
                    .types
                    .variably_modified(self.unit.sema.bindings[binding].ty)
            });
        if variably_modified {
            return Err(LoweringError::unsupported(
                Construct::NestedVariablyModified,
                definition.source_vectors,
            ));
        }
        Ok(())
    }

    /// The address of `binding`, an automatic object of an enclosing
    /// function, read from that function's record.
    pub(super) fn captured_address(
        &mut self,
        binding: usize,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let data = self.unit.sema.bindings[binding];
        let owner = self
            .unit
            .function_scope(data.scope)
            .filter(|&owner| owner != self.record.scope)
            .ok_or_else(|| {
                LoweringError::missing("an automatic object is used outside its function", source)
            })?;
        if self.unit.sema.types.variably_modified(data.ty) {
            return Err(LoweringError::unsupported(
                Construct::NestedVariablyModified,
                source,
            ));
        }
        let word = self.unit.nested.word(owner, binding);
        _ = self.unit.address_taken.insert(binding);
        let record = self.record_of(owner, source)?;
        let at = self.offset_address(record, 8 * u64::from(word));
        Ok(self.load_pointer(at))
    }

    /// The address of the record of the function scope `scope`: this
    /// function's own, or one an enclosing function built, reached through
    /// the first word of each record from this function's static chain.
    pub(super) fn record_of(
        &mut self,
        scope: usize,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let missing = || LoweringError::missing("a static chain does not reach its record", source);
        if scope == self.record.scope {
            return self.chain_record.ok_or_else(missing);
        }
        let mut record = self.chain_in.ok_or_else(missing)?;
        let mut at = self
            .unit
            .enclosing_function(self.record.scope)
            .ok_or_else(missing)?;
        while at != scope {
            record = self.load_pointer(record);
            at = self.unit.enclosing_function(at).ok_or_else(missing)?;
        }
        Ok(record)
    }

    /// The address of the nested function `func`: a trampoline in the frame
    /// of the function that defines it, which calls it with that function's
    /// record. A function inside the definer uses the definer's trampoline
    /// area, which the definer fills on entry; the definer uses that one
    /// too if there is one, or else a slot of its own, made at each use.
    pub(super) fn nested_address(
        &mut self,
        func: FuncId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let Some(definer) = self.unit.nested.definer(func) else {
            return Ok(self.inst(InstData::FuncAddr { func }, Type::Ptr));
        };
        let own = definer == self.record.scope;
        let index = if own {
            self.unit
                .nested
                .trampoline_indexes
                .get(&(definer, func))
                .copied()
        } else {
            Some(self.unit.nested.trampoline(definer, func))
        };
        let storage = if let Some(index) = index {
            let record = self.record_of(definer, source)?;
            let at = self.offset_address(record, 8);
            let area = self.load_pointer(at);
            self.offset_address(area, TRAMPOLINE_SIZE * u64::from(index))
        } else {
            let slot = match self.trampolines.get(&func) {
                | Some(&slot) => slot,
                | None => {
                    let size = u32::try_from(TRAMPOLINE_SIZE).expect("a trampoline is small");
                    let slot = self.object_slot(size, Align::TRAMPOLINE);
                    _ = self.trampolines.insert(func, slot);
                    slot
                },
            };
            let storage = self.inst(InstData::StackAddr { slot }, Type::Ptr);
            let record = self.record_of(definer, source)?;
            self.make_trampoline(storage, func, record);
            storage
        };
        Ok(self.inst(
            InstData::Unary {
                opcode: Opcode::AdjustTrampoline,
                ty:     Type::Ptr,
                arg:    storage,
            },
            Type::Ptr,
        ))
    }

    /// Writes the trampoline at `storage` that calls `func` with `chain`.
    fn make_trampoline(&mut self, storage: Value, func: FuncId, chain: Value) {
        let target = self.inst(InstData::FuncAddr { func }, Type::Ptr);
        _ = self.draft.push(
            self.memory_op(InstData::MemoryRange {
                opcode: Opcode::Trampoline,
                flags:  InstFlags::empty(),
                align:  Align::TRAMPOLINE,
                args:   [storage, target, chain],
            }),
            None,
        );
    }

    /// The static chain a call of the nested function `func` passes, if it
    /// is one.
    pub(super) fn chain_for(
        &mut self,
        func: FuncId,
        source: SourceVectors,
    ) -> Result<Option<Value>, LoweringError> {
        match self.unit.nested.definer(func) {
            | Some(definer) => self.record_of(definer, source).map(Some),
            | None => Ok(None),
        }
    }

    fn load_pointer(&mut self, address: Value) -> Value {
        self.inst(
            InstData::Load {
                ty:    Type::Ptr,
                flags: InstFlags::empty(),
                align: WORD,
                tag:   None,
                addr:  address,
            },
            Type::Ptr,
        )
    }

    fn store_pointer(&mut self, value: Value, address: Value) {
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty:    Type::Ptr,
                flags: InstFlags::empty(),
                align: WORD,
                tag:   None,
                args:  [value, address],
            }),
            None,
        );
    }
}

/// The alignment of a record word.
const WORD: Align = match Align::from_bytes(8) {
    | Some(align) => align,
    | None => Align::BYTE,
};
