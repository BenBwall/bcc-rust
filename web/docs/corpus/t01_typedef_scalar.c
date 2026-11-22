typedef double celsius;
typedef unsigned char byte;

int main(void) {
    celsius warm = 21.5;
    byte b = 250;
    b = b + 10;
    typedef int count;
    count n = (int)warm + b;
    return n;
}
