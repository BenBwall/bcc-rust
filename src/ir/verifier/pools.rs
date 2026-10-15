//! Validates length-prefixed operand pools before type, edge or dominance
//! checks use the body's unchecked list accessors. Malformed encodings stop
//! dependent checks for that function, after reporting all broken handles.

use super::{
    BlockCall,
    Body,
    Entity,
    EntityKind,
    Inst,
    InstData,
    Location,
    Value,
    Verifier,
    VerifierErrorKind,
};

impl Verifier<'_, '_, '_, '_> {
    /// Check every placed instruction's pool references, including the
    /// nested block calls and constant references of switch cases.
    pub(super) fn pools(&mut self) -> bool {
        let body = self.body;
        let mut sound = true;
        for block in body.blocks() {
            for &inst in body.block_insts(block) {
                let valid = match *body.inst(inst) {
                    | InstData::Call { args, .. } | InstData::CallIndirect { args, .. } =>
                        self.pool_list(inst, EntityKind::ValueList, args.0, 1, 1),
                    | InstData::Jump { dest } => self.pool_edge(inst, dest),
                    | InstData::Brif { dests, .. } => {
                        let first = self.pool_edge(inst, dests[0]);
                        let second = self.pool_edge(inst, dests[1]);
                        first && second
                    },
                    | InstData::Switch { default, cases, .. } => {
                        let mut valid = self.pool_edge(inst, default);
                        if let Some(pairs) = entries(body, cases.0, 1, 2) {
                            // The encoding bounds are checked above. Inspect
                            // raw pairs so a bad constant never reaches the
                            // unchecked switch_cases accessor.
                            for pair in pairs.as_chunks::<2>().0 {
                                if pair[0].index() >= body.constant_count() {
                                    self.error(
                                        Location::Inst(inst),
                                        VerifierErrorKind::InvalidReference(
                                            EntityKind::Constant,
                                            pair[0].as_u32(),
                                        ),
                                    );
                                    valid = false;
                                }
                                valid &= self.pool_edge(inst, BlockCall(pair[1].as_u32()));
                            }
                        } else {
                            self.error(
                                Location::Inst(inst),
                                VerifierErrorKind::InvalidReference(EntityKind::CaseList, cases.0),
                            );
                            valid = false;
                        }
                        valid
                    },
                    | InstData::IndirectBr { dests, .. } => {
                        if let Some(calls) = entries(body, dests.0, 1, 1) {
                            let mut valid = true;
                            for call in calls {
                                valid &= self.pool_edge(inst, BlockCall(call.as_u32()));
                            }
                            valid
                        } else {
                            self.error(
                                Location::Inst(inst),
                                VerifierErrorKind::InvalidReference(EntityKind::BlockList, dests.0),
                            );
                            false
                        }
                    },
                    | _ => true,
                };
                sound &= valid;
            }
        }
        sound
    }

    fn pool_edge(&mut self, inst: Inst, call: BlockCall) -> bool {
        self.pool_list(inst, EntityKind::BlockCall, call.0, 2, 1)
    }

    fn pool_list(
        &mut self,
        inst: Inst,
        kind: EntityKind,
        offset: u32,
        header: usize,
        stride: usize,
    ) -> bool {
        if entries(self.body, offset, header, stride).is_some() {
            true
        } else {
            self.error(
                Location::Inst(inst),
                VerifierErrorKind::InvalidReference(kind, offset),
            );
            false
        }
    }
}

/// Check the length slot, the complete header and the encoded payload using
/// checked arithmetic; even zero-length lists must include their headers.
fn entries<'a>(
    body: &'a Body<'_>,
    offset: u32,
    header: usize,
    stride: usize,
) -> Option<&'a [Value]> {
    let start = offset as usize;
    let len = body.pool.get(start)?.index().checked_mul(stride)?;
    let payload = start.checked_add(header)?;
    let end = payload.checked_add(len)?;
    body.pool.get(payload..end)
}
