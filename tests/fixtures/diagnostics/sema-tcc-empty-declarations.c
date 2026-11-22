/* TCC warns of an unnamed structure that declares nothing, a typedef among
   them, and accepts a tagged one, an enumeration and attributes alone. */
typedef struct { int a; };
struct { int b; };
static union { int c; };
typedef struct T { int d; };
typedef enum { A };
enum { B };
__attribute__((unused));
typedef int section_type __attribute__((section(".foo")));
section_type in_data = 3;
struct S { int member __attribute__((section(".m"))); };
int f(int p) { int local __attribute__((section(".l"))) = p; return local; }
