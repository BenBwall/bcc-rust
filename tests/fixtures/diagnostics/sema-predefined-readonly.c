void f(void) {
  __PRETTY_FUNCTION__[0] = 1;
  __FUNCTION__ = 0;
}
int after;
