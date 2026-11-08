int gv;
enum { GE = 1 };
typedef int GT;
int gf(void);
static int sv;
void f(int p) {
  { enum { p = 3 }; }
  typedef int LT;
  { typedef int LT; }
  { typedef int GT; }
  { enum { sv = 1 }; }
  { enum { GT = 6 }; }
  { int GE = 7; (void)GE; }
  { typedef int gv; }
  int lv = 0;
  { enum { lv = 4 }; }
  { enum { GE = 5 }; }
  { enum { gf = 7 }; }
  (void)lv;
}
