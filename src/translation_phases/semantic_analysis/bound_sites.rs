//! Type-site evaluation facts produced during semantic type construction.
//! GNU extension: GCC Typeof evaluates operands of variably modified type.
//! <https://gcc.gnu.org/onlinedocs/gcc/Typeof.html>
//! C99: §6.5.3.4p2, p. 80; PDF p. 92 (runtime size operands).

use super::{
    Analyzer,
    ExtendedType,
    SyntaxOperand,
    TypeSpecifiers,
};
use crate::translation_phases::parsing::syntax::Expression;

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Keep execution at the written type site: typedef reuse does not execute
    /// the original typeof operand again. Nested type operands copy their
    /// retained actions; expression operands execute only when their type is
    /// VM. GNU extension: GCC Typeof; C99: §6.7.5.2p5, pp. 116-117;
    /// PDF pp. 128-129 (remembered array extents).
    pub(super) fn finish_typeof(&mut self, syntax: &'tu ExtendedType<'tu>) {
        let ExtendedType::Typeof {
            operand,
            unqualified,
        } = *syntax
        else {
            return;
        };
        let ty = self.operand_type(operand);
        let evaluations = if self.types.variably_modified(ty) {
            match operand {
                | SyntaxOperand::Expression(expression) =>
                    self.types.tu.alloc_slice_copy(&[expression]),
                | SyntaxOperand::Type(name) => {
                    let Some(site) = self.type_name_sites.get(&std::ptr::from_ref(name).addr())
                    else {
                        self.values.push(self.types.unknown());
                        return;
                    };
                    site.operand_evaluations
                },
            }
        } else {
            &[]
        };
        _ = self
            .typeof_evaluations
            .insert(std::ptr::from_ref(syntax).addr(), evaluations);
        let ty = if unqualified {
            self.types.unqualified_array(self.types.non_atomic(ty))
        } else {
            ty
        };
        self.values.push(ty);
    }

    /// The producer consumes an immediate specifier identity, never provenance.
    pub(super) fn spec_evaluations(
        &self,
        spec: TypeSpecifiers<'tu>,
    ) -> Option<&'tu [&'tu Expression<'tu>]> {
        if let TypeSpecifiers::Extended(syntax @ ExtendedType::Typeof { .. }) = spec {
            self.typeof_evaluations
                .get(&std::ptr::from_ref(syntax).addr())
                .copied()
        } else {
            Some(&[])
        }
    }
}
