// Floating point: values are close to what you wrote, not equal to it.
#include <stdio.h>

int main(void) {
    int total = 7;
    int count = 2;
    double average = total / count;      // int division first: 3, then 3.0
    printf("total / count is %f\n", average);
    average = (double)total / count;     // a double division: 3.5
    printf("(double)total / count is %f\n", average);

    double sum = 0.1 + 0.2;              // neither 0.1 nor 0.2 is exact
    printf("0.1 + 0.2 == 0.3 is %d\n", sum == 0.3);
    printf("0.1 + 0.2 is %.17g\n", sum);

    double tenth = 0.0;
    for (int i = 0; i < 10; i++) {
        tenth += 0.1;                    // each addition rounds again
    }
    printf("ten 0.1s make %.17g\n", tenth);

    double price = 2.99;
    int whole = (int)price;              // truncates toward zero: 2
    printf("(int)%.2f is %d\n", price, whole);
    return 0;
}
