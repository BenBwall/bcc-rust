//! Iterative heavy-light indexing. Construction visits each region a bounded
//! number of times; LCA and first-entered-VM queries cross only light edges.
//! C99: §6.8.6.1p1, p. 137; PDF p. 149 (VM entry diagnostics).

use super::{
    Region,
    RegionId,
    RegionRun,
    Transition,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

pub(in crate::translation_phases::semantic_analysis) struct Ancestry<'s> {
    head: ArenaVec<'s, RegionId>,
    depth: ArenaVec<'s, usize>,
    position: ArenaVec<'s, usize>,
    order: ArenaVec<'s, RegionId>,
    #[cfg(test)]
    pub(in crate::translation_phases::semantic_analysis) construction_work: usize,
    #[cfg(test)]
    pub(in crate::translation_phases::semantic_analysis) query_work: std::cell::Cell<usize>,
}

impl<'s> Ancestry<'s> {
    pub(in crate::translation_phases::semantic_analysis) fn new(
        regions: &[Region],
        scratch: &'s Bump,
    ) -> Self {
        let count = regions.len();
        #[cfg(test)]
        let mut construction_work = 0;
        let mut size = ArenaVec::new_in(scratch);
        size.resize(count, 1usize);
        let mut heavy = ArenaVec::new_in(scratch);
        heavy.resize(count, None::<RegionId>);
        for i in (0..count).rev() {
            #[cfg(test)]
            {
                construction_work += 1;
            }
            if let Some(parent) = regions[i].parent {
                size[parent.0] += size[i];
                if heavy[parent.0].is_none_or(|h| size[h.0] < size[i]) {
                    heavy[parent.0] = Some(RegionId(i));
                }
            }
        }
        let mut head = ArenaVec::new_in(scratch);
        let mut depth = ArenaVec::new_in(scratch);
        for (i, region) in regions.iter().enumerate() {
            #[cfg(test)]
            {
                construction_work += 1;
            }
            depth.push(region.parent.map_or(0, |p| depth[p.0] + 1));
            head.push(
                region
                    .parent
                    .filter(|p| heavy[p.0] == Some(RegionId(i)))
                    .map_or(RegionId(i), |p| head[p.0]),
            );
        }
        // Group each heavy chain into a contiguous run. Stable region order
        // already orders the vertices within a chain by increasing depth.
        let mut length = ArenaVec::new_in(scratch);
        length.resize(count, 0usize);
        for h in &head {
            #[cfg(test)]
            {
                construction_work += 1;
            }
            length[h.0] += 1;
        }
        let mut starts = ArenaVec::new_in(scratch);
        let mut end = 0;
        for &len in &length {
            #[cfg(test)]
            {
                construction_work += 1;
            }
            starts.push(end);
            end += len;
        }
        let mut order = ArenaVec::new_in(scratch);
        order.resize(count, RegionId(0));
        let mut position = ArenaVec::new_in(scratch);
        for (i, h) in head.iter().enumerate() {
            #[cfg(test)]
            {
                construction_work += 1;
            }
            let pos = starts[h.0];
            starts[h.0] += 1;
            order[pos] = RegionId(i);
            position.push(pos);
        }
        Self {
            head,
            depth,
            position,
            order,
            #[cfg(test)]
            construction_work,
            #[cfg(test)]
            query_work: std::cell::Cell::new(0),
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn lca(
        &self,
        regions: &[Region],
        mut a: RegionId,
        mut b: RegionId,
    ) -> RegionId {
        while self.head[a.0] != self.head[b.0] {
            self.step();
            if self.depth[self.head[a.0].0] < self.depth[self.head[b.0].0] {
                std::mem::swap(&mut a, &mut b);
            }
            a = regions[self.head[a.0].0]
                .parent
                .expect("points share a function root");
        }
        self.step();
        if self.depth[a.0] < self.depth[b.0] {
            a
        } else {
            b
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn transition(
        &self,
        regions: &[Region],
        from: RegionId,
        to: RegionId,
    ) -> Transition {
        let common = self.lca(regions, from, to);
        Transition {
            left:    RegionRun {
                first:       from,
                stop_before: common,
            },
            entered: RegionRun {
                first:       to,
                stop_before: common,
            },
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn at_depth(
        &self,
        regions: &[Region],
        mut region: RegionId,
        depth: usize,
    ) -> RegionId {
        while self.depth[self.head[region.0].0] > depth {
            self.step();
            region = regions[self.head[region.0].0]
                .parent
                .expect("requested ancestor exists");
        }
        self.step();
        self.order[self.position[region.0] - (self.depth[region.0] - depth)]
    }

    pub(in crate::translation_phases::semantic_analysis) fn first_entered_vm(
        &self,
        regions: &[Region],
        transition: Transition,
    ) -> Option<RegionId> {
        self.first_entered(regions, transition, |region| region.nearest_vm)
    }

    /// The outermost entered GNU cleanup declaration, found as VM regions are.
    pub(in crate::translation_phases::semantic_analysis) fn first_entered_cleanup(
        &self,
        regions: &[Region],
        transition: Transition,
    ) -> Option<RegionId> {
        self.first_entered(regions, transition, |region| {
            region.nearest_cleanup_declaration
        })
    }

    fn first_entered(
        &self,
        regions: &[Region],
        transition: Transition,
        nearest_of: impl Fn(&Region) -> Option<RegionId>,
    ) -> Option<RegionId> {
        let common = transition.entered.stop_before;
        let nearest = nearest_of(&regions[transition.entered.first.0])?;
        if ancestor(regions, nearest, common) {
            return None;
        }
        // Find the first vertex below common that inherits a VM newer than
        // common. Heavy chains are contiguous, allowing a binary search on
        // one chain after at most logarithmically many light-edge steps.
        let mut vertex = transition.entered.first;
        loop {
            self.step();
            let h = self.head[vertex.0];
            if self.depth[h.0] <= self.depth[common.0] {
                break;
            }
            let parent = regions[h.0].parent.expect("entered path has a parent");
            if nearest_of(&regions[parent.0]).is_none_or(|vm| ancestor(regions, vm, common)) {
                break;
            }
            vertex = parent;
        }
        let h = self.head[vertex.0];
        let mut low = if self.depth[h.0] <= self.depth[common.0] {
            self.position[self.at_depth(regions, vertex, self.depth[common.0] + 1).0]
        } else {
            self.position[h.0]
        };
        let mut high = self.position[vertex.0];
        while low < high {
            self.step();
            let mid = low + (high - low) / 2;
            let new_vm = nearest_of(&regions[self.order[mid].0])
                .is_some_and(|vm| !ancestor(regions, vm, common));
            if new_vm {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        nearest_of(&regions[self.order[low].0])
    }

    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "The test-only query counter follows the production algorithm."
        )
    )]
    fn step(&self) {
        #[cfg(test)]
        self.query_work.set(self.query_work.get() + 1);
    }
}

pub(in crate::translation_phases::semantic_analysis) fn ancestor(
    regions: &[Region],
    from: RegionId,
    to: RegionId,
) -> bool {
    let a = regions[from.0];
    let b = regions[to.0];
    a.function == b.function && a.preorder <= b.preorder && b.preorder < a.subtree_end
}
