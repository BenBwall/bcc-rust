//! Arena storage uses operating-system virtual memory to keep allocations
//! at stable addresses. Bump allocation fills a committed prefix, while arena
//! lists store immutable slices behind one pointer.
//!
//! Read [`bump::Bump::alloc`], [`vm::GrowingRegion::ensure_committed`],
//! and [`arena_list::ArenaList::copy_from_slice`].
//!
//! Files by role:
//! - Allocation: `memory/bump.rs`.
//! - Virtual memory: `memory/vm.rs`.
//! - Immutable lists: `memory/arena_list.rs`.

// Allocation
pub(crate) mod bump;

// Virtual memory
pub(crate) mod vm;

// Immutable storage
/// Compile checks import the crate-private allocator from its source file.
///
/// ```compile_fail,E0080
/// # #[path = "util/memory/bump.rs"]
/// # mod bump;
/// let arena = bump::Bump::new();
/// arena.alloc(String::from("owned"));
/// ```
pub(crate) mod arena_list;
