/* `-fdiagnostics-parseable-fixits` writes each hint again after the
   snippet, with byte columns; `-fno-diagnostics-fixit-info` hides only the
   line under the source. */
int f(_Bool b, int y, int *p) {
	int r = ~b;
  p = (y);
  p = *p;
  return r + ~b;
}
