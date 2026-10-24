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
    machine::{
        Machine,
        ObjectMetadata,
        text,
    },
    model::{
        Diagnostic,
        MAX_CELLS,
        MAX_OBJECTS,
        Object,
        Type,
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
    ) -> Result<(), Diagnostic> {
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
            | Type::Pointer => String::from("int *"),
            | Type::Array(count) => text(format_args!("int[{count}]")),
        };
        let top = self.activations.len() - 1;
        self.state.objects.push(Object {
            id: self.state.objects.len() + 1,
            name: String::from(name),
            r#type,
            frame_id: self.activations[top].id,
            storage_duration: "automatic",
            lifetime: "alive",
            value,
        });
        self.metadata.push(ObjectMetadata { scope, ty });
        self.cells += cells;
        Ok(())
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
            // representations; this prototype stops conservatively,
            // rather than asserting every possible indeterminate
            // read must be UB.
            return Err(Diagnostic {
                citation: Some("C99 §6.7.8p10, p. 126; PDF p. 138"),
                ..Diagnostic::new(
                    "indeterminate",
                    self.state.source_range,
                    "read of an indeterminate value; prototype policy stops execution without \
                     claiming all such reads are UB",
                )
            });
        }
        self.check_pointer_lifetime(&value)?;
        Ok(value)
    }

    pub fn store(
        &mut self,
        target: usize,
        offset: i32,
        mut value: Value,
    ) -> Result<(), Diagnostic> {
        let index = self.checked_location(target, offset)?;
        let object = &self.state.objects[target - 1];
        if self.metadata[object.id - 1].ty == Type::Pointer && value == Value::Integer(0) {
            value = Value::Pointer {
                target: None,
                offset: 0,
            };
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
            | Value::Array(elements) => elements[index] = value,
            | slot => *slot = value,
        }
        Ok(())
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
                "C99 §6.5.3.2p4, p. 79; PDF p. 91",
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
            Diagnostic::ub(
                self.state.source_range,
                "access outside an object or through a one-past pointer",
                "C99 §6.5.6p8, p. 83; PDF p. 95",
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
                "object accessed outside its lifetime",
                "C99 §6.2.4p2, p. 32; PDF p. 44",
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
            return Err(Diagnostic {
                citation: Some("C99 §6.2.4p2, p. 32; PDF p. 44"),
                ..Diagnostic::new(
                    "indeterminate",
                    self.state.source_range,
                    "pointer became indeterminate when its target lifetime ended; prototype \
                     policy stops on use",
                )
            });
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

    /// C99: §6.5 paragraph 5, p. 67; PDF p. 79.
    pub fn overflow(&self) -> Diagnostic {
        Diagnostic::ub(
            self.state.source_range,
            "signed int result is outside the representable range",
            "C99 §6.5p5, p. 67; PDF p. 79",
        )
    }

    fn integer_binary(&self, operator: &str, left: i32, right: i32) -> Result<Value, Diagnostic> {
        if matches!(operator, "/" | "%") && right == 0 {
            return Err(Diagnostic::ub(
                self.state.source_range,
                "integer division or remainder by zero",
                "C99 §6.5.5p5, p. 82; PDF p. 94",
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
        result.map(Value::Integer).ok_or_else(|| self.overflow())
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
                "C99 §6.5.6p8, p. 83; PDF p. 95",
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
                "C99 §6.5.6p8, p. 83; PDF p. 95",
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
