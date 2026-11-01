//! GCC's `#pragma scalar_storage_order` crosses the batch boundary by token
//! ordinal, as `#pragma pack` does (`pragma_pack.rs`). Only the GCC identity
//! retains it; semantic analysis reads the order in effect where a record
//! definition ends.
//! C99: §6.10.6p1, p. 159; PDF p. 171 (implementation-defined pragma).
//! GNU extension: GCC manual, "Scalar Storage Order Pragmas".
//! <https://gcc.gnu.org/onlinedocs/gcc/Scalar-Storage-Order-Pragmas.html>

use super::SourceVectors;

/// The storage order a pragma selects for the records defined after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageOrder {
    /// `default`: the target's own byte order.
    Default,
    BigEndian,
    LittleEndian,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StorageOrderPragma {
    /// The output-token ordinal before which the pragma appeared.
    pub(crate) position: usize,
    pub(crate) order:    StorageOrder,
    pub(crate) source:   SourceVectors,
}
