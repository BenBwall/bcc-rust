/// Compile checks import the crate-private allocator from its source file.
///
/// ```compile_fail,E0080
/// # #[path = "util/bump.rs"]
/// # mod bump;
/// let arena = bump::Bump::new();
/// arena.alloc(String::from("owned"));
/// ```
pub(crate) mod arena_list;

pub(crate) mod bump;

pub(crate) mod vm;
