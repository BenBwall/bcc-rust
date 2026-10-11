//! What ABI lowering reports instead of lowering a module.

use std::fmt;

use crate::ir::Type;

/// A case ABI lowering does not handle, in the function where it appears.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct AbiError<'ir> {
    pub(crate) function: &'ir str,
    pub(crate) kind:     AbiErrorKind,
}

/// What ABI lowering does not handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AbiErrorKind {
    /// A `va_arg` of a type the target's expansion does not read yet.
    VaArgType(Type),
    /// An aggregate whose low eightbyte has no class but whose high one
    /// does, which no C type without vector or attribute extensions has.
    EmptyLowEightbyte,
}

impl fmt::Display for AbiError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}: ", self.function)?;
        match self.kind {
            | AbiErrorKind::VaArgType(ty) => write!(
                f,
                "va_arg of type {ty} is not yet supported by ABI lowering for this target"
            ),
            | AbiErrorKind::EmptyLowEightbyte => f.write_str(
                "an aggregate whose first eightbyte holds no member is not yet supported by ABI \
                 lowering",
            ),
        }
    }
}
