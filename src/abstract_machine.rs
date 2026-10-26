//! Bounded, source-driven C execution for the browser prototype.
//!
//! [`compile`] parses an explicit subset into source-located operations;
//! [`Session::step`] executes one operation and retains the entire machine.
//! For `int x = 1;`, declaration first exposes indeterminate storage, then
//! expression operations compute `1`, and a store initializes the object.
//! [`Session::back`] restores history; stepping forward replays it exactly.
//!
//! Read `parser.rs` and `expressions.rs` for compilation, `machine.rs` and
//! `memory.rs` for execution, `model.rs` for the public observation types, and
//! `json.rs` for the versioned wire format. `lexer.rs` retains byte ranges.
//! `tests.rs` exercises actual execution, rejection, UB and replay.
//!
//! This independent subset does not invoke bcc's syntax/semantic frontend.
//! It uses a signed 32-bit int model and element-based pointer offsets.
//! Operand evaluation is left-to-right; nested side effects are rejected.
//! It is an educational execution model, not a C conformance checker.
//!
//! C99: §6.7.8 paragraph 10, p. 126; PDF p. 138 (indeterminate locals).
//! C99: §6.2.4 paragraph 2, p. 32; PDF p. 44 (object lifetime).
//! C99: §6.5 paragraph 5, p. 67; PDF p. 79 (arithmetic exceptions).
//! C99: §6.5.6 paragraph 8, p. 83; PDF p. 95 (pointer bounds).

// Standalone session storage, independent of the compiler's native arenas.
#![expect(
    clippy::disallowed_types,
    reason = "The bounded browser session owns reclaimable std buffers; compiler VM arenas are \
              unavailable on wasm32. This exception applies only to the independent evaluator."
)]

// Compilation
mod expressions;
mod lexer;
mod parser;

// Execution and observations
mod json;
mod machine;
mod memory;
mod model;

use machine::Machine;
pub use model::{
    Diagnostic,
    Frame,
    Object,
    Program,
    SourceRange,
    State,
    Value,
};

/// Compiles the bounded C subset. Unsupported input is an explicit error.
///
/// # Errors
/// Returns the first syntax, unsupported-construct or resource diagnostic.
pub fn compile(source: &str) -> Result<Program, Diagnostic> {
    parser::compile(source)
}

/// A reversible execution session. History and all runtime resources are
/// bounded.
#[derive(Debug)]
pub struct Session {
    program:        Program,
    history:        Vec<Machine>,
    cursor:         usize,
    retained_bytes: usize,
}

impl Session {
    /// Executes one operation, or replays the next recorded state.
    /// Terminal sessions remain unchanged. Limits stop with a diagnostic.
    pub fn step(&mut self) -> &State {
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
        } else if !self.state().terminal() {
            let mut next = self.history[self.cursor].clone();
            if next.state.step >= model::MAX_STEPS {
                next.stop(Diagnostic::limit(
                    next.state.source_range,
                    "execution step limit reached",
                ));
            } else {
                next.execute(&self.program);
            }
            next.state.step += 1;
            let cost = next.retention_cost();
            if self.retained_bytes.saturating_add(cost) > model::MAX_TRACE_BYTES {
                // Preserve the prefix, recording a small terminal copy. The
                // budget reserves room for this final diagnostic state.
                next.stop(Diagnostic::limit(
                    next.state.source_range,
                    "trace retention limit reached",
                ));
            }
            self.retained_bytes = self.retained_bytes.saturating_add(cost);
            self.history.push(next);
            self.cursor += 1;
        }
        self.state()
    }

    /// Restores the preceding complete state; the initial state is a fixed
    /// point.
    pub fn back(&mut self) -> &State {
        self.cursor = self.cursor.saturating_sub(1);
        self.state()
    }

    /// Returns the current observation without advancing execution.
    #[must_use]
    pub fn state(&self) -> &State {
        &self.history[self.cursor].state
    }

    /// Encodes the current observation using schema version 1.
    #[must_use]
    pub fn state_json(&self) -> String {
        self.state().to_json()
    }

    /// Encodes all recorded observations, including states ahead of the cursor.
    #[must_use]
    pub fn trace_json(&self) -> String {
        json::trace(&self.program, &self.history)
    }
}

impl Program {
    /// Starts a fresh session at `main`, without executing its first operation.
    #[must_use]
    pub fn session(&self) -> Session {
        let initial = Machine::new(self);
        let retained_bytes = initial.retention_cost();
        let history = Vec::from([initial]);
        Session {
            program: self.clone(),
            history,
            cursor: 0,
            retained_bytes,
        }
    }

    /// The exact source from which this program was compiled.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Editable demonstration: a call changes an array element through a pointer.
pub const DEMO_SOURCE: &str = "int bump(int *p) {\n    int before = *p;\n    *p = before + 1;\n    return *p;\n}\n\nint main(void) {\n    int values[3] = {1, 2, 3};\n    int total;\n    total = 0;\n    int *cursor = &values[0];\n    for (int i = 0; i < 3; i++) {\n        int next = bump(cursor);\n        total = total + next;\n        cursor = cursor + 1;\n    }\n    return total;\n}\n";

#[cfg(test)]
mod tests;
