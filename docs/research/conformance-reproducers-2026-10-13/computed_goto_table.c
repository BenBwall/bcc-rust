/* Labels as values in a static table (Lua's lvm.c dispatch, LUA_USE_JUMPTABLE): "not yet supported by
   lowering: this static initializer". */
int main(void) {
    static const void *const table[] = { &&a, &&b };
    int i = 0;
    goto *table[1];
a:  i = 1;
b:  return i - 0;
}
