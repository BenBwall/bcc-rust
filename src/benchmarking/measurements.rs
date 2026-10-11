use super::{
    BenchmarkInput,
    Bump,
    Path,
    benchmark_context,
    with_prepared_parse,
    with_preprocessor,
};

/// Preprocesses and parses `input` to report its arena high-water marks, then
/// compiles it through phase 7 to report its peak regions and commit.
#[doc(hidden)]
#[must_use]
pub fn arena_usage(input: BenchmarkInput) -> ArenaUsage {
    use crate::util::vm::accounting;
    let (preprocessor_high_water, expansion_high_water) = {
        let tu = Bump::new();
        let mut context = benchmark_context(&tu);
        with_preprocessor(
            &mut context,
            Path::new("<input>"),
            input.source(),
            crate::headers::HeaderSearch::default(),
            |mut preprocessor, context, pp| {
                let mut tokens = crate::util::region_vec::RegionVec::new();
                let _ = preprocessor.preprocess_into_arena(context, usize::MAX, &mut tokens);
                (pp.high_water(), preprocessor.expansion_high_water())
            },
        )
    };
    let parse_high_water = with_prepared_parse(input, |prepared| {
        let parse = prepared.parse;
        _ = prepared.parse();
        parse.high_water()
    });
    let before = accounting::live();
    accounting::reset_peak();
    let tu_high_water = {
        let tu = Bump::new();
        let mut context = benchmark_context(&tu);
        let unit = crate::pipeline::parse_translation_unit(
            &mut context,
            Path::new("<input>"),
            input.source(),
            crate::headers::HeaderSearch::default(),
        );
        _ = unit.external_declarations().len();
        tu.high_water()
    };
    let peak = accounting::peak();
    ArenaUsage {
        preprocessor_high_water,
        expansion_high_water,
        parse_high_water,
        tu_high_water,
        peak_regions: peak.regions - before.regions,
        peak_reserved: peak.reserved - before.reserved,
        peak_committed: peak.committed - before.committed,
    }
}

/// What the arenas of one compilation held, measured in-process.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaUsage {
    /// The preprocessing arena's high-water mark over phases 4 to 6 (the
    /// plan's `'pp` peak), lexed files included.
    pub preprocessor_high_water: usize,
    /// The expansion arena's high-water mark; it is reset between
    /// top-level expansions.
    pub expansion_high_water:    usize,
    /// The parse arena's high-water mark over phase 7 (the plan's `'parse`
    /// peak): frames, their pools, scopes, and recovery state.
    pub parse_high_water:        usize,
    /// The translation-unit arena's high-water mark after phase 7: source
    /// text, diagnostics, and everything else that lives as long as the unit.
    pub tu_high_water:           usize,
    /// The most virtual-memory regions live at once over phases 1 to 7.
    pub peak_regions:            usize,
    /// The most address space those regions reserved at once.
    pub peak_reserved:           usize,
    /// The most bytes committed across all regions at once.
    pub peak_committed:          usize,
}
