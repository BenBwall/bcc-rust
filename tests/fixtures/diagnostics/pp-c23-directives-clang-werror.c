/* -Werror makes Clang's -Wc23-extensions warnings errors, each printed
   beside the error in the same directive rather than in its place. */
#elifdef YES
const unsigned char bytes[] = {1,
#embed
};
int after;
