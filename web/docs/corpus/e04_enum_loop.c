enum day { MON = 1, TUE, WED, THU, FRI };

int main(void) {
    int hours = 0;
    for (enum day d = MON; d <= FRI; d++) {
        hours += 8;
    }
    return hours;
}
