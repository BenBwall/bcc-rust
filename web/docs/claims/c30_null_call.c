/* A call through a null pointer to a function. */
static int (*handler)(int);
int main(void) {
    return handler(1);
}
