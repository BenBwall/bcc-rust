struct S { int a, b; };
struct T { struct S s; int c; };
struct S one = {1};
struct S zero = {0};
struct S empty = {};
struct S designated = {.a = 1};
struct S all = {1, 2};
struct T nested = {{1}, 2};
struct T inside = {.s = {1}, .c = 1};
