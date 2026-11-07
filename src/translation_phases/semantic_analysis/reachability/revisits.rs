//! How the walk repeats without reading the whole function again. A jump
//! backward or a loop's back edge reaches what the walk has passed, so the
//! walk repeats until nothing new is reached; a chain of `n` jumps backward
//! takes `n` repetitions. Each repetition reads only what a new fact can
//! change, so the chain costs `O(n)` instead of `O(n²)`.
//!
//! Each compound, labeled, selection and iteration statement remembers its
//! last visit: the liveness it started with, the innermost switch's entry,
//! what its end was, and which `break` and `continue` it reached. A visit
//! that starts the same way, with nothing new reached inside it, ends the
//! same way, so the walk takes the remembered end. A new fact marks the
//! statement it is about (the label a jump reaches, the loop a back edge
//! repeats) and each statement around it, and a compound resumes at its
//! first marked item and stops again where its liveness rejoins the last
//! visit's. The analysis only grows from pass to pass, which makes this safe:
//! an unchanged start gives an unchanged end.

use super::{
    Analyzer,
    Flow,
    S,
};
use crate::{
    translation_phases::parsing::syntax::{
        BlockItem,
        Statement,
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

/// No compound item: the parent is a statement other than a compound.
const NO_ITEM: u32 = u32::MAX;

/// A local label: its name and the identity of the statement declaring it.
type LocalLabel = (StringCacheId, usize);

/// The jumps out of a statement it reached.
#[derive(Clone, Copy, Default)]
struct JumpsOut {
    /// A `break` of the innermost statement around it that has one.
    breaks:    bool,
    /// A `continue` of the innermost loop around it.
    continues: bool,
}

/// A statement's last visit.
#[derive(Clone, Copy)]
struct Visit {
    /// Whether its start was reachable.
    live:    bool,
    /// Whether the innermost switch's condition was reached, which decides
    /// its case labels.
    entered: Option<bool>,
    /// Whether its end was reachable.
    end:     bool,
    /// The jumps out of it that its visits reached.
    jumps:   JumpsOut,
    /// Something new was reached inside it since the visit began.
    dirty:   bool,
    /// The remembered statement around it, or 0.
    parent:  usize,
    /// Its item's index in `parent`, if that is a compound.
    item:    u32,
    /// A compound's first item's entry in [`Revisits::entries`].
    entries: u32,
}

/// What the walk remembers of its last visits.
pub(super) struct Revisits<'s> {
    scratch: &'s Bump,
    /// Each remembered statement's last visit, by identity.
    visits:  ArenaMap<'s, usize, Visit>,
    /// The remembered statements being visited, innermost last, with the
    /// item a compound is at.
    path:    ArenaVec<'s, (usize, u32)>,
    /// Whether each compound item's start was reachable at its last visit.
    entries: ArenaVec<'s, bool>,
    /// The items of each compound that something new reached, by identity.
    pending: ArenaMap<'s, usize, ArenaVec<'s, u32>>,
    /// The local labels each compound's items declare, with the item.
    locals:  ArenaMap<'s, usize, ArenaVec<'s, (u32, LocalLabel)>>,
    /// Each label's statement, by the key [`Analyzer::label_key`] gives.
    labels:  ArenaMap<'s, (usize, StringCacheId), usize>,
}

impl<'s> Revisits<'s> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            scratch,
            visits: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            path: ArenaVec::new_in(scratch),
            entries: ArenaVec::new_in(scratch),
            pending: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            locals: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            labels: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
        }
    }

    /// Records where a label is, for the jumps that reach it later.
    pub(super) fn note_label(&mut self, label: (usize, StringCacheId), statement: &Statement<'_>) {
        _ = self
            .labels
            .insert(label, std::ptr::from_ref(statement).addr());
    }

    /// A jump newly reaches `label`.
    pub(super) fn reach_label(&mut self, label: (usize, StringCacheId)) {
        if let Some(&statement) = self.labels.get(&label) {
            self.touch(statement);
        }
    }

    /// Something new was reached at `statement`: it and each statement
    /// around it must be visited again. A marked statement's surroundings
    /// already are, until it is visited.
    pub(super) fn touch(&mut self, statement: usize) {
        let mut id = statement;
        while let Some(visit) = self.visits.get_mut(&id) {
            if visit.dirty {
                return;
            }
            visit.dirty = true;
            let (parent, item) = (visit.parent, visit.item);
            if item != NO_ITEM {
                let scratch = self.scratch;
                self.pending
                    .entry(parent)
                    .or_insert_with(|| ArenaVec::new_in(scratch))
                    .push(item);
            }
            id = parent;
        }
    }
}

/// Whether the walk remembers visits to a statement of this kind: those that
/// hold others. A leaf is cheap to visit again.
const fn remembered(kind: &S<'_>) -> bool {
    matches!(
        kind,
        S::Compound { .. }
            | S::Attributed(_)
            | S::Label(..)
            | S::Case(..)
            | S::CaseRange(_)
            | S::Default(_)
            | S::If { .. }
            | S::Switch { .. }
            | S::While { .. }
            | S::DoWhile { .. }
            | S::For(_)
    )
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Whether the end of `statement` is reachable, when its start is if
    /// `live`, from its last visit if nothing it depends on changed.
    /// C99: §6.8, pp. 131-139; PDF pp. 143-151.
    pub(super) fn flow_statement(
        &mut self,
        flow: &mut Flow<'s>,
        statement: &'tu Statement<'tu>,
        live: bool,
    ) -> bool {
        if !remembered(&statement.kind) {
            return self.walk_statement(flow, statement, live);
        }
        let id = std::ptr::from_ref(statement).addr();
        let entered = flow.switches.last().map(|switch| switch.entered);
        let previous = flow.revisits.visits.get(&id).copied();
        if let Some(visit) = previous
            && !visit.dirty
            && visit.live == live
            && visit.entered == entered
        {
            Self::merge_jumps_out(flow, visit.jumps);
            return visit.end;
        }
        let (parent, item) = flow.revisits.path.last().copied().unwrap_or((0, NO_ITEM));
        let outer = Self::take_jumps_out(flow);
        flow.revisits
            .visits
            .entry(id)
            .or_insert(Visit {
                live,
                entered,
                end: false,
                jumps: JumpsOut::default(),
                dirty: false,
                parent,
                item,
                entries: NO_ITEM,
            })
            .dirty = false;
        flow.revisits.path.push((id, NO_ITEM));
        let end = match statement.kind {
            | S::Compound { items } => {
                let resumable = previous.filter(|visit| visit.entered == entered);
                self.flow_compound(flow, id, items.as_slice(), live, resumable)
            },
            | _ => self.walk_statement(flow, statement, live),
        };
        _ = flow.revisits.path.pop();
        let jumps = Self::take_jumps_out(flow);
        let visit = flow
            .revisits
            .visits
            .get_mut(&id)
            .expect("the visit this call recorded");
        visit.live = live;
        visit.entered = entered;
        visit.end = end;
        visit.jumps.breaks |= jumps.breaks;
        visit.jumps.continues |= jumps.continues;
        let inner = visit.jumps;
        Self::merge_jumps_out(flow, outer);
        Self::merge_jumps_out(flow, inner);
        end
    }

    /// A compound statement's items in order. A visit that starts as the
    /// last one did reads only the items something new reached, and those
    /// after them until the liveness is the last visit's again.
    fn flow_compound(
        &mut self,
        flow: &mut Flow<'s>,
        id: usize,
        items: &'tu [BlockItem<'tu>],
        live: bool,
        previous: Option<Visit>,
    ) -> bool {
        let scope = flow.local_labels.len();
        let scratch = flow.revisits.scratch;
        let mut pending = flow
            .revisits
            .pending
            .insert(id, ArenaVec::new_in(scratch))
            .unwrap_or_else(|| ArenaVec::new_in(scratch));
        let end = match previous {
            | Some(visit) if !items.is_empty() => {
                pending.sort_unstable();
                pending.dedup();
                self.resume_compound(flow, id, items, live, visit, &pending)
            },
            | _ => self.walk_compound(flow, id, items, live, previous.is_none()),
        };
        flow.local_labels.truncate(scope);
        end
    }

    /// Each item of a compound, the first time or when the innermost switch
    /// was newly entered, which can reach any of its case labels.
    fn walk_compound(
        &mut self,
        flow: &mut Flow<'s>,
        id: usize,
        items: &'tu [BlockItem<'tu>],
        live: bool,
        first: bool,
    ) -> bool {
        let base = Self::compound_entries(flow, id, items.len());
        let mut live = live;
        for (index, item) in items.iter().enumerate() {
            let declared = flow.local_labels.len();
            Self::enter_item(flow, base, index, live);
            live = self.flow_item(flow, item, live);
            if first && flow.local_labels.len() > declared {
                let scratch = flow.revisits.scratch;
                let locals = flow
                    .revisits
                    .locals
                    .entry(id)
                    .or_insert_with(|| ArenaVec::new_in(scratch));
                for &local in &flow.local_labels[declared..] {
                    locals.push((u32::try_from(index).unwrap_or(NO_ITEM), local));
                }
            }
        }
        live
    }

    /// The items of a compound that something new reached, or each one
    /// from the first if its start changed.
    fn resume_compound(
        &mut self,
        flow: &mut Flow<'s>,
        id: usize,
        items: &'tu [BlockItem<'tu>],
        live: bool,
        previous: Visit,
        pending: &[u32],
    ) -> bool {
        let base = previous.entries as usize;
        let mut end = previous.end;
        let mut waiting = pending.iter().map(|&item| item as usize).peekable();
        let mut next = if previous.live == live {
            waiting.peek().copied()
        } else {
            Some(0)
        };
        let mut declared = 0;
        while let Some(start) = next {
            let mut current = if start == 0 {
                live
            } else {
                flow.revisits.entries[base + start]
            };
            let mut index = start;
            loop {
                declared = Self::declare_locals_before(flow, id, declared, index);
                Self::enter_item(flow, base, index, current);
                current = self.flow_item(flow, &items[index], current);
                index += 1;
                while waiting.next_if(|&item| item < index).is_some() {}
                if index == items.len() {
                    end = current;
                    break;
                }
                if flow.revisits.entries[base + index] == current && waiting.peek() != Some(&index)
                {
                    break;
                }
            }
            next = waiting.peek().copied();
        }
        end
    }

    fn flow_item(&mut self, flow: &mut Flow<'s>, item: &'tu BlockItem<'tu>, live: bool) -> bool {
        match *item {
            | BlockItem::Statement(child) => self.flow_statement(flow, child, live),
            | BlockItem::Declaration(declaration) => self.flow_declaration(flow, declaration, live),
            | BlockItem::FunctionDefinition(_) => live,
        }
    }

    /// Where a compound's items' entries are, allocated on its first visit.
    fn compound_entries(flow: &mut Flow<'s>, id: usize, count: usize) -> usize {
        let revisits = &mut flow.revisits;
        let visit = revisits.visits.get_mut(&id).expect("the compound's visit");
        if visit.entries == NO_ITEM {
            visit.entries = u32::try_from(revisits.entries.len()).unwrap_or(NO_ITEM);
            revisits.entries.extend(std::iter::repeat_n(false, count));
        }
        visit.entries as usize
    }

    fn enter_item(flow: &mut Flow<'s>, base: usize, index: usize, live: bool) {
        let revisits = &mut flow.revisits;
        revisits.entries[base + index] = live;
        if let Some(top) = revisits.path.last_mut() {
            top.1 = u32::try_from(index).unwrap_or(NO_ITEM);
        }
    }

    /// Declares the local labels of the items before `index` that a resumed
    /// visit skips, from the `declared`th on; the item at `index` declares
    /// its own.
    fn declare_locals_before(
        flow: &mut Flow<'s>,
        id: usize,
        declared: usize,
        index: usize,
    ) -> usize {
        let Some(locals) = flow.revisits.locals.get(&id) else {
            return declared;
        };
        let mut declared = declared;
        while let Some(&(item, local)) = locals.get(declared)
            && item as usize <= index
        {
            if (item as usize) < index {
                flow.local_labels.push(local);
            }
            declared += 1;
        }
        declared
    }

    /// The jumps out the innermost target and loop hold, which it clears for
    /// a statement to set its own.
    fn take_jumps_out(flow: &mut Flow<'s>) -> JumpsOut {
        let breaks = flow
            .targets
            .last_mut()
            .is_some_and(|target| std::mem::take(&mut target.breaks));
        let continues = flow
            .targets
            .iter_mut()
            .rev()
            .find(|target| target.is_loop)
            .is_some_and(|target| std::mem::take(&mut target.continues));
        JumpsOut { breaks, continues }
    }

    fn merge_jumps_out(flow: &mut Flow<'s>, jumps: JumpsOut) {
        if jumps.breaks
            && let Some(target) = flow.targets.last_mut()
        {
            target.breaks = true;
        }
        if jumps.continues
            && let Some(target) = flow.targets.iter_mut().rev().find(|t| t.is_loop)
        {
            target.continues = true;
        }
    }
}
