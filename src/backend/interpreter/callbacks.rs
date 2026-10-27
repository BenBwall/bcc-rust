//! `qsort` and `bsearch`, the C library functions that call back into the
//! program's comparison function.
//!
//! The machine runs them itself rather than the host, since a comparison is
//! an ordinary interpreted call: [`Machine::compare`] pushes the comparison
//! function's frame and steps the machine until that frame returns, so its
//! instructions count against the step limit and its traps stop the program
//! where they happen. `qsort` is glibc's stable merge sort step for step, so
//! the comparison function sees the same calls glibc makes: pointers into
//! the array as it is being merged, or, for elements over 32 bytes, which
//! glibc sorts through an array of pointers, pointers to the elements where
//! they started. `bsearch` is glibc's bisection. glibc's fallbacks when its
//! temporary buffer cannot be allocated are not modelled.
//!
//! C99: §7.20.5, pp. 318-319; PDF pp. 330-331.

use super::{
    Machine,
    host::Host,
    memory::ObjectKind,
    outcome::Termination,
    trap::{
        Fault,
        UbKind,
        Unsupported,
    },
    value::RuntimeValue,
};
use crate::ir::{
    Align,
    Type,
};

/// The functions this module runs.
pub(crate) const FUNCTIONS: [&str; 2] = ["qsort", "bsearch"];

/// The state of a comparison call the machine is running for the library.
#[derive(Clone, Copy, Debug)]
pub(super) enum Callback {
    /// Running; its frame returns to this frame depth.
    Waiting(usize),
    /// Returned this value.
    Returned(Option<RuntimeValue>),
}

/// glibc sorts elements larger than this through pointers to them.
const INDIRECT_SIZE: u128 = 32;

/// What a merge sort orders, with the comparison function.
#[derive(Clone, Copy)]
enum Sorted {
    /// The program's elements themselves.
    Elements(RuntimeValue),
    /// Pointers to them.
    Pointers(RuntimeValue),
}

/// How a comparison call ended.
enum Compared {
    /// The function returned this value.
    Order(i32),
    /// The program ended inside it, as `exit` does.
    Ended(Termination),
}

impl Machine<'_, '_, '_> {
    /// Runs `qsort` or `bsearch` with the call's arguments, returning its
    /// result, or the program's ending if a comparison ended it.
    pub(super) fn library_callback(
        &mut self,
        name: &str,
        host: &mut dyn Host,
    ) -> Result<Result<Option<RuntimeValue>, Termination>, Fault> {
        // Comparisons reuse the argument buffers, so keep the call's own.
        let mut args = [RuntimeValue::Poison; 5];
        for (slot, &value) in args.iter_mut().zip(self.arguments.iter()) {
            *slot = value;
        }
        let types: &[Type] = &self.argument_types;
        let qsort = types == [Type::Ptr, Type::I64, Type::I64, Type::Ptr];
        let bsearch = types == [Type::Ptr, Type::Ptr, Type::I64, Type::I64, Type::Ptr];
        let integer =
            |value: RuntimeValue| value.as_int().ok_or(Fault::Ub(UbKind::PoisonInHostCall));
        if name == "qsort" {
            if !qsort {
                return Err(UbKind::SignatureMismatch.into());
            }
            let (count, size) = (integer(args[1])?, integer(args[2])?);
            let total = count.checked_mul(size).ok_or(UbKind::OutOfBounds)?;
            if count > 1 && size > 0 {
                let sorted = if size > INDIRECT_SIZE {
                    self.sort_indirectly(args[0], count, size, args[3], host)
                } else {
                    let temporary = self.scratch_object(total)?;
                    let sorted = self.merge_sort(
                        args[0],
                        count,
                        size,
                        RuntimeValue::Ptr(temporary),
                        Sorted::Elements(args[3]),
                        host,
                    );
                    self.memory
                        .free(temporary.provenance.expect("a fresh object"));
                    sorted
                };
                if let Err(termination) = sorted? {
                    return Ok(Err(termination));
                }
            }
            return Ok(Ok(None));
        }
        if !bsearch {
            return Err(UbKind::SignatureMismatch.into());
        }
        let (key, base, size) = (args[0], args[1], integer(args[3])?);
        let (mut low, mut high) = (0, integer(args[2])?);
        // An array past the address space would wrap element offsets.
        if high
            .checked_mul(size)
            .is_none_or(|total| total > u128::from(u64::MAX))
        {
            return Err(UbKind::OutOfBounds.into());
        }
        while low < high {
            let middle = u128::midpoint(low, high);
            let element = self
                .memory
                .ptr_add(base, RuntimeValue::Int(middle * size), true);
            match self.compare(args[4], key, element, host)? {
                | Compared::Ended(termination) => return Ok(Err(termination)),
                | Compared::Order(order) if order < 0 => high = middle,
                | Compared::Order(order) if order > 0 => low = middle + 1,
                | Compared::Order(_) => return Ok(Ok(Some(element))),
            }
        }
        Ok(Ok(Some(RuntimeValue::NULL)))
    }

    /// A heap object of `size` bytes for the sort's own use.
    fn scratch_object(&mut self, size: u128) -> Result<super::value::Pointer, Fault> {
        let size = u64::try_from(size).map_err(|_| UbKind::OutOfBounds)?;
        self.memory.allocate(ObjectKind::Heap, size, Align::BYTE)
    }

    /// glibc's indirect sort for elements over 32 bytes: it sorts an array
    /// of pointers to the elements, so comparisons see the elements where
    /// they started, then moves each element to its sorted place.
    fn sort_indirectly(
        &mut self,
        base: RuntimeValue,
        count: u128,
        size: u128,
        function: RuntimeValue,
        host: &mut dyn Host,
    ) -> Result<Result<(), Termination>, Fault> {
        let pointers = self.scratch_object(count * 8)?;
        let temporary = self.scratch_object(count * 8)?;
        let copy = self.scratch_object(count * size)?;
        let at = |this: &Self, pointer: RuntimeValue, offset: u128| {
            this.memory
                .ptr_add(pointer, RuntimeValue::Int(offset), true)
        };
        let (pointers, temporary, copy) = (
            RuntimeValue::Ptr(pointers),
            RuntimeValue::Ptr(temporary),
            RuntimeValue::Ptr(copy),
        );
        let release = |this: &mut Self| {
            for object in [pointers, temporary, copy] {
                let object = object.as_pointer().expect("a fresh object");
                this.memory.free(object.provenance.expect("a fresh object"));
            }
        };
        for index in 0..count {
            let element = at(self, base, index * size);
            self.memory.store(
                Type::Ptr,
                element,
                at(self, pointers, index * 8),
                Align::BYTE,
            )?;
        }
        let sorted = self.merge_sort(
            pointers,
            count,
            8,
            temporary,
            Sorted::Pointers(function),
            host,
        );
        if !matches!(sorted, Ok(Ok(()))) {
            release(self);
            return sorted;
        }
        let total = RuntimeValue::Int(count * size);
        self.memory.copy(copy, base, total, Align::BYTE, false)?;
        let base_address = base.as_pointer().map_or(0, |pointer| pointer.address);
        for index in 0..count {
            let element =
                self.memory
                    .load(Type::Ptr, at(self, pointers, index * 8), Align::BYTE)?;
            // The pointers are the sort's own, each into the array.
            let offset = element
                .as_pointer()
                .and_then(|pointer| pointer.address.checked_sub(base_address))
                .ok_or(UbKind::OutOfBounds)?;
            let source = at(self, copy, u128::from(offset));
            let destination = at(self, base, index * size);
            self.memory.copy(
                destination,
                source,
                RuntimeValue::Int(size),
                Align::BYTE,
                false,
            )?;
        }
        release(self);
        Ok(Ok(()))
    }

    /// glibc's `msort_with_tmp`: sorts the halves of `count` elements of
    /// `size` bytes at `base`, merges them into `temporary` and copies the
    /// merged prefix back. The elements are the program's own, or pointers
    /// to them for the indirect sort.
    fn merge_sort(
        &mut self,
        base: RuntimeValue,
        count: u128,
        size: u128,
        temporary: RuntimeValue,
        sorted: Sorted,
        host: &mut dyn Host,
    ) -> Result<Result<(), Termination>, Fault> {
        if count <= 1 {
            return Ok(Ok(()));
        }
        let at = |this: &Self, pointer: RuntimeValue, index: u128| {
            this.memory
                .ptr_add(pointer, RuntimeValue::Int(index * size), true)
        };
        let (mut first, mut second) = (count / 2, count - count / 2);
        let mut left = base;
        let mut right = at(self, base, first);
        if let Err(termination) = self.merge_sort(left, first, size, temporary, sorted, host)? {
            return Ok(Err(termination));
        }
        if let Err(termination) = self.merge_sort(right, second, size, temporary, sorted, host)? {
            return Ok(Err(termination));
        }
        let element = RuntimeValue::Int(size);
        let mut output = temporary;
        while first > 0 && second > 0 {
            let compared = match sorted {
                | Sorted::Elements(function) => self.compare(function, left, right, host)?,
                | Sorted::Pointers(function) => {
                    let a = self.memory.load(Type::Ptr, left, Align::BYTE)?;
                    let b = self.memory.load(Type::Ptr, right, Align::BYTE)?;
                    self.compare(function, a, b, host)?
                },
            };
            let take_left = match compared {
                | Compared::Ended(termination) => return Ok(Err(termination)),
                | Compared::Order(order) => order <= 0,
            };
            let source = if take_left { &mut left } else { &mut right };
            self.memory
                .copy(output, *source, element, Align::BYTE, false)?;
            *source = at(self, *source, 1);
            if take_left {
                first -= 1;
            } else {
                second -= 1;
            }
            output = at(self, output, 1);
        }
        if first > 0 {
            self.memory.copy(
                output,
                left,
                RuntimeValue::Int(first * size),
                Align::BYTE,
                false,
            )?;
        }
        self.memory.copy(
            base,
            temporary,
            RuntimeValue::Int((count - second) * size),
            Align::BYTE,
            false,
        )?;
        Ok(Ok(()))
    }

    /// Calls the comparison function `function` with `a` and `b` and runs it
    /// to its return.
    fn compare(
        &mut self,
        function: RuntimeValue,
        a: RuntimeValue,
        b: RuntimeValue,
        host: &mut dyn Host,
    ) -> Result<Compared, Fault> {
        let func = self.memory.function_at(function)?;
        let func = self.module.function_definition(func);
        let signature = self.module.function_signature(func);
        if signature.params != [Type::Ptr, Type::Ptr]
            || signature.result != Some(Type::I32)
            || self.module.function(func).body.is_none()
        {
            return Err(UbKind::InvalidComparison.into());
        }
        self.arguments.clear();
        self.argument_types.clear();
        self.argument_abis.clear();
        for value in [a, b] {
            self.arguments.push(value);
            self.argument_types.push(Type::Ptr);
            self.argument_abis.push(None);
        }
        let floor = self.frames.len();
        let outer = self.callback.replace(Callback::Waiting(floor));
        let entered = self
            .check_arguments(signature)
            .and_then(|()| self.push_frame(func, None));
        let result = entered.and_then(|()| {
            loop {
                if let Some(Callback::Returned(value)) = self.callback {
                    break Ok(Ok(value));
                }
                if self.frames.len() <= floor {
                    // A `longjmp` left the comparison past the library call.
                    break Err(Fault::Unsupported(Unsupported::CallbackEscape));
                }
                match self.step(host) {
                    | Ok(None) => {},
                    | Ok(Some(termination)) => break Ok(Err(termination)),
                    | Err(trap) => break Err(Fault::Nested(trap)),
                }
            }
        });
        self.callback = outer;
        match result? {
            | Err(termination) => Ok(Compared::Ended(termination)),
            | Ok(Some(RuntimeValue::Int(bits))) => Ok(Compared::Order(i32::from_ne_bytes(
                u32::try_from(bits & 0xFFFF_FFFF)
                    .expect("masked")
                    .to_ne_bytes(),
            ))),
            | _ => Err(UbKind::PoisonInHostCall.into()),
        }
    }
}
