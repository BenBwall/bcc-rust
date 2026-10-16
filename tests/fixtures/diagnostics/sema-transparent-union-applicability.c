union U { int *a; long *b; };
union U __attribute__((transparent_union));
int object __attribute__((transparent_union));
union __attribute__((transparent_union)) U *pointer;
struct S { int a; } __attribute__((transparent_union));
int following_valid;
