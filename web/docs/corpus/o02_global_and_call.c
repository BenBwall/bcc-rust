int g = 0;
int bump(void) { g = g + 1; return g; }
int main(void) {
    return g + bump();
}
