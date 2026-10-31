//! Object identity, lifetime and scalar arithmetic checks, without host
//! pointers.
//!
//! C99: §6.2.4 paragraph 2, p. 32; PDF p. 44 (expired objects).
//! C99: §6.7.8 paragraph 10, p. 126; PDF p. 138 (indeterminate locals).
//! C99: §6.5 paragraph 5, p. 67; PDF p. 79 (signed overflow).
//! C99: §6.5.5 paragraph 5, p. 82; PDF p. 94 (division by zero).
//! C99: §6.5.6 paragraph 8, p. 83; PDF p. 95 (pointer arithmetic).
//! C99: §6.5.6 paragraph 7, p. 83; PDF p. 95 (scalar pointer arithmetic).
//! C99: §6.7.8 paragraph 21, p. 127; PDF p. 139 (partial initialization).
//! C99: §6.5.3.2 paragraph 4, p. 79; PDF p. 91 (invalid indirection).

use super::{
    clauses,
    machine::{
        Machine,
        ObjectMetadata,
    },
    model::{
        Diagnostic,
        MAX_CELLS,
        MAX_OBJECTS,
        Object,
        Type,
        Undefined,
        Value,
    },
};

impl Machine {
    pub fn allocate(
        &mut self,
        name: &str,
        ty: Type,
        scope: usize,
        value: Value,
        parameter: bool,
    ) -> Result<usize, Diagnostic> {
        let cells = match ty {
            | Type::Array(count) => count,
            | _ => 1,
        };
        if self.state.objects.len() >= MAX_OBJECTS || self.cells + cells > MAX_CELLS {
            return Err(Diagnostic::limit(
                self.state.source_range,
                "object or memory cell limit reached",
            ));
        }
        let value = if let Type::Array(count) = ty {
            let mut elements = Vec::new();
            elements.resize(count, Value::Indeterminate);
            Value::Array(elements)
        } else if ty == Type::Pointer && value == Value::Integer(0) {
            Value::Pointer {
                target: None,
                offset: 0,
            }
        } else {
            value
        };
        let r#type = match ty {
            | Type::Int => String::from("int"),
            | Type::Char => String::from("char"),
            | Type::Pointer => String::from("int *"),
            | Type::Array(count) => format!("int[{count}]"),
        };
        let top = self.activations.len() - 1;
        let id = self.state.objects.len() + 1;
        self.state.objects.push(Object {
            id,
            name: String::from(name),
            r#type,
            frame_id: self.activations[top].id,
            storage_duration: "automatic",
            lifetime: "alive",
            parameter,
            value,
        });
        self.metadata.push(ObjectMetadata { scope, ty });
        self.cells += cells;
        Ok(id)
    }

    /// The value stored at a location, without lifetime or bounds checks.
    pub fn peek(&self, target: usize, offset: i32) -> Option<Value> {
        let object = self.state.objects.get(target.checked_sub(1)?)?;
        match &object.value {
            | Value::Array(elements) => elements.get(usize::try_from(offset).ok()?).cloned(),
            | value => Some(value.clone()),
        }
    }

    /// The declared type of an object.
    pub fn object_type(&self, target: usize) -> Option<Type> {
        self.metadata
            .get(target.checked_sub(1)?)
            .map(|metadata| metadata.ty)
    }

    pub fn named(&self, name: &str) -> Result<usize, Diagnostic> {
        let top = self.activations.len() - 1;
        self.state
            .objects
            .iter()
            .rev()
            .find(|object| {
                object.name == name
                    && object.frame_id == self.activations[top].id
                    && object.lifetime == "alive"
            })
            .map(|object| object.id)
            .ok_or_else(|| self.error("identifier has no live object in this activation"))
    }

    pub fn read(&self, target: usize, offset: i32) -> Result<Value, Diagnostic> {
        let index = self.checked_location(target, offset)?;
        let object = &self.state.objects[target - 1];
        let value = match &object.value {
            | Value::Array(elements) => elements[index].clone(),
            | value => value.clone(),
        };
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

    /// Stores `value`, converted to the object's type, and returns the stored
    /// value. A `char` keeps the low 8 bits of an int as two's complement, the
    /// implementation-defined result x86-64 compilers give.
    /// C99: §6.3.1.3 paragraph 3, p. 43; PDF p. 55.
    pub fn store(
        &mut self,
        target: usize,
        offset: i32,
        mut value: Value,
    ) -> Result<Value, Diagnostic> {
        let index = self.checked_location(target, offset)?;
        let object = &self.state.objects[target - 1];
        if self.metadata[object.id - 1].ty == Type::Pointer && value == Value::Integer(0) {
            value = Value::Pointer {
                target: None,
                offset: 0,
            };
        }
        if self.metadata[object.id - 1].ty == Type::Char
            && let Value::Integer(integer) = value
        {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "Keeping the low 8 bits is the modeled conversion."
            )]
            let low = integer as i8;
            value = Value::Integer(i32::from(low));
        }
        let valid = match self.metadata[object.id - 1].ty {
            | Type::Pointer => matches!(value, Value::Pointer { .. }),
            | _ => matches!(value, Value::Integer(_)),
        };
        if !valid {
            return Err(self.error("store type mismatch"));
        }
        self.check_pointer_lifetime(&value)?;
        match &mut self.state.objects[target - 1].value {
            | Value::Array(elements) => elements[index] = value.clone(),
            | slot => *slot = value.clone(),
        }
        Ok(value)
    }

    pub fn dereference(&self, pointer: &Value) -> Result<(usize, i32), Diagnostic> {
        match *pointer {
            | Value::Pointer {
                target: Some(target),
                offset,
            } => {
                let _index = self.checked_location(target, offset)?;
                Ok((target, offset))
            },
            | Value::Pointer { target: None, .. } => Err(Diagnostic::ub(
                self.state.source_range,
                "indirection through a null pointer",
                &clauses::INDIRECTION,
                Undefined::NullIndirection,
            )),
            | _ => Err(self.error("indirection requires an int pointer")),
        }
    }

    fn checked_location(&self, target: usize, offset: i32) -> Result<usize, Diagnostic> {
        let object = self.live_object(target)?;
        let length = match self.metadata[object.id - 1].ty {
            | Type::Array(count) => count,
            | _ => 1,
        };
        let index = usize::try_from(offset)
            .ok()
            .filter(|&offset| offset < length);
        index.ok_or_else(|| {
            let name = &object.name;
            let cause = if usize::try_from(offset) == Ok(length) {
                Undefined::OnePastEnd
            } else {
                Undefined::OutsideObject
            };
            let message = match self.metadata[object.id - 1].ty {
                | Type::Array(_) if usize::try_from(offset) == Ok(length) => format!(
                    "`{name}` has {length} elements, [0] to [{}], and this access is at \
                     [{offset}], one past the end: a pointer may point there, but no element is \
                     there to read or write",
                    length - 1
                ),
                | Type::Array(_) => format!(
                    "`{name}` has {length} elements, [0] to [{}], and this access is at \
                     [{offset}], outside the array",
                    length - 1
                ),
                // C99 §6.5.6p7: a single object acts as an array of one.
                | _ if offset == 1 => format!(
                    "`{name}` is a single object, which pointer arithmetic treats as an array of \
                     one element, and this access is at [1], one past the end: a pointer may \
                     point there, but no object is there to read or write"
                ),
                | _ => format!(
                    "`{name}` is a single object, which pointer arithmetic treats as an array of \
                     one element, and this access is at [{offset}], outside it"
                ),
            };
            Diagnostic::ub(
                self.state.source_range,
                &message,
                &clauses::POINTER_ARITHMETIC,
                cause,
            )
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

    pub fn binary_value(
        &self,
        operator: &str,
        left: Value,
        right: Value,
    ) -> Result<Value, Diagnostic> {
        self.check_pointer_lifetime(&left)?;
        self.check_pointer_lifetime(&right)?;
        match (left, right) {
            | (Value::Integer(left), Value::Integer(right)) =>
                self.integer_binary(operator, left, right),
            | (pointer @ Value::Pointer { .. }, Value::Integer(delta))
                if matches!(operator, "+" | "-") =>
            {
                let delta = if operator == "-" {
                    -i64::from(delta)
                } else {
                    i64::from(delta)
                };
                self.offset(&pointer, delta)
            },
            | (Value::Integer(delta), pointer @ Value::Pointer { .. }) if operator == "+" =>
                self.offset(&pointer, i64::from(delta)),
            | (left @ Value::Pointer { .. }, right @ Value::Pointer { .. })
                if matches!(operator, "==" | "!=") =>
            {
                // C permits adjacent one-past pointers to compare equal. That
                // representation question is outside this identity model.
                if let (
                    Value::Pointer {
                        target: Some(a),
                        offset: x,
                    },
                    Value::Pointer {
                        target: Some(b),
                        offset: y,
                    },
                ) = (&left, &right)
                    && a != b
                    && (*x != 0 || *y != 0)
                {
                    return Err(self
                        .error("comparison of offsets in distinct objects is outside the subset"));
                }
                Ok(Value::Integer(i32::from(
                    (left == right) == (operator == "=="),
                )))
            },
            | (pointer @ Value::Pointer { .. }, Value::Integer(0))
            | (Value::Integer(0), pointer @ Value::Pointer { .. })
                if matches!(operator, "==" | "!=") =>
            {
                let null = matches!(pointer, Value::Pointer { target: None, .. });
                Ok(Value::Integer(i32::from(null == (operator == "=="))))
            },
            | _ => Err(self.error("unsupported binary operand types")),
        }
    }

    /// Signed overflow of `left operator right` (or of `operator right` when
    /// `left` is `None`), with the mathematical result that did not fit.
    /// C99: §6.5 paragraph 5, p. 67; PDF p. 79.
    pub fn overflow(&self, left: Option<i32>, operator: &str, right: i32) -> Diagnostic {
        let (a, b) = (i64::from(left.unwrap_or(0)), i64::from(right));
        let exact = match operator {
            | "+" => a + b,
            | "-" => a - b,
            | "*" => a * b,
            // Only i32::MIN / -1 and i32::MIN % -1 overflow; both are
            // undefined because the quotient does not fit.
            | _ => -a,
        };
        let expression = left.map_or_else(
            || format!("{operator}({right})"),
            |left| format!("{left} {operator} {right}"),
        );
        let message = if operator == "%" {
            format!(
                "{expression} is undefined: the quotient {exact} does not fit in an int, whose \
                 range is {} to {}",
                i32::MIN,
                i32::MAX
            )
        } else {
            format!(
                "{expression} = {exact}, which does not fit in an int (range {} to {}): signed \
                 overflow",
                i32::MIN,
                i32::MAX
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

    fn integer_binary(&self, operator: &str, left: i32, right: i32) -> Result<Value, Diagnostic> {
        if matches!(operator, "/" | "%") && right == 0 {
            return Err(Diagnostic::ub(
                self.state.source_range,
                "integer division or remainder by zero",
                &clauses::DIVISION,
                Undefined::DivisionByZero,
            ));
        }
        let result = match operator {
            | "+" => left.checked_add(right),
            | "-" => left.checked_sub(right),
            | "*" => left.checked_mul(right),
            | "/" => left.checked_div(right),
            | "%" => left.checked_rem(right),
            | "==" => Some(i32::from(left == right)),
            | "!=" => Some(i32::from(left != right)),
            | "<" => Some(i32::from(left < right)),
            | "<=" => Some(i32::from(left <= right)),
            | ">" => Some(i32::from(left > right)),
            | ">=" => Some(i32::from(left >= right)),
            | _ => return Err(self.error("unsupported integer operator")),
        };
        result
            .map(Value::Integer)
            .ok_or_else(|| self.overflow(Some(left), operator, right))
    }

    fn offset(&self, pointer: &Value, delta: i64) -> Result<Value, Diagnostic> {
        let Value::Pointer {
            target: Some(target),
            offset,
        } = *pointer
        else {
            return Err(Diagnostic::ub(
                self.state.source_range,
                "pointer arithmetic requires a pointer to an object",
                &clauses::POINTER_ARITHMETIC,
                Undefined::NullArithmetic,
            ));
        };
        let object = self.live_object(target)?;
        // C99 §6.5.6p7 treats a scalar as a one-element array here.
        let length = match self.metadata[object.id - 1].ty {
            | Type::Array(count) => count,
            | _ => 1,
        };
        let result = i64::from(offset).checked_add(delta).filter(|&value| {
            value >= 0 && usize::try_from(value).is_ok_and(|index| index <= length)
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
        })
    }
}
