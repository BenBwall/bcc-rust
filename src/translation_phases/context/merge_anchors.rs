use super::{
    Context,
    SourceVectors,
};

/// Which ranges [`Context::merge_vector_list`] keeps, decided in order
/// without collecting them: empty ranges are dropped, and parser anchors are
/// dropped after the first real range, or before it when they point at its
/// start.
#[derive(Clone, Copy)]
pub(super) struct MergeAnchors {
    /// Whether any range is an anchor; otherwise every nonempty range stays.
    pub(super) any:        bool,
    pub(super) first_real: Option<SourceVectors>,
    pub(super) real_seen:  bool,
}

impl MergeAnchors {
    /// Whether the next range in order, `source`, is kept.
    pub(super) fn keeps(&mut self, context: &Context<'_>, source: SourceVectors) -> bool {
        if source.length() == 0 {
            return false;
        }
        if !self.any {
            return true;
        }
        if context.is_parser_anchor(source) {
            !(self.real_seen
                || self
                    .first_real
                    .is_some_and(|first| context.anchors_start_of(source, first)))
        } else {
            self.real_seen = true;
            true
        }
    }

    pub(super) fn new(context: &Context<'_>, sources: &[SourceVectors]) -> Self {
        let any = sources
            .iter()
            .any(|source| context.is_parser_anchor(*source));
        Self {
            any,
            first_real: any
                .then(|| {
                    sources
                        .iter()
                        .find(|source| source.length() != 0 && !context.is_parser_anchor(**source))
                        .copied()
                })
                .flatten(),
            real_seen: false,
        }
    }
}
