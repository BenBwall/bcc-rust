/* TCC 0.9.27 warns of a label that ends a compound statement, at its
   closing brace. */
int main(int c) {
  switch (c) {
  case 1:
  }
  switch (c) { default: }
  { first: ; second:
  }
  if (c) goto done;
  c = 1;
done:

}
