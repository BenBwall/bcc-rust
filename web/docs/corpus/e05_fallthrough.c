enum level { LOW, MEDIUM, HIGH };

int main(void) {
    enum level l = MEDIUM;
    int alarms = 0;
    switch (l) {
    case HIGH:
        alarms++;
    case MEDIUM:
        alarms++;
    case LOW:
        alarms++;
    }
    return alarms;
}
