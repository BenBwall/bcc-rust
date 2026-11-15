/* GCC reports a block's unused variables and labels as it leaves the block,
   latest first, after every warning inside it. */
int blocks(int a, unsigned u) {
    int outer;
    int first, second;
    {
        int inner;
        if (a < u)
            return 1;
    }
    if (a > u)
        return 2;
    for (int i = 0; i < 1; i++) {
        int in_loop;
    }
    int set;
    set = 3;
    return 0;
}

/* An ordinary label belongs to the function's outermost block wherever it
   is defined; a local label to its own block. */
int labels(int a, unsigned u) {
    __label__ declared;
    int x;
first:
    {
        __label__ local;
        int y;
    local:
        ;
    nested:
        ;
    }
declared:
    return a < u;
}

/* A statement expression is a block of its own. */
int statement_expression(int a, unsigned u) {
    int x = ({ int y; a < u; });
    return x;
}

/* The end of the function reports its parameters only set, its typedefs and
   its unused parameters, in that order. */
static int parameters(int only_set, int unused, int read) {
    typedef int Outer;
    {
        typedef int Inner;
        int z;
    }
    only_set = 1;
    if (read)
        return 1;
    return 0;
}

int use(void) { return parameters(1, 2, 3); }
