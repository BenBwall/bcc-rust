//! Single-session, UTF-8 JSON bridge to the browser execution engine.
//!
//! Read `bcc_compile`, then `respond`. The caller owns input allocations;
//! response storage stays here until the next operation. No JavaScript glue
//! generator, filesystem, native compiler, or imported host function is needed.

use std::{
    alloc::{
        self,
        Layout,
    },
    cell::RefCell,
};

use crate::abstract_machine::Session;

/// Compile caller-owned UTF-8 source and retain a fresh session.
///
/// # Safety
/// A nonempty input must be initialized bytes in a live `bcc_alloc` allocation.
/// Calls into this instance must be single-threaded and non-reentrant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcc_compile(ptr: *const u8, len: usize) -> u32 {
    let memory_bytes = core::arch::wasm32::memory_size::<0>() * 65_536;
    if len > INPUT_LIMIT
        || (len != 0 && ptr.is_null())
        || ptr
            .addr()
            .checked_add(len)
            .is_none_or(|end| end > memory_bytes)
    {
        return fail("input exceeds limit or has a null pointer");
    }
    let bytes = if len == 0 {
        &[]
    } else {
        // SAFETY: the caller supplies initialized bytes in a live allocation;
        // the null case was rejected above.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    };
    let Ok(source) = std::str::from_utf8(bytes) else {
        return fail("source must be UTF-8");
    };
    BRIDGE.with_borrow_mut(|bridge| {
        let session = Session::compile(source);
        bridge.response = session.state_json();
        bridge.session = Some(session);
    });
    0
}

/// Return the current state without advancing the session.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_state() -> u32 {
    respond(|session| session.state_json())
}

/// Advance one execution step and return the state.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_step() -> u32 {
    respond(|session| {
        session.step();
        session.state_json()
    })
}

/// Restore the previous execution state.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_back() -> u32 {
    respond(|session| {
        session.back();
        session.state_json()
    })
}

/// Restore the compiled session's initial state.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_reset() -> u32 {
    respond(|session| {
        session.reset();
        session.state_json()
    })
}

/// Return the deterministic execution trace as JSON.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_trace() -> u32 {
    respond(|session| session.trace_json())
}

/// Return the version of this memory and operation protocol.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_abi_version() -> u32 {
    1
}

/// Allocate zeroed caller-owned bytes, returning null for zero or excessive
/// size.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_alloc(len: usize) -> *mut u8 {
    if len == 0 || len > INPUT_LIMIT {
        return std::ptr::null_mut();
    }
    let Ok(layout) = Layout::array::<u8>(len) else {
        return std::ptr::null_mut();
    };
    // SAFETY: a nonzero layout with byte alignment was checked above.
    unsafe { alloc::alloc_zeroed(layout) }
}

/// Release a caller-owned input allocation; null is a no-op.
///
/// # Safety
/// `ptr` must be null or a live `bcc_alloc(len)` result. Free it exactly once,
/// using the original length, and never free a response pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcc_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let layout = Layout::array::<u8>(len).expect("original allocation layout");
    // SAFETY: the caller promises the original pointer, layout and ownership.
    unsafe { alloc::dealloc(ptr, layout) };
}

/// Borrow the latest response bytes until the next operation.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_response_ptr() -> *const u8 {
    BRIDGE.with_borrow(|bridge| bridge.response.as_ptr())
}

/// Return the byte length of the latest response.
#[unsafe(no_mangle)]
pub extern "C" fn bcc_response_len() -> usize {
    BRIDGE.with_borrow(|bridge| bridge.response.len())
}

#[expect(
    clippy::disallowed_types,
    reason = "The ABI owns JSON response bytes independently of compilation arenas."
)]
fn respond(action: impl FnOnce(&mut Session) -> String) -> u32 {
    BRIDGE.with_borrow_mut(|bridge| {
        let Some(session) = bridge.session.as_mut() else {
            bridge.response = String::from(NO_SESSION);
            return 1;
        };
        bridge.response = action(session);
        0
    })
}

fn fail(message: &str) -> u32 {
    BRIDGE.with_borrow_mut(|bridge| {
        bridge.response.clear();
        // Messages are fixed ASCII literals with no JSON escapes.
        bridge.response.push_str("{\"version\":1,\"error\":\"");
        bridge.response.push_str(message);
        bridge.response.push_str("\"}");
    });
    1
}

#[expect(
    clippy::disallowed_types,
    reason = "The bridge retains the engine's owned JSON between browser calls."
)]
struct Bridge {
    session:  Option<Session>,
    response: String,
}

thread_local! {
    #[expect(
        clippy::disallowed_types,
        reason = "The browser ABI owns its persistent JSON response buffer."
    )]
    static BRIDGE: RefCell<Bridge> = const { RefCell::new(Bridge {
        session: None,
        response: String::new(),
    }) };
}

const INPUT_LIMIT: usize = 1024 * 1024;
const NO_SESSION: &str = "{\"version\":1,\"error\":\"compile a session first\"}";
