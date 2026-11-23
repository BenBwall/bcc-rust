int main(void) {
    int mode = 2;
    int result = 0;
    switch (mode) {
    case 1: {
        int scale = 10;
        result = scale;
    case 2:
        scale = 20;
        result += scale;
    }
    }
    return result;
}
