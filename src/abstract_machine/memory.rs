//! Object identity, lifetime and scalar arithmetic checks, without host
//! pointers.
//!
//! C99: §6.2.4 paragraph 2, p. 32; PDF p. 44 (expired objects).
//! C99: §6.7.8 paragraph 10, p. 126; PDF p. 138 (indeterminate locals).
//! C99: §6.5 paragraph 5, p. 67; PDF p. 79 (signed overflow).
//! C99: §6.2.5 paragraph 9, p. 34; PDF p. 46 (unsigned arithmetic wraps).
//! C99: §6.5.7 paragraphs 3-5, pp. 84-85; PDF pp. 96-97 (shifts).
//! C99: §6.5.5 paragraph 5, p. 82; PDF p. 94 (division by zero).
//! C99: §6.5.6 paragraph 8, p. 83; PDF p. 95 (pointer arithmetic).
//! C99: §6.5.6 paragraph 7, p. 83; PDF p. 95 (scalar pointer arithmetic).
//! C99: §6.7.8 paragraph 21, p. 127; PDF p. 139 (partial initialization).
//! C99: §6.5.3.2 paragraph 4, p. 79; PDF p. 91 (invalid indirection).
//! C99: §6.2.6.1 paragraph 6, p. 38; PDF p. 50 (structure values).
//! C99: §6.2.4 paragraph 3, p. 32; PDF p. 44 (static storage duration).

use super::{
    clauses,
    integers::Integer,
    layout,
    machine::{
        Machine,
        ObjectMetadata,
    },
    model::{
        Diagnostic,
        MAX_CELLS,
        MAX_OBJECTS,
        Object,
        Place,
        Program,
        Type,
        Undefined,
        Value,
    },
};

impl Machine {
    /// Create the automatic object `name` of type `ty` in the innermost
    /// call, holding `value` (an aggregate starts with every scalar
    /// indeterminate unless `value` is a whole structure).
    pub fn allocate(
        &mut self,
        name: &str,
        ty: Type,
        scope: usize,
        value: Value,
        parameter: bool,
    ) -> Result<usize, Diagnostic> {
        let value = match (ty, value) {
            | (Type::Array(..) | Type::Struct(_), Value::Indeterminate) =>
                ty.indeterminate(&self.structures),
            | (pointer, Value::Integer(0)) if pointer.is_pointer() => Value::null(),
            // A parameter receives its argument converted as if by
            // assignment (C99 §6.5.2.2p7, §6.3.1.3).
            | (Type::Integer(integer), Value::Integer(value)) =>
                Value::Integer(integer.convert(value)),
            | (_, value) => value,
        };
        let top = self.activations.len() - 1;
        let frame = self.activations[top].id;
        self.create(name, ty, frame, scope, value, parameter, None, false)
    }

    /// Create static object `index` of the program with its initial value.
    pub fn allocate_static(
        &mut self,
        program: &Program,
        index: usize,
    ) -> Result<usize, Diagnostic> {
        let object = &program.statics[index];
        let id = self.create(
            &object.name,
            object.ty,
            0,
            0,
            object.value.clone(),
            false,
            object.function.clone(),
            object.literal,
        )?;
        if object.constant {
            self.make_const(id);
        }
        Ok(id)
    }

    /// Mark object `id` as defined `const`: its type is spelled with the
    /// qualifier, and a store into it after its initialization stops.
    pub fn make_const(&mut self, id: usize) {
        self.metadata[id - 1].constant = true;
        let object = &mut self.state.objects[id - 1];
        object.r#type.insert_str(0, "const ");
    }

    /// Whether the program may store into the object `target`: modifying
    /// an object defined `const` through an lvalue without the qualifier,
    /// which only a cast that removed the `const` can form here, is
    /// undefined. Direct assignments to a `const` object were rejected at
    /// compilation, as the constraints require.
    /// C99: §6.7.3 paragraph 5, p. 109; PDF p. 121.
    pub fn check_writable(&self, target: usize, offset: i32) -> Result<(), Diagnostic> {
        if !self
            .metadata
            .get(target.wrapping_sub(1))
            .is_some_and(|metadata| metadata.constant)
        {
            return Ok(());
        }
        let object = &self.state.objects[target - 1];
        Err(Diagnostic::ub(
            self.state.source_range,
            &format!(
                "{} belongs to `{}`, an object defined {}, and this store reaches it through an \
                 lvalue that is not const: the cast that formed the pointer removed the const, \
                 but not the promise",
                self.location(target, offset),
                object.name,
                object.r#type
            ),
            &clauses::CONST_WRITE,
            Undefined::ConstWrite,
        ))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One object's complete description."
    )]
    fn create(
        &mut self,
        name: &str,
        ty: Type,
        frame_id: usize,
        scope: usize,
        value: Value,
        parameter: bool,
        function: Option<String>,
        literal: bool,
    ) -> Result<usize, Diagnostic> {
        let cells = ty.leaves(&self.structures);
        if self.state.objects.len() >= MAX_OBJECTS || self.cells + cells > MAX_CELLS {
            return Err(Diagnostic::limit(
                self.state.source_range,
                "object or memory cell limit reached",
            ));
        }
        let id = self.state.objects.len() + 1;
        let static_storage = frame_id == 0;
        self.state.objects.push(Object {
            id,
            name: String::from(name),
            r#type: ty.spelling(&self.structures),
            size: ty.size_align(&self.structures).0,
            frame_id,
            storage_duration: if static_storage {
                "static"
            } else {
                "automatic"
            },
            function,
            lifetime: "alive",
            parameter,
            literal,
            value,
        });
        self.metadata.push(ObjectMetadata {
            scope,
            ty,
            constant: false,
        });
        self.cells += cells;
        Ok(id)
    }

    /// The value stored at a location, without lifetime or bounds checks.
    pub fn peek(&self, target: usize, offset: i32) -> Option<Value> {
        let object = self.state.objects.get(target.checked_sub(1)?)?;
        layout::leaf(&object.value, usize::try_from(offset).ok()?).cloned()
    }

    /// The declared type of an object.
    pub fn object_type(&self, target: usize) -> Option<Type> {
        self.metadata
            .get(target.checked_sub(1)?)
            .map(|metadata| metadata.ty)
    }

    /// The type of the scalar at leaf `offset` of an object.
    pub fn leaf_type(&self, target: usize, offset: i32) -> Option<Type> {
        self.object_type(target)?
            .leaf_type(&self.structures, usize::try_from(offset).ok()?)
    }

    /// The object a name designates: an automatic object of the innermost
    /// call, or a static object, which the program created before `main`.
    pub fn named(&self, place: &Place) -> Result<usize, Diagnostic> {
        let name = match place {
            | Place::Static(index) => return Ok(index + 1),
            | Place::Local(name) => name,
        };
        let top = self.activations.len() - 1;
        self.state
            .objects
            .iter()
            .rev()
            .find(|object| {
                object.name == *name
                    && object.frame_id == self.activations[top].id
                    && object.lifetime == "alive"
            })
            .map(|object| object.id)
            .ok_or_else(|| self.error("identifier has no live object in this activation"))
    }

    pub fn read(&self, target: usize, offset: i32) -> Result<Value, Diagnostic> {
        let index = self.checked_location(target, offset)?;
        let object = &self.state.objects[target - 1];
        let value = layout::leaf(&object.value, index)
            .cloned()
            .unwrap_or(Value::Indeterminate);
        if value == Value::Indeterminate {
            // C99 indeterminate values may be unspecified or trap
            // representations; the machine stops conservatively, without
            // asserting that every indeterminate read is undefined.
            return Err(Diagnostic::indeterminate(
                self.state.source_range,
                &format!(
                    "nothing has been stored in {} yet, so its value is indeterminate; the \
                     machine stops instead of making up a value",
                    self.location(target, offset)
                ),
                &clauses::INDETERMINATE,
            ));
        }
        self.check_pointer_lifetime(&value)?;
        Ok(value)
    }

    /// The value of the whole structure `structure` at leaf `offset`: a copy
    /// of every member, indeterminate ones included, since a structure's
    /// value is never a trap representation (C99 §6.2.6.1p6).
    pub fn read_structure(
        &self,
        target: usize,
        offset: i32,
        structure: usize,
    ) -> Result<Value, Diagnostic> {
        let index = self.checked_location(target, offset)?;
        let object = &self.state.objects[target - 1];
        let ty = Type::Struct(structure);
        let mut leaves = Vec::new();
        layout::flatten(&object.value, &mut leaves);
        let end = (index + ty.leaves(&self.structures)).min(leaves.len());
        Ok(ty.shaped(&self.structures, &leaves[index..end]))
    }

    /// Stores `value`, converted to the object's type, and returns the stored
    /// value. An unsigned type reduces it modulo 2^N; a signed type that
    /// cannot hold it keeps the low N bits as two's complement, the
    /// implementation-defined result x86-64 compilers give. A structure is
    /// stored member by member.
    /// C99: §6.3.1.3 paragraphs 1-3, p. 43; PDF p. 55.
    pub fn store(
        &mut self,
        target: usize,
        offset: i32,
        mut value: Value,
    ) -> Result<Value, Diagnostic> {
        let index = self.checked_location(target, offset)?;
        let object = &self.state.objects[target - 1];
        if object.literal {
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "{} is an element of the string literal {}, whose array the program must not \
                     modify",
                    self.location(target, offset),
                    object.name
                ),
                &clauses::STRING_LITERAL_WRITE,
                Undefined::StringLiteralWrite,
            ));
        }
        if let Value::Struct { .. } = value {
            let mut leaves = Vec::new();
            layout::flatten(&value, &mut leaves);
            let object = &mut self.state.objects[target - 1].value;
            for (at, leaf) in leaves.into_iter().enumerate() {
                if let Some(slot) = layout::leaf_mut(object, index + at) {
                    *slot = leaf;
                }
            }
            return Ok(value);
        }
        let ty = self.leaf_type(target, offset);
        if ty.is_some_and(Type::is_pointer) && value == Value::Integer(0) {
            value = Value::null();
        }
        if let (Some(Type::Integer(integer)), Value::Integer(number)) = (ty, &value) {
            value = Value::Integer(integer.convert(*number));
        }
        let valid = if ty.is_some_and(Type::is_pointer) {
            matches!(value, Value::Pointer { .. })
        } else {
            matches!(value, Value::Integer(_))
        };
        if !valid {
            return Err(self.error("store type mismatch"));
        }
        self.check_pointer_lifetime(&value)?;
        if let Some(slot) = layout::leaf_mut(&mut self.state.objects[target - 1].value, index) {
            *slot = value.clone();
        }
        Ok(value)
    }

    /// The location `pointer` points to and the structure type it
    /// designates, if any. Pointing one past the end of its array, or
    /// further out, designates nothing.
    pub fn dereference(&self, pointer: &Value) -> Result<(usize, i32, Option<usize>), Diagnostic> {
        match *pointer {
            | Value::Pointer {
                target: Some(target),
                offset,
                first,
                length,
                structure,
            } => {
                if offset < first || offset >= first + length {
                    let object = self.live_object(target)?;
                    return Err(self.outside(object, offset, first, length));
                }
                let _index = self.checked_location(target, offset)?;
                Ok((target, offset, structure))
            },
            | Value::Pointer { target: None, .. } => Err(Diagnostic::ub(
                self.state.source_range,
                "indirection through a null pointer",
                &clauses::INDIRECTION,
                Undefined::NullIndirection,
            )),
            | _ => Err(self.error("indirection requires a pointer")),
        }
    }

    /// The undefined behavior of an access at leaf `offset` of `object`,
    /// outside the array of `length` elements starting at leaf `first`.
    fn outside(&self, object: &Object, offset: i32, first: i32, length: i32) -> Diagnostic {
        let name = self.array_name(object.id, first);
        let index = offset - first;
        // Pointer arithmetic stops anything further out first.
        let cause = if index == length {
            Undefined::OnePastEnd
        } else {
            Undefined::PointerOutside
        };
        let array = self.object_type(object.id).is_some_and(Type::is_array) || length > 1;
        let message = if array && index == length {
            format!(
                "{name} has {length} elements, [0] to [{}], and this access is at [{index}], one \
                 past the end: a pointer may point there, but no element is there to read or write",
                length - 1
            )
        } else if array {
            format!(
                "{name} has {length} elements, [0] to [{}], and this access is at [{index}], \
                 outside the array",
                length - 1
            )
        } else if index == 1 {
            // C99 §6.5.6p7: a single object acts as an array of one.
            format!(
                "{name} is a single object, which pointer arithmetic treats as an array of one \
                 element, and this access is at [1], one past the end: a pointer may point there, \
                 but no object is there to read or write"
            )
        } else {
            format!(
                "{name} is a single object, which pointer arithmetic treats as an array of one \
                 element, and this access is at [{index}], outside it"
            )
        };
        Diagnostic::ub(
            self.state.source_range,
            &message,
            &clauses::POINTER_ARITHMETIC,
            cause,
        )
    }

    /// The scalar index of leaf `offset` in a live object.
    fn checked_location(&self, target: usize, offset: i32) -> Result<usize, Diagnostic> {
        let object = self.live_object(target)?;
        let length = layout::count(&object.value);
        let index = usize::try_from(offset)
            .ok()
            .filter(|&offset| offset < length);
        index.ok_or_else(|| {
            let total = i32::try_from(length).unwrap_or(i32::MAX);
            match object.value {
                | Value::Array(_) => self.outside(object, offset, 0, total),
                | _ => self.outside(object, offset, 0, 1),
            }
        })
    }

    fn live_object(&self, target: usize) -> Result<&Object, Diagnostic> {
        let Some(object) = target
            .checked_sub(1)
            .and_then(|index| self.state.objects.get(index))
        else {
            return Err(self.error("pointer target ID does not exist"));
        };
        if object.lifetime != "alive" {
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!("`{}` is accessed after its lifetime ended", object.name),
                &clauses::LIFETIME,
                Undefined::AfterLifetime,
            ));
        }
        Ok(object)
    }

    fn check_pointer_lifetime(&self, value: &Value) -> Result<(), Diagnostic> {
        if let Value::Pointer {
            target: Some(target),
            ..
        } = value
            && self
                .state
                .objects
                .get(target - 1)
                .is_some_and(|object| object.lifetime != "alive")
        {
            return Err(Diagnostic::indeterminate(
                self.state.source_range,
                "the object this pointer points to has reached the end of its lifetime, so the \
                 pointer's value is indeterminate",
                &clauses::LIFETIME,
            ));
        }
        Ok(())
    }

    pub fn truth(&self, value: &Value) -> Result<bool, Diagnostic> {
        self.check_pointer_lifetime(value)?;
        match value {
            | Value::Integer(integer) => Ok(*integer != 0),
            | Value::Pointer { target, .. } => Ok(target.is_some()),
            | _ => Err(self.error("condition requires a determinate scalar")),
        }
    }

    /// `left operator right`, computed in `ty` when both are integers:
    /// the common type the usual arithmetic conversions chose, or for a
    /// shift the promoted left operand's type. `ty` is `None` for pointer
    /// arithmetic and pointer comparisons.
    pub fn binary_value(
        &self,
        operator: &str,
        ty: Option<Integer>,
        left: Value,
        right: Value,
    ) -> Result<Value, Diagnostic> {
        self.check_pointer_lifetime(&left)?;
        self.check_pointer_lifetime(&right)?;
        match (left, right) {
            | (Value::Integer(left), Value::Integer(right)) =>
                self.integer_binary(operator, ty.unwrap_or(Integer::Int), left, right),
            | (pointer @ Value::Pointer { .. }, Value::Integer(delta))
                if matches!(operator, "+" | "-") =>
            {
                let delta = if operator == "-" { -delta } else { delta };
                self.offset(&pointer, delta)
            },
            | (Value::Integer(delta), pointer @ Value::Pointer { .. }) if operator == "+" =>
                self.offset(&pointer, delta),
            | (left @ Value::Pointer { .. }, right @ Value::Pointer { .. })
                if matches!(operator, "==" | "!=") =>
            {
                // C permits adjacent one-past pointers to compare equal. That
                // representation question is outside this identity model.
                if let (
                    Value::Pointer {
                        target: Some(a),
                        offset: x,
                        ..
                    },
                    Value::Pointer {
                        target: Some(b),
                        offset: y,
                        ..
                    },
                ) = (&left, &right)
                    && a != b
                    && (*x != 0 || *y != 0)
                {
                    return Err(self
                        .error("comparison of offsets in distinct objects is outside the subset"));
                }
                Ok(Value::Integer(i128::from(
                    left.same_address(&right) == (operator == "=="),
                )))
            },
            | (pointer @ Value::Pointer { .. }, Value::Integer(0))
            | (Value::Integer(0), pointer @ Value::Pointer { .. })
                if matches!(operator, "==" | "!=") =>
            {
                let null = matches!(pointer, Value::Pointer { target: None, .. });
                Ok(Value::Integer(i128::from(null == (operator == "=="))))
            },
            | _ => Err(self.error("unsupported binary operand types")),
        }
    }

    /// Signed overflow of `left operator right` in `ty` (or of `operator
    /// right` when `left` is `None`), with the mathematical result that did
    /// not fit.
    /// C99: §6.5 paragraph 5, p. 67; PDF p. 79.
    pub fn overflow(
        &self,
        ty: Integer,
        left: Option<i128>,
        operator: &str,
        right: i128,
    ) -> Diagnostic {
        let (a, b) = (left.unwrap_or(0), right);
        let exact = match operator {
            | "+" => a + b,
            | "-" => a - b,
            | "*" => a * b,
            // Only MIN / -1 and MIN % -1 overflow; both are undefined
            // because the quotient does not fit.
            | _ => -a,
        };
        let expression = left.map_or_else(
            || format!("{operator}({right})"),
            |left| format!("{left} {operator} {right}"),
        );
        let name = ty.spelling();
        let message = if operator == "%" {
            format!(
                "{expression} is undefined: the quotient {exact} does not fit in {}, whose range \
                 is {} to {}",
                article(name),
                ty.min(),
                ty.max()
            )
        } else {
            format!(
                "{expression} = {exact}, which does not fit in {} (range {} to {}): signed \
                 overflow",
                article(name),
                ty.min(),
                ty.max()
            )
        };
        let cause = if matches!(operator, "/" | "%") {
            Undefined::QuotientOverflow
        } else {
            Undefined::SignedOverflow
        };
        Diagnostic::ub(
            self.state.source_range,
            &message,
            &clauses::EXCEPTIONAL_CONDITION,
            cause,
        )
    }

    /// `left operator right` in the integer type `ty`. A signed result
    /// that does not fit is undefined; an unsigned one is reduced modulo
    /// 2^N (C99 §6.2.5p9).
    fn integer_binary(
        &self,
        operator: &str,
        ty: Integer,
        left: i128,
        right: i128,
    ) -> Result<Value, Diagnostic> {
        if matches!(operator, "/" | "%") && right == 0 {
            return Err(Diagnostic::ub(
                self.state.source_range,
                "integer division or remainder by zero",
                &clauses::DIVISION,
                Undefined::DivisionByZero,
            ));
        }
        // MIN / -1 and MIN % -1: the quotient does not fit, and C11 makes
        // both undefined (C99 §6.5.5p6 leaves the remainder unsettled).
        if matches!(operator, "/" | "%") && ty.signed() && !ty.fits(left / right) {
            return Err(self.overflow(ty, Some(left), operator, right));
        }
        let truth =
            |holds: bool| -> Result<Value, Diagnostic> { Ok(Value::Integer(i128::from(holds))) };
        let exact = match operator {
            | "==" => return truth(left == right),
            | "!=" => return truth(left != right),
            | "<" => return truth(left < right),
            | "<=" => return truth(left <= right),
            | ">" => return truth(left > right),
            | ">=" => return truth(left >= right),
            | "<<" | ">>" => return self.shift_value(operator, ty, left, right),
            | "+" => left.checked_add(right),
            | "-" => left.checked_sub(right),
            | "*" => left.checked_mul(right),
            | "/" => left.checked_div(right),
            | "%" => left.checked_rem(right),
            | _ => return Err(self.error("unsupported integer operator")),
        };
        if !ty.signed() {
            // Both operands are below 2^64, so only a product of two
            // unsigned long long values can leave i128; reduce it in u128.
            let reduced = exact.map_or_else(
                || {
                    let product = left.unsigned_abs().wrapping_mul(right.unsigned_abs());
                    i128::try_from(product % (1_u128 << ty.bits())).unwrap_or_default()
                },
                |exact| ty.convert(exact),
            );
            return Ok(Value::Integer(reduced));
        }
        exact
            .filter(|&exact| ty.fits(exact))
            .map(Value::Integer)
            .ok_or_else(|| self.overflow(ty, Some(left), operator, right))
    }

    /// `left << right` or `left >> right` in `ty`, the promoted left
    /// operand's type.
    ///
    /// C99: §6.5.7 paragraphs 3-5, pp. 84-85; PDF pp. 96-97.
    fn shift_value(
        &self,
        operator: &str,
        ty: Integer,
        left: i128,
        right: i128,
    ) -> Result<Value, Diagnostic> {
        let bits = ty.bits();
        let Some(count) = u32::try_from(right).ok().filter(|&count| count < bits) else {
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "`{left} {operator} {right}` shifts {}, which has {bits} bits, by {right}: a \
                     shift count must be 0 to {}",
                    article(ty.spelling()),
                    bits - 1
                ),
                &clauses::SHIFT_COUNT,
                Undefined::ShiftCount,
            ));
        };
        if operator == ">>" {
            // A negative signed value shifts arithmetically, copying its
            // sign bit, as bcc and GCC define it (C99 §6.5.7p5).
            return Ok(Value::Integer(left >> count));
        }
        let exact = left << count;
        if !ty.signed() {
            return Ok(Value::Integer(ty.convert(exact)));
        }
        if left < 0 {
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "`{left} << {right}` shifts a negative {} left, which C leaves undefined",
                    ty.spelling()
                ),
                &clauses::LEFT_SHIFT,
                Undefined::ShiftNegative,
            ));
        }
        if !ty.fits(exact) {
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "`{left} << {right}` is {left} × 2^{right} = {exact}, which does not fit in \
                     {} (largest {}): undefined for a signed type",
                    article(ty.spelling()),
                    ty.max()
                ),
                &clauses::LEFT_SHIFT,
                Undefined::ShiftOverflow,
            ));
        }
        Ok(Value::Integer(exact))
    }

    fn offset(&self, pointer: &Value, delta: i128) -> Result<Value, Diagnostic> {
        let Value::Pointer {
            target: Some(target),
            offset,
            first,
            length,
            structure,
        } = *pointer
        else {
            return Err(Diagnostic::ub(
                self.state.source_range,
                "pointer arithmetic requires a pointer to an object",
                &clauses::POINTER_ARITHMETIC,
                Undefined::NullArithmetic,
            ));
        };
        let _object = self.live_object(target)?;
        // C99 §6.5.6p7 treats an object outside any array as an array of
        // one element here.
        let result = i128::from(offset).checked_add(delta).filter(|&value| {
            value >= i128::from(first) && value <= i128::from(first) + i128::from(length)
        });
        let Some(offset) = result else {
            return Err(Diagnostic::ub(
                self.state.source_range,
                "pointer arithmetic leaves the object and its one-past position",
                &clauses::POINTER_ARITHMETIC,
                Undefined::PointerOutside,
            ));
        };
        let offset =
            i32::try_from(offset).map_err(|_| self.error("pointer offset exceeds subset range"))?;
        Ok(Value::Pointer {
            target: Some(target),
            offset,
            first,
            length,
            structure,
        })
    }
}

/// A type with its article: "an int", "a char", "an unsigned int".
pub(super) fn article(spelling: &str) -> String {
    if spelling.starts_with(['a', 'e', 'i', 'o', 'u']) {
        format!("an {spelling}")
    } else {
        format!("a {spelling}")
    }
}
