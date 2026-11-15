//! TCC's `#pragma comment(option, "...")` sets a bit-field layout that
//! crosses the batch boundary by token ordinal, as `#pragma pack` does
//! (`pragma_pack.rs`). Only the TCC identity retains it; semantic analysis
//! reads the layout in effect where a record definition ends, where TCC
//! lays the record out.
//! C99: §6.10.6p1, p. 159; PDF p. 171 (implementation-defined pragma).
//! TCC: `pragma_parse` in `tccpp.c` hands the string to `tcc_set_options`.

/// The bit-field layout an option pragma selects for the records whose
/// definitions end after it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LayoutPragma {
    /// The output-token ordinal before which the pragma appeared.
    pub(crate) position:     usize,
    /// `-mms-bitfields` (true) or `-mno-ms-bitfields` (false).
    pub(crate) ms_bitfields: bool,
}

/// The bit-field layout in effect at `position` by the last of `pragmas`
/// before it, or `configured` (the command line's, `None` for the
/// target's) before any.
pub(crate) fn ms_bitfields_at(
    pragmas: &[LayoutPragma],
    position: usize,
    configured: Option<bool>,
) -> Option<bool> {
    let effective = pragmas.partition_point(|pragma| pragma.position <= position);
    effective
        .checked_sub(1)
        .map_or(configured, |index| Some(pragmas[index].ms_bitfields))
}
