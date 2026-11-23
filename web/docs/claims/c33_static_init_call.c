int g(void);
int x = g();
int g(void) { return 1; }
int main(void) { return x; }
