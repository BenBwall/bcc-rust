/* GCC/MSVC packing state; Clang checks these facts on every target. */
#pragma pack(1)
struct A { char c; int i; };
#pragma pack()
#pragma pack(push, outer, 1)
struct B { char c; int i; };
#pragma pack(push, inner, 2)
struct C { char c; int i; };
#pragma pack(pop, outer)
struct D { char c; int i; };
#pragma pack(push)
#pragma pack(4)
struct E { char c; long long i; };
#pragma pack(pop)
struct F { char c; int i; } __attribute__((packed));
struct G { char c; int i; };
#define AROUND(x) struct BeforeMacro { char c; int i; }; x struct AfterMacro { char c; int i; };
AROUND(_Pragma("pack(1)"))
#pragma pack()
_Static_assert(sizeof(struct BeforeMacro)==8 && sizeof(struct AfterMacro)==5, "macro argument order");
#define DO_PRAGMA(x) _Pragma(#x)
#define PACKING 1
DO_PRAGMA(pack(push, macro, PACKING))
struct Bits { char c; unsigned f:32; char tail; };
struct Zero { char c; unsigned f:1; unsigned :0; char tail; };
struct Inactive { char c; unsigned :0; char tail; };
struct ExplicitBits { unsigned a:1; unsigned b:1 __attribute__((aligned(16))); char tail; };
union PackedUnion { unsigned f:1; };
struct Explicit { char c; int i __attribute__((aligned(16))); };
typedef int AlignedInt __attribute__((aligned(16)));
struct Inherited { char c; AlignedInt i; };
struct __attribute__((packed, aligned(16))) Raised { char c; int i; };
struct Nested {
#pragma pack(4)
 char c; int i;
};
struct AfterNested { char c; int i; };
DO_PRAGMA(pack(pop, macro))
_Static_assert(sizeof(struct Bits)==6 && __builtin_offsetof(struct Bits,tail)==5, "packed value bits");
#ifdef _WIN32
_Static_assert(sizeof(struct Inactive)==2 && sizeof(struct ExplicitBits)==5 && sizeof(union PackedUnion)==4, "Microsoft units");
/* MinGW GCC and MSVC: the zero-width field ends f's unit at byte 5. Clang's
   MinGW layout ignores the pack cap there and overlaps tail with f's unit;
   bcc, which does not claim to be Clang, follows MinGW GCC (#55). */
#if defined(__MINGW32__) && defined(__clang__)
_Static_assert(sizeof(struct Zero)==8 && __builtin_offsetof(struct Zero,tail)==4, "Clang MinGW zero boundary");
#else
_Static_assert(sizeof(struct Zero)==6 && __builtin_offsetof(struct Zero,tail)==5, "Microsoft zero boundary");
#endif
#ifdef __MINGW32__
_Static_assert(sizeof(struct Explicit)==5 && sizeof(struct Inherited)==5, "GNU aligned cap");
#else
_Static_assert(sizeof(struct Explicit)==32 && sizeof(struct Inherited)==32, "MSVC explicit alignment");
#endif
#else
_Static_assert(sizeof(struct Zero)==5 && sizeof(struct Inactive)==5, "System V zero boundary");
_Static_assert(sizeof(struct ExplicitBits)==2 && sizeof(union PackedUnion)==1, "System V occupied bits");
_Static_assert(sizeof(struct Explicit)==5 && sizeof(struct Inherited)==5, "GNU aligned cap");
#endif
_Static_assert(sizeof(struct Raised)==16 && _Alignof(struct Raised)==16, "record alignment");
_Static_assert(sizeof(struct Nested)==5 && sizeof(struct AfterNested)==8, "definition captures opening state");
_Static_assert(sizeof(struct A)==5 && sizeof(struct B)==5, "pack one");
_Static_assert(sizeof(struct C)==6 && sizeof(struct D)==8, "named stack");
_Static_assert(sizeof(struct E)==12 && sizeof(struct F)==5 && sizeof(struct G)==8, "restore");
int main(void) {
 struct A a={7,42}; struct Bits b={7,0xFEDCBA98U,11};
 struct ExplicitBits e={1,1,13}; struct Zero z={17,1,19}; union PackedUnion u={1};
 b.f+=3; e.b=0; z.f=0;
 return a.c!=7 || a.i!=42 || b.f!=0xFEDCBA9BU || b.tail!=11 || e.a!=1 || e.b!=0 || e.tail!=13 || z.c!=17 || z.f!=0 || z.tail!=19 || u.f!=1;
}
