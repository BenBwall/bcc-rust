void f(const double m[][3]);
void g(int n, const double m[][n]);
void h(const double (*m)[]);
double a[2][3];
int (*pa)[3]; const int (*cpa)[3];
int (**ppa)[3]; const int (**pcpa)[3]; const int (**pcpinc)[];
void added(void) { f(a); g(3, a); h(a); cpa = pa; }
int compared(void) { return cpa == pa && cpa < pa; }
void dropped(void) { pa = cpa; }
void nested(void) { pcpa = ppa; }
void nested_bound(void) { pcpinc = ppa; }
int following_valid;
