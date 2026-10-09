typedef int v4i __attribute__((vector_size(16)));
typedef float v4f __attribute__((vector_size(16)));
typedef double v4d __attribute__((vector_size(32)));
_Static_assert(sizeof(v4i) == 16, "integer vector size");
_Static_assert(_Alignof(v4i) == 16, "integer vector alignment");
_Static_assert(sizeof(v4d) == 32, "wide vector size");
_Static_assert(_Alignof(v4d) == 32, "wide vector alignment");
struct holder { v4i value; v4f array[2]; };
_Static_assert(sizeof(struct holder) == 48, "member layout");

v4i operations(v4i a, v4i b, int x) {
    v4i c = {1, 2, 3, 4};
    c += a * b;
    c = (a + x) ^ (b >> 1);
    c[2] = x;
    return ~c + (a < b);
}
v4f conversions(v4i a, v4f b) {
    v4f c = (v4f)a;
    return __builtin_convertvector(a, v4f) + __builtin_shufflevector(c,b,0,5,2,7);
}

typedef int v3i __attribute__((vector_size(12)));
_Static_assert(sizeof(v3i) == 16 && _Alignof(v3i) == 16, "three padded lanes");
typedef int va __attribute__((vector_size((8+8)*2), __aligned__(1)));
_Static_assert(sizeof(va) == 32 && _Alignof(va) == 1, "explicit alignment");
v4i lax(v4i a, v4f b) { a = b; return a + b; }
typedef int octal_vector __attribute__((vector_size(020)));
_Static_assert(sizeof(octal_vector)==16,"octal vector size");
typedef int separated_a __attribute__((aligned(1))) __attribute__((vector_size(16)));
typedef int separated_b __attribute__((vector_size(16))) __attribute__((aligned(1)));
_Static_assert(sizeof(separated_a)==16 && _Alignof(separated_a)==1,"separate alignment first");
_Static_assert(sizeof(separated_b)==16 && _Alignof(separated_b)==1,"separate vector first");
v4i static_integer = {1,2,3,4};
v4f static_float = {1.0f,2.0f,3.0f,4.0f};
