// Floating point: values are close to what you wrote, not equal to it.
#include <stdio.h>

int main(void) {
    int total = 7;
    int count = 2;
    // int division: 3, then 3.0
    double average = total / count;
    printf("total / count is %f\n", average);
    // a double division: 3.5
    average = (double)total / count;
    printf("(double)total / count is %f\n", average);

    // neither 0.1 nor 0.2 is exact
    double sum = 0.1 + 0.2;
    printf("0.1 + 0.2 == 0.3 is %d\n", sum == 0.3);
    printf("0.1 + 0.2 is %.17g\n", sum);

    double tenth = 0.0;
    for (int i = 0; i < 10; i++) {
        // rounds at every addition
        tenth += 0.1;
    }
    printf("ten 0.1s make %.17g\n", tenth);

    double price = 2.99;
    // truncates toward zero: 2
    int whole = (int)price;
    printf("(int)%.2f is %d\n", price, whole);
    return 0;
}
