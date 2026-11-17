/* Clang takes a discarded statement expression's value from its last
   statement, after its labels; its value statement itself warns nothing. */
int h(void);
void f(int a) {
  ({ a + 1; });
  ({ h(); a; });
  ({ goto l; l: a == 1; });
  ({ h(); });
  ({ (void)a; });
  ({ int v = a; v; });
}
