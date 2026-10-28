struct bf { int x : 3; };
struct bfu { unsigned x : 31; };
struct bfz { int x : 3; int : 0; };
struct zbf { int : 0; int x : 3; };
struct bf2 { int x : 3; int y : 2; };
struct bfc { char c : 3; };
struct bfs { short s : 9; };
struct bfll { long long x : 40; };
struct bfb { _Bool b : 1; };
struct bfcs { int x : 3; char c; };
struct bfpad { char c : 3; } __attribute__((aligned(4)));
struct bfun { int x : 3; int : 5; };
union ubf { int x : 3; };
union u2 { int a; int b; };
union u1 { int a; };
struct a1 { int a[1]; };
struct a2 { int a[2]; };
struct ac1 { char a[1]; };
struct as1 { struct { int x; } s[1]; };
struct af1 { float f[1]; };
struct a0 { int z[0]; int x; };
struct e { struct {} e; int x; };
struct n1 { struct bf inner; };
struct ld { long double l; };
struct p { int *p; };
struct cplx { _Complex float c; };
struct bfi64 { long long x : 3; };
struct h1 { __fp16 h; };
struct h2 { __fp16 a, b; };
union uh { __fp16 h; };
#define F(T) struct T f_##T(struct T a) { return a; }
F(bf) F(bfu) F(bfz) F(zbf) F(bf2) F(bfc) F(bfs) F(bfll) F(bfb) F(bfcs) F(bfpad) F(bfun)
F(a1) F(a2) F(ac1) F(as1) F(af1) F(a0) F(e) F(n1) F(ld) F(p) F(cplx) F(bfi64) F(h1) F(h2)
union ubf f_ubf(union ubf a) { return a; }
union u2 f_u2(union u2 a) { return a; }
union u1 f_u1(union u1 a) { return a; }
union uh f_uh(union uh a) { return a; }
