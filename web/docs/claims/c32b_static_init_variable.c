/* A static initializer that reads a variable that is not const. */
static int a = 1;
int b = a;
int main(void) { return b; }
