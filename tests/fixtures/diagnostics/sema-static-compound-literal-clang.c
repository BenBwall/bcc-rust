static double scaled = (double)((double){0x1p20});
static int sum = (int){3} + 1;
static int truncated = (int)(double){2.5};
static int braced[1] = { (int)(int){3} };
enum { E = (int){3} };
static int read_volatile = (volatile int){3} + 1;
static int element = (int[]){1, 2}[1];
void f(void) { static int *p = &(int){3}; (void)p; }
int following_valid;
