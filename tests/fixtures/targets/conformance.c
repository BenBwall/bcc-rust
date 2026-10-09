/* Target contract, checked by bcc and the pinned Clang in C11 mode.
 * C99: representations 6.2.5, integer constants 6.4.4.1, literals 6.4.4.4
 * and 6.4.5, record layout 6.7.2.1, typedefs 7.17 and 7.18. */
#include <stddef.h>
#include <stdint.h>
#include <stdarg.h>
#include <float.h>
#define CHECK(e) _Static_assert(e, #e)
#define LAYOUT(t,s,a) CHECK(sizeof(t)==s); CHECK(_Alignof(t)==a)
#define TYPE(e,t) CHECK(_Generic((e),t:1,default:0))
LAYOUT(_Bool,1,1);
LAYOUT(char,1,1);
LAYOUT(signed char,1,1);
LAYOUT(unsigned char,1,1);
LAYOUT(short,2,2);
LAYOUT(unsigned short,2,2);
LAYOUT(int,4,4);
LAYOUT(unsigned int,4,4);
LAYOUT(long long,8,8);
LAYOUT(unsigned long long,8,8);
LAYOUT(float,4,4);
LAYOUT(double,8,8);
LAYOUT(float _Complex,8,4);
LAYOUT(double _Complex,16,8);
LAYOUT(void*,8,8);
CHECK((char)-1 < 0);
CHECK(__CHAR_BIT__ == 8);
CHECK(__BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__);
TYPE(2147483647,int);
TYPE(4294967295U,unsigned int);
TYPE(0x80000000,unsigned int);
TYPE(1L,long);
TYPE(1LL,long long);
TYPE((int8_t)0,signed char);
TYPE((uint8_t)0,unsigned char);
TYPE((int16_t)0,short);
TYPE((uint16_t)0,unsigned short);
TYPE((int32_t)0,int);
TYPE((uint32_t)0,unsigned int);
TYPE((int_fast8_t)0,signed char);
TYPE((int_fast16_t)0,short);
TYPE((int_fast32_t)0,int);
TYPE((int_least8_t)0,signed char);
TYPE((int_least16_t)0,short);
TYPE((int_least32_t)0,int);
TYPE((__SIG_ATOMIC_TYPE__)0,int);
CHECK(__SIG_ATOMIC_MAX__ == 2147483647);
CHECK(sizeof(L"A") == 2*sizeof(wchar_t));
TYPE(0.0L,long double);
enum WidePositive { positive = 4294967296 };
enum WideNegative { negative = -4294967296 };
#ifdef _M_X64
LAYOUT(enum WidePositive,4,4);
LAYOUT(enum WideNegative,4,4);
TYPE(positive,int);
CHECK(positive==0 && negative==0);
#else
LAYOUT(enum WidePositive,8,8);
LAYOUT(enum WideNegative,8,8);
CHECK(positive==4294967296 && negative==-4294967296);
#endif
struct A { unsigned char a:3; unsigned int b:5; unsigned char c:2; char tail; };
struct B { char lead; unsigned int :0; char tail; };
struct C { unsigned char a:3; unsigned int :0; char tail; };
struct D { unsigned int a:1; unsigned char :0; unsigned int b:1; char tail; };
struct E { unsigned char a:4; unsigned char b:4; unsigned char c:1; char tail; };
struct F { unsigned int :3; char tail; };
struct G { unsigned int :0; char tail; };
struct H { unsigned int a:1; unsigned int :0; unsigned int :0; char tail; };
struct J { unsigned int a:31; unsigned int b:2; char tail; };
struct K { long a; void *b; long double c; };
union U { unsigned int a:3; unsigned char b:2; char tail; };
union V { unsigned int :0; char tail; };
union W { unsigned int :3; char tail; };
LAYOUT(struct E,3,1);
CHECK(offsetof(struct E,tail)==2);
LAYOUT(struct G,1,1);
LAYOUT(struct H,8,4);
CHECK(offsetof(struct H,tail)==4);
LAYOUT(union V,1,1);
CHECK(offsetof(union U,tail)==0);
#ifdef _WIN32
CHECK(((long)-1 < (unsigned int)0)==0);
CHECK((long)0xffffffffUL == -1L);
#if !(L'\xff' < -1)
#error wide character must use unsigned intmax in Windows preprocessing
#endif
LAYOUT(long,4,4);
LAYOUT(unsigned long,4,4);
LAYOUT(wchar_t,2,2);
LAYOUT(__WINT_TYPE__,2,2);
LAYOUT(va_list,8,8);
TYPE((va_list)0,char*);
TYPE((wchar_t)0,unsigned short);
TYPE(L'a',unsigned short);
TYPE((__WINT_TYPE__)0,unsigned short);
TYPE((size_t)0,unsigned long long);
TYPE((ptrdiff_t)0,long long);
TYPE((intptr_t)0,long long);
TYPE((uintptr_t)0,unsigned long long);
TYPE((intmax_t)0,long long);
TYPE((uintmax_t)0,unsigned long long);
TYPE((int64_t)0,long long);
TYPE((uint64_t)0,unsigned long long);
TYPE((int_fast64_t)0,long long);
TYPE((int_least64_t)0,long long);
TYPE(2147483648,long long);
TYPE(2147483648L,long long);
TYPE(0x100000000,long long);
CHECK(sizeof(L"\U0001f600")==6);
CHECK(__WCHAR_UNSIGNED__==1);
CHECK(__WCHAR_MAX__==65535);
CHECK(_WIN32==1 && _WIN64==1);
LAYOUT(struct A,12,4);
CHECK(offsetof(struct A,tail)==9);
LAYOUT(struct B,2,1);
CHECK(offsetof(struct B,tail)==1);
LAYOUT(struct C,8,4);
CHECK(offsetof(struct C,tail)==4);
LAYOUT(struct D,12,4);
CHECK(offsetof(struct D,tail)==8);
LAYOUT(struct F,8,4);
CHECK(offsetof(struct F,tail)==4);
LAYOUT(struct J,12,4);
CHECK(offsetof(struct J,tail)==8);
LAYOUT(union U,4,1);
LAYOUT(union W,4,1);
#ifdef __MINGW32__
LAYOUT(long double,16,16);
LAYOUT(long double _Complex,32,16);
CHECK(LDBL_MANT_DIG==64);
CHECK(__MINGW32__==1 && __MINGW64__==1 && __SEH__==1);
LAYOUT(struct K,32,16);
#else
LAYOUT(long double,8,8);
LAYOUT(long double _Complex,16,8);
CHECK(LDBL_MANT_DIG==53);
CHECK(_M_X64==100 && _M_AMD64==100);
LAYOUT(struct K,24,8);
#endif
#else
CHECK(((long)-1 < (unsigned int)0)==1);
CHECK(L'\xffffffff' == -1);
enum WideEscape { escape = L'\xffffffff' };
CHECK(escape == -1);
#if L'\xffffffff' != -1
#error wide character must sign extend to intmax in Linux preprocessing
#endif
#if L'\xff' < -1
#error wide character must use signed intmax in Linux preprocessing
#endif
LAYOUT(long,8,8);
LAYOUT(unsigned long,8,8);
LAYOUT(long double,16,16);
LAYOUT(long double _Complex,32,16);
LAYOUT(wchar_t,4,4);
LAYOUT(__WINT_TYPE__,4,4);
LAYOUT(va_list,24,8);
TYPE((wchar_t)0,int);
TYPE(L'a',int);
TYPE((__WINT_TYPE__)0,unsigned int);
TYPE((size_t)0,unsigned long);
TYPE((ptrdiff_t)0,long);
TYPE((intptr_t)0,long);
TYPE((uintptr_t)0,unsigned long);
TYPE((intmax_t)0,long);
TYPE((uintmax_t)0,unsigned long);
TYPE((int64_t)0,long);
TYPE((uint64_t)0,unsigned long);
TYPE((int_fast64_t)0,long);
TYPE((int_least64_t)0,long);
TYPE(2147483648,long);
TYPE(2147483648L,long);
TYPE(0x100000000,long);
CHECK(sizeof(L"\U0001f600")==8);
CHECK(__linux__==1 && __unix__==1 && __ELF__==1);
CHECK(__LP64__==1 && _LP64==1);
CHECK(LDBL_MANT_DIG==64);
LAYOUT(struct A,4,4);
CHECK(offsetof(struct A,tail)==2);
LAYOUT(struct B,5,1);
CHECK(offsetof(struct B,tail)==4);
LAYOUT(struct C,5,1);
LAYOUT(struct D,4,4);
CHECK(offsetof(struct D,tail)==2);
LAYOUT(struct F,2,1);
LAYOUT(struct J,8,4);
CHECK(offsetof(struct J,tail)==5);
LAYOUT(union U,4,4);
LAYOUT(union W,1,1);
LAYOUT(struct K,32,16);
#endif
CHECK(offsetof(struct K,b)==8);
CHECK(offsetof(struct K,c)==16);
/* All four #if models promote signed/unsigned integer types to 64 bits. */
#if 0xffffffff != 4294967295 || 0x7fffffffffffffffL != 9223372036854775807LL
#error wrong preprocessing intmax model
#endif
void arguments(int n, ...) {
    va_list a,b;
    va_start(a,n);
    va_copy(b,a);
    int value=va_arg(b,int);
    va_end(a);
    va_end(b);
    (void)value;
}
