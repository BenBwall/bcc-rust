typedef __WCHAR_TYPE__ wchar_t;
static char exact[3] = (char[3]){"ab"};
char completed[] = (char[3]){"ab"};
char larger[4] = (char[3]){"ab"};
char smaller[2] = (char[3]){"ab"};
char unsized_literal[3] = (char[]){"ab"};
char too_long[3] = (char[3]){"abcd"};
char plain_too_long[3] = "abcd";
wchar_t wide[3] = (wchar_t[3]){L"ab"};
char mixed[3] = (unsigned char[3]){"ab"};
int integers[2] = (int[2]){1, 2};
char braced[3] = {(char[3]){"ab"}};
char qualified_source[3] = (const char[3]){"ab"};
struct member { char text[3]; int value; } member = { (char[3]){"ab"}, 1 };
char conditional[3] = 1 ? (char[3]){"ab"} : (char[3]){"cd"};
void f(int i) {
    char automatic[3] = (char[3]){"ab"};
    static char local_static[3] = (char[3]){"ab"};
    char automatic_larger[4] = (char[3]){"ab"};
    int side_effect[2] = (int[2]){i++, 2};
    (void)automatic; (void)local_static; (void)automatic_larger; (void)side_effect;
}
struct flexible { int n; int a[]; } flexible = { 1, (int[2]){7, 8} };
int following_valid;
