struct F { int n; int v[]; };
static struct F scalar = { .v = 5 };
static struct F element = { .v[1] = 2 };
static struct F listed = { .v = { 1 }, .n = 1 };
int following_valid;
