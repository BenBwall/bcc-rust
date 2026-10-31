struct bits { int b : 3; } s;
int x;
extern int e;
auto e = 1;
void f(__auto_type a);
struct member { __auto_type m; };
void g(void) {
    __auto_type missing;
    auto braced = {1};
    auto first = 1, second = 2;
    auto *pointer = &x;
    __auto_type array[2] = {1, 2};
    __auto_type outer = x;
    auto inner = inner + 1;
    __auto_type field = s.b;
    auto tagged = (struct local { int m; }){1};
    static auto int typed = 1;
}
