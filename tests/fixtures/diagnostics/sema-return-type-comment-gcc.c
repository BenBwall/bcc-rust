/* GCC reports a missing return at the first token on the closing brace's
   line, which a comment before it is not. */
static int commented(void) {
 /* comment */ }
static int two_comments(void) {
/* a */ /* b */	}
static int code_after(int x) {
 /* c */ x++; }
int use(void) { return commented() + two_comments() + code_after(1); }
