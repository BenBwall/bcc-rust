//! Valid C that lowering once reported as a missing semantic fact: objects
//! of incomplete type, void expressions designated by `*`, GNU function
//! pointer arithmetic and a `main` whose result is not `int`.

use super::*;

#[test]
fn incomplete_external_objects_are_declared_for_their_address() {
    // C99 §6.2.5p22, p. 36; PDF p. 48 and §6.5.3.2p1, p. 78; PDF p. 90: an
    // lvalue of incomplete structure type still has an address.
    for (source, declaration) in [
        (
            "struct s; extern struct s object; struct s *f(void) { return &object; }",
            "global @object external size 0, align 1\n",
        ),
        (
            "extern union u object; void *f(void) { return &object; }",
            "global @object external size 0, align 1\n",
        ),
        (
            "extern void object; void *f(void) { return &object; }",
            "global @object external size 0, align 1\n",
        ),
    ] {
        let ir = lowered(source);
        assert!(ir.contains(declaration), "{source}\n{ir}");
        assert!(ir.contains("global_addr @object"), "{source}\n{ir}");
    }
    // A later completion still sizes the object.
    let ir = lowered(
        "struct s; extern struct s object; struct s *f(void) { return &object; } struct s { long \
         a, b; };",
    );
    assert!(
        ir.contains("global @object external size 16, align 8\n"),
        "{ir}"
    );
}

#[test]
fn dereferenced_void_pointers_are_void_expressions() {
    // C99 §6.3.2.2p1, p. 47; PDF p. 59 (DR 106): nothing is accessed.
    let ir = functions(&lowered(
        "void f(void *p, volatile void *q, int i) { *p; i ? *p : *p; *p, *q; (void)*q; }",
    ));
    assert!(!ir.contains("load"), "{ir}");
}

#[test]
fn function_pointer_arithmetic_steps_by_bytes() {
    // GNU extension: GCC "Pointer Arith" counts a function as one byte.
    let ir = functions(&lowered(
        "void (*f)(void); void (*g(int n))(void) { f + 1; f += 2; return f - n; }",
    ));
    assert!(ir.contains("ptr_add"), "{ir}");
}

#[test]
fn main_with_a_non_int_result_has_no_implicit_zero() {
    // C99 §5.1.2.2.3p1, p. 13; PDF p. 25: only an int-compatible main
    // returns 0 at its closing brace; the status is otherwise unspecified.
    let ir = functions(&lowered("const char *main(void) {}"));
    assert!(ir.contains("poison.ptr"), "{ir}");
    let ir = functions(&lowered("int main(void) {}"));
    assert!(ir.contains("iconst.i32 0"), "{ir}");
}

#[test]
fn empty_lists_complete_arrays_of_unknown_size_with_no_elements() {
    // GNU extension: GCC manual, "Arrays of Length Zero"; GCC and Clang give
    // `int[]` initialized by `{}` zero elements rather than leaving the type
    // incomplete.
    let ir = lowered(
        "int g[] = {}; int f(void) { int a[] = {}; return sizeof g + sizeof a + sizeof (int[]){}; \
         } int *h(void) { return (int[]){}; }",
    );
    assert!(
        ir.contains("global @g external size 0, align 4 = zero"),
        "{ir}"
    );
    assert!(!ir.contains("iconst.i64 4"), "{ir}");
}
