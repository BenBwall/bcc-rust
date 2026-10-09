/* Shared Clang/bcc target and type-generic primitive probe. */
/* Clang accepts the literal's internal type even on MSVC. */
_Static_assert(sizeof(1.0q) == 16, "literal size");
_Static_assert(_Alignof(__typeof__(1.0q)) == 16, "literal align");
_Static_assert(sizeof(1.0qi) == 32, "complex literal size");
_Static_assert(sizeof(1.0iq) == 32, "imaginary before q");
_Static_assert(__builtin_types_compatible_p(__typeof__(1.0jq), __typeof__(1.0qj)), "j suffix orders");
_Static_assert(__builtin_types_compatible_p(__typeof__(0x1p0iQ), __typeof__(0x1p0Qi)), "hex suffix orders");
_Static_assert((__builtin_classify_type)(1.0q) == 8, "parenthesized builtin");
__typeof__(1.0q) literal_type = 1.0q;
_Static_assert(__builtin_classify_type(1) == 1, "integer");
_Static_assert(__builtin_classify_type((_Bool)0) == 4, "Clang bool classification");
_Static_assert(__builtin_classify_type("text") == 5, "array decay");
_Static_assert(__builtin_classify_type(1.0) == 8, "real");
_Static_assert(__builtin_classify_type(1.0fi) == 9, "complex");
struct classified_record { int member; };
union classified_union { int member; };
enum classified_enum { classified_zero };
int classified_function(void);
_Static_assert(__builtin_classify_type((struct classified_record){0}) == 12, "record");
_Static_assert(__builtin_classify_type((union classified_union){0}) == 13, "union");
_Static_assert(__builtin_classify_type((enum classified_enum)0) == 1, "enum");
_Static_assert(__builtin_classify_type(classified_function) == 5, "function decay");
_Static_assert(__builtin_classify_type((void)0) == 0, "void");
_Static_assert(__builtin_types_compatible_p(const double, double), "qualifiers");
_Static_assert(!__builtin_types_compatible_p(double *, const double *), "nested qualifiers");
_Static_assert(__builtin_types_compatible_p(int[], int[3]), "incomplete array");
_Static_assert(_Generic(1.0f, float: 7, default: 0) == 7, "generic");
_Static_assert(__builtin_choose_expr(1, 7, (void)0) == 7, "choose");
_Static_assert(__imag__ 7 == 0, "imaginary integer");
int selected;
void select_lvalue(void) {
    __builtin_choose_expr(1, selected, (void)0) = 3;
    _Generic(1, int: selected, default: (void)0) = 4;
}

#ifdef __SIZEOF_FLOAT128__
__float128 quad = 1.0000000000000000000000000000000002Q;
__float128 hex = 0x1.0000000000000000000000000001p0q;
_Complex __float128 complex_quad;
__float128 _Complex reversed_complex;
typedef __float128 quad_alias;
_Static_assert(sizeof(__float128) == 16, "binary128 size");
_Static_assert(_Alignof(__float128) == 16, "binary128 alignment");
_Static_assert(sizeof(_Complex __float128) == 32, "complex size");
_Static_assert(_Alignof(_Complex __float128) == 16, "complex alignment");
_Static_assert(__builtin_classify_type(quad) == 8, "binary128 classification");
_Static_assert(__builtin_classify_type(complex_quad) == 9, "complex classification");
_Static_assert(!__builtin_types_compatible_p(__float128, long double), "distinct");
_Static_assert(_Generic(quad + 1.0L, __float128: 1, default: 0), "rank");
_Static_assert(_Generic(quad + 1.0, __float128: 1, default: 0), "double rank");
_Static_assert(_Generic(quad + (__int128)1, __float128: 1, default: 0), "integer rank");
_Static_assert(_Generic(1.0q, __float128: 1, default: 0), "suffix type");
_Static_assert(_Generic(__real__ complex_quad, __float128: 1, default: 0), "real component");
_Static_assert(_Generic(__imag__ complex_quad, __float128: 1, default: 0), "imag component");
_Static_assert(_Generic(quad + 1.0fi, _Complex __float128: 1, default: 0), "complex rank");
void components(void) { __real__ complex_quad = quad; __imag__ complex_quad = quad; }
#endif
