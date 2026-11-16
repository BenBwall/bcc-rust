// Zero-initialized static objects take no room in the file (see `size`).
int zeroed[100000];
int counted = 7;
int main(void) { return zeroed[5] + counted; }
