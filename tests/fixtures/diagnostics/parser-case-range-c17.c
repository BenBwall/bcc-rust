int f(int x) {
  switch (x) {
  case 1 ... 3: return 1;
  case 4: return 2;
  default: return 0;
  }
}
int following;
