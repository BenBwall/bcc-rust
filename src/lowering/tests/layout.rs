//! Layout queries and bit-field execution at the phase-7/lowering seam.
//! Expected layouts follow bundled Clang on all four target triples.

use super::{
    numeric_programs,
    programs::Program,
    *,
};
use crate::backend::llvm::Toolchain;

#[test]
fn round4_zero_width_explicit_alignment() {
    check(
        "round4_zero_width_explicit_alignment",
        r#"
struct __attribute__((packed)) S { char a; int :0 __attribute__((aligned(16))); char b; };
struct __attribute__((packed)) T { char a; int bits:1; int :0 __attribute__((aligned(16))); char b; };
#ifdef _WIN32
#ifdef __MINGW32__
_Static_assert(__builtin_offsetof(struct S,b)==16 && _Alignof(struct S)==16, "explicit zero width");
_Static_assert(__builtin_offsetof(struct T,b)==16 && _Alignof(struct T)==16, "active zero width");
#else
_Static_assert(__builtin_offsetof(struct S,b)==1 && _Alignof(struct S)==1, "inactive zero width");
_Static_assert(__builtin_offsetof(struct T,b)==16 && _Alignof(struct T)==16, "active zero width");
#endif
#else
_Static_assert(__builtin_offsetof(struct S,b)==16 && _Alignof(struct S)==1, "zero width boundary");
_Static_assert(__builtin_offsetof(struct T,b)==16 && _Alignof(struct T)==1, "active zero width");
#endif
int main(void) { struct S s={7,42}; struct T t={11,0,13}; return s.a!=7 || s.b!=42 || t.a!=11 || t.b!=13; }
"#,
    );
}

#[test]
fn packed_bit_field_explicit_alignment() {
    check(
        "packed_bit_field_explicit_alignment",
        r#"
struct __attribute__((packed)) S { unsigned a:1; unsigned b:1 __attribute__((aligned(4))); };
struct __attribute__((packed)) T { unsigned a:1; unsigned b:1 __attribute__((aligned(1))); char tail; };
#ifdef _WIN32
_Static_assert(sizeof(struct S)==4, "size");
_Static_assert(sizeof(struct T)==5 && __builtin_offsetof(struct T,tail)==4, "offset");
#ifdef __MINGW32__
_Static_assert(_Alignof(struct S)==4, "alignment");
#else
_Static_assert(_Alignof(struct S)==1, "alignment");
#endif
#else
_Static_assert(sizeof(struct S)==8 && _Alignof(struct S)==4, "size/alignment");
_Static_assert(sizeof(struct T)==3 && __builtin_offsetof(struct T,tail)==2, "offset");
#endif
int main(void) { struct S s={1,1}; struct T t={1,1,42};
 unsigned char *p=(unsigned char *)&s;
#ifdef _WIN32
 if(p[0]!=3) return 1;
#else
 if(p[0]!=1 || p[4]!=1) return 1;
#endif
 s.b=0; return s.a!=1 || s.b!=0 || t.tail!=42 || t.a!=1 || t.b!=1;
}"#,
    );
}

#[test]
fn packed_union_bit_fields_use_occupied_bits() {
    check(
        "packed_union_bit_fields_use_occupied_bits",
        r#"
union __attribute__((packed)) U { unsigned a:1; unsigned b:8; };
union __attribute__((packed)) Z { unsigned :0; char c; };
union __attribute__((packed)) N { unsigned a:9; unsigned b:17; };
struct R { union U u; char tail; };
_Static_assert(_Alignof(union U)==1 && sizeof(union Z)==1, "alignment/zero");
#ifdef _WIN32
_Static_assert(sizeof(union U)==4 && sizeof(union N)==4, "size");
_Static_assert(__builtin_offsetof(struct R,tail)==4, "offset");
#else
_Static_assert(sizeof(union U)==1 && sizeof(union N)==3, "size");
_Static_assert(__builtin_offsetof(struct R,tail)==1, "offset");
#endif
static union U global={.b=255};
int main(void) { union U u={.b=123}; union N n={.b=12345}; struct R r={{.b=42},7};
 if(global.b!=255 || u.b!=123 || n.b!=12345 || r.u.b!=42 || r.tail!=7) return 1;
 u.a=0; return u.a!=0 || u.b!=122;
}"#,
    );
}

#[test]
fn packed_bit_fields_span_beyond_the_declared_slot() {
    check(
        "packed_bit_fields_span_beyond_the_declared_slot",
        r#"
struct __attribute__((packed)) P { unsigned a:1; unsigned b:32; char tail; };
struct __attribute__((packed)) C { unsigned char a:7; signed char b:8; char tail; };
struct __attribute__((packed)) L { unsigned long long a:7; unsigned long long b:64; char tail; };
struct __attribute__((packed)) W { unsigned __int128 a:7; unsigned __int128 b:128; char tail; };
struct __attribute__((packed)) H { unsigned __int128 a:7; signed __int128 b:128; unsigned __int128 c:1; };
_Static_assert(_Alignof(struct P)==1 && _Alignof(struct W)==1, "alignment");
#ifdef _WIN32
_Static_assert(sizeof(struct P)==9 && __builtin_offsetof(struct P,tail)==8, "32-bit layout");
_Static_assert(sizeof(struct C)==3 && __builtin_offsetof(struct C,tail)==2, "8-bit layout");
_Static_assert(sizeof(struct L)==17 && __builtin_offsetof(struct L,tail)==16, "64-bit layout");
_Static_assert(sizeof(struct W)==33 && __builtin_offsetof(struct W,tail)==32, "128-bit layout");
#else
_Static_assert(sizeof(struct P)==6 && __builtin_offsetof(struct P,tail)==5, "32-bit layout");
_Static_assert(sizeof(struct C)==3 && __builtin_offsetof(struct C,tail)==2, "8-bit layout");
_Static_assert(sizeof(struct L)==10 && __builtin_offsetof(struct L,tail)==9, "64-bit layout");
_Static_assert(sizeof(struct W)==18 && __builtin_offsetof(struct W,tail)==17, "128-bit layout");
#endif
static struct P global={1,123,7};
static struct W wide={85,((unsigned __int128)1<<127)|123,9};
static struct H signed_global={85,-5,1};
int main(void) {
 struct P p={1,123,7}; struct C c={85,-5,11}; struct L l={85,0xFEDCBA9876543210ULL,13};
 volatile struct W w={85,((unsigned __int128)1<<127)|123,9};
 volatile struct H h={85,-5,1};
 if(global.b!=123 || global.a!=1 || global.tail!=7 || p.b!=123) return 1;
 if(c.a!=85 || c.b!=-5 || c.tail!=11 || l.a!=85 || l.b!=0xFEDCBA9876543210ULL || l.tail!=13) return 2;
 if(w.a!=85 || w.b!=wide.b || w.tail!=9 || wide.a!=85 || wide.tail!=9) return 3;
 if(h.a!=85 || h.b!=-5 || h.c!=1 || signed_global.b!=-5 || signed_global.c!=1) return 4;
 if((p.b=0xFFFFFFFFU)!=0xFFFFFFFFU || (c.b=-128)!=-128) return 5;
 p.b+=2; c.b+=1; l.b=~0ULL;
 if((w.b=~(unsigned __int128)0)!=~(unsigned __int128)0 || (h.b=-1)!=-1) return 6;
 h.b=((__int128)1<<126); w.b=0;
 return p.a!=1 || p.b!=1 || p.tail!=7 || c.a!=85 || c.b!=-127 || c.tail!=11 || l.a!=85 || l.b!=~0ULL || l.tail!=13 || w.a!=85 || w.b!=0 || w.tail!=9 || h.a!=85 || h.b!=((__int128)1<<126) || h.c!=1;
}"#,
    );
}

#[test]
fn multiple_alignas_specifiers_use_the_strictest_requirement() {
    check(
        "multiple_alignas_specifiers_use_the_strictest_requirement",
        r#"
_Alignas(8) _Alignas(16) int a=7;
_Alignas(16) _Alignas(8) int b=11;
_Alignas(1) _Alignas(16) int c=13;
_Alignas(0) _Alignas(16) int d=17;
struct S { char c; _Alignas(8) _Alignas(16) int a; char tail; };
_Static_assert(sizeof(struct S)==32 && _Alignof(struct S)==16 && __builtin_offsetof(struct S,a)==16 && __builtin_offsetof(struct S,tail)==20, "layout");
_Static_assert(__alignof__(a)==16 && __alignof__(b)==16 && __alignof__(c)==16 && __alignof__(d)==16, "objects");
int main(void) { struct S s={1,42,3}; _Alignas(16) _Alignas(8) int local=19;
 return a!=7 || b!=11 || c!=13 || d!=17 || s.a!=42 || s.tail!=3 || __alignof__(local)!=16 || local!=19;
}"#,
    );
}

#[test]
fn alignment_attribute_operands_use_semantic_constants() {
    check(
        "alignment_attribute_operands_use_semantic_constants",
        r#"
enum { A=16 };
typedef int I __attribute__((aligned(sizeof(int)*2)));
struct __attribute__((aligned(_Alignof(double)*2))) R { char c; int n; };
struct M { char c; int n __attribute__((aligned(sizeof(int)))); char tail; };
struct __attribute__((packed)) P { char c; int n __attribute__((aligned(__alignof__(int)))); };
struct N { char c; int n __attribute__((aligned(A/2 + sizeof(int)*2))); };
int a __attribute__((aligned((sizeof(int) + _Alignof(int))*2)))=7;
int __attribute__((aligned(sizeof(int)*4))) *pointer;
_Alignas(sizeof(int)*2) _Alignas(_Alignof(double)*2) int standard;
_Static_assert(sizeof(I)==4 && _Alignof(I)==8, "typedef");
_Static_assert(sizeof(struct R)==16 && _Alignof(struct R)==16, "record");
_Static_assert(sizeof(struct M)==12 && _Alignof(struct M)==4 && __builtin_offsetof(struct M,n)==4 && __builtin_offsetof(struct M,tail)==8, "member");
_Static_assert(sizeof(struct P)==8 && _Alignof(struct P)==4 && __builtin_offsetof(struct P,n)==4, "packed member");
_Static_assert(sizeof(struct N)==32 && _Alignof(struct N)==16 && __builtin_offsetof(struct N,n)==16, "enum/arithmetic");
_Static_assert(__alignof__(a)==16 && __alignof__(pointer)==16 && __alignof__(*pointer)==4 && __alignof__(standard)==16, "objects");
int main(void) { struct M m={1,42,3}; struct P p={1,17}; I i=19;
 return a!=7 || m.n!=42 || m.tail!=3 || p.n!=17 || i!=19;
}"#,
    );
}

#[test]
fn alignas_runtime_array_types_keep_their_natural_alignment() {
    check(
        "alignas_runtime_array_types_keep_their_natural_alignment",
        r#"
struct S { char c; int n; };
_Static_assert(sizeof(struct S)==8 && _Alignof(struct S)==4 && __builtin_offsetof(struct S,n)==4, "element layout");
int main(void) {
 int n=3; typedef int Row[n]; typedef double Doubles[n];
 _Alignas(4) Row a; _Alignas(8) Doubles b; _Alignas(Row) int c=7;
 _Alignas(1) _Alignas(4) int d[n];
 a[2]=42; b[2]=11.0; d[2]=13;
 return sizeof a!=12 || sizeof b!=24 || sizeof d!=12 || __alignof__(a)!=4 || __alignof__(b)!=8 || __alignof__(c)!=4 || __alignof__(d)!=4 || a[2]!=42 || b[2]!=11.0 || c!=7 || d[2]!=13;
}"#,
    );
}

#[track_caller]
fn check(name: &'static str, source: &'static str) {
    let directory = crate::test_support::TempDir::new(name);
    let input = directory.join("reference.c");
    std::fs::write(&input, source).unwrap();
    // Syntax-only cross-target checks need no foreign linker/sysroot.
    let reference = Toolchain::bundled(Target::LinuxGnu).unwrap();
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let output = crate::test_support::native::run_native(
            std::process::Command::new(reference.clang())
                .args(["-std=gnu11", "-fsyntax-only"])
                .arg(format!("--target={}", target.triple()))
                .arg(&input),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{name}: {target:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            numeric_programs::run_main(name, source, target),
            0,
            "{name}: {target:?}"
        );
    }
    if let Some(target) = numeric_programs::host() {
        let toolchain = Toolchain::bundled(target).expect("bundled native toolchain");
        let directory = crate::test_support::TempDir::new(name);
        let program = Program {
            name,
            source,
            expected: 0,
        };
        assert_eq!(
            numeric_programs::run_through_llvm(&toolchain, &directory, &program, target),
            [0; 2]
        );
    }
}
