[[vendor::tag((1),[2],{3})]] alignas(16) _Atomic(int) object;
thread_local int thread; constexpr int c=1;
static_assert(1); enum E : unsigned { A [[deprecated]] };
unsigned _BitInt(16) bits; _Decimal32 d32; _Decimal64 d64; _Decimal128 d128;
typeof(c) copy; typeof_unqual(const int) value; auto inferred=1;
int f(int) { bool b=true; b=false; void *p=nullptr; int a[3];
  int n=alignof(int)+_Countof a+_Countof(int[4]);
  n+=_Generic(int,int:1,default:0); if(int x=1;x) n=x;
  switch(n) {case 1 ... 3:break;} outer: for(;;){continue outer; break outer;}
  label: int x=(static int){}; end: }
