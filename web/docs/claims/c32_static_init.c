/* Static initializers must be constant expressions (C99 6.7.8p4). */
static const int a = 1;
int b = a;
int main(void) { return b; }
