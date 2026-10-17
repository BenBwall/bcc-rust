//! Undefined weak TLS resolves to a shared thread base, not absolute zero.

use super::*;

#[test]
fn unresolved_weak_tls_has_one_nonnull_base_without_defining_storage() {
    let text = "\
global @first weak thread_local size 4, align 4
global @second weak thread_local size 16, align 16
global @ordinary weak size 4, align 4
global @defined weak thread_local size 4, align 4 = bytes \"07000000\"
function @main() -> i32 external {
block0:
    v0 = global_addr @first
    v1 = global_addr @second
    v2 = global_addr @ordinary
    v3 = global_addr @defined
    v4 = iconst.i64 0
    v5 = inttoptr.ptr v4
    v6 = icmp.ptr ne v0, v5
    v7 = icmp.ptr eq v0, v1
    v8 = icmp.ptr eq v2, v5
    v9 = icmp.ptr ne v0, v3
    v10 = and.i1 v6, v7
    v11 = and.i1 v8, v9
    v12 = and.i1 v10, v11
    v13 = zext.i32 v12
    return v13
}
";
    assert_eq!(returned(text, "main"), int(1));

    let read = "\
global @missing weak thread_local size 4, align 4
function @main() -> i32 external {
block0:
    v0 = global_addr @missing
    v1 = load.i32 v0, align 4
    return v1
}
";
    let run = run_text(read, "main");
    assert!(
        matches!(run.result, Err(Trap::UnknownGlobal(_, _))),
        "{run:?}"
    );
}
